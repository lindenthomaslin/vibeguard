//! 全屏界面：顶部图标栏、中间对话与选择区、固定在底部的输入框和状态栏。
//!
//! 键盘线程、AI 流式任务和定时器都往同一个通道发消息，主循环每批处理完再重绘一次，
//! 流式输出再快也不会逐 token 重绘。

mod form;
mod layout;
mod state;
mod view;

use crate::app::App;
use crate::chat::Entry;
use crate::commands::{display_value, env_override, source_label};
use crate::config::{self, Thinking, mask_key};
use crate::llm::{ChatOptions, ChatResponse, LlmClient, Message, Role, StreamEvent};
use crate::repl::SYSTEM_PROMPT;
use crate::sessions::{Conversation, Store, format_ago};
use crate::ui;
use anyhow::{Result, bail};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use crossterm::execute;
use form::ModelSettings;
use ratatui::DefaultTerminal;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use state::{Action, AppState, COMMANDS, SessionItem, start_demo};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use unicode_width::UnicodeWidthStr;
use view::Props;

const HINT: &str = "Enter 发送 · Ctrl-J 换行 · / 命令 · PgUp/PgDn 滚动 · Ctrl-D 退出";
const BUSY_HINT: &str = "Esc 取消 · PgUp/PgDn 滚动";

/// 后台任务发回的消息都带着发起时的请求编号；取消后编号变化，迟到的消息会被丢弃
enum Msg {
    Term(Event),
    Reasoning(u64, String),
    Content(u64, String),
    Retry(u64, String),
    Done(u64, Result<(ChatResponse, Duration)>),
    ToolSuggestion {
        path: String,
        old_code: String,
        new_code: String,
    },
    Tick,
}

/// 进入全屏界面，直到用户退出。终端状态在返回前（含出错、panic）都会还原。
pub async fn run(app: &mut App) -> Result<()> {
    let mut terminal = ratatui::try_init()?;
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    let result = event_loop(app, &mut terminal).await;
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn fresh_conversation(store: &Store, model: &str) -> Conversation {
    let ms = chrono::Utc::now().timestamp_millis();
    Conversation::new(store.fresh_id(ms), ms / 1000, model)
}

struct Session<'a> {
    app: &'a mut App,
    state: AppState,
    store: Store,
    /// 当前对话；有内容后每轮问答都会保存到磁盘
    conv: Conversation,
    tx: UnboundedSender<Msg>,
    task: Option<JoinHandle<()>>,
    /// 最近一次后台请求的编号
    req: u64,
    /// 正在进行的请求对应的用户输入，成功后才写入对话
    pending_user: Option<String>,
    last_usage: String,
    cwd: String,
}

async fn event_loop(app: &mut App, terminal: &mut DefaultTerminal) -> Result<()> {
    let (tx, mut rx) = unbounded_channel();

    // 键盘线程：带超时轮询，便于退出时停下
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (tx, stop) = (tx.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if !event::poll(Duration::from_millis(100)).unwrap_or(false) {
                    continue;
                }
                match event::read() {
                    Ok(ev) => {
                        if tx.send(Msg::Term(ev)).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }
    // 定时器：驱动等待时的 spinner
    let ticker = {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                interval.tick().await;
                if tx.send(Msg::Tick).is_err() {
                    break;
                }
            }
        })
    };

    let cwd = std::env::current_dir()
        .map(|p| ui::tilde(&p))
        .unwrap_or_default();
    let store = Store::beside_config(&app.config_path);
    let conv = fresh_conversation(&store, &app.effective().model);
    let mut session = Session {
        app,
        state: AppState::new(),
        store,
        conv,
        tx,
        task: None,
        req: 0,
        pending_user: None,
        last_usage: String::new(),
        cwd,
    };
    session.welcome();

    let mut dirty = true;
    let result = loop {
        if dirty {
            if let Err(e) = session.draw(terminal) {
                break Err(e);
            }
            dirty = false;
        }
        let Some(first) = rx.recv().await else {
            break Ok(());
        };
        let mut batch = vec![first];
        while let Ok(m) = rx.try_recv() {
            batch.push(m);
        }
        let mut quit = false;
        for msg in batch {
            quit |= session.handle(msg, &mut dirty);
        }
        if quit {
            break Ok(());
        }
    };

    stop.store(true, Ordering::Relaxed);
    ticker.abort();
    if let Some(task) = session.task.take() {
        task.abort();
    }
    result
}

impl Session<'_> {
    fn welcome(&mut self) {
        if self.app.effective().api_key.is_none() {
            self.state.push(Entry::Warn(
                "尚未配置 API Key：输入 /model 填写 API 地址、Key 和模型，或设置环境变量 DEEPSEEK_API_KEY"
                    .into(),
            ));
        }
        for w in self.app.effective().warnings() {
            self.state.push(Entry::Warn(w));
        }
        let mut tip = "直接输入问题与 AI 对话，输入 / 查看命令".to_string();
        if let Ok(Some(last)) = self.store.latest(None) {
            tip.push_str(&format!(
                "；/resume 可恢复上一次对话「{}」（{}）",
                last.title,
                format_ago(last.updated, now_secs())
            ));
        }
        self.state.push(Entry::Info(tip));
    }

    fn draw(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let cfg = self.app.effective();
        let thinking = cfg.thinking.to_string();
        let choice = self.state.choice();
        let (input, cursor) = {
            let (chars, cursor) = self.state.input();
            (chars.to_vec(), cursor)
        };
        let input_text: String = input.iter().collect();
        let input_tokens = crate::tokens::estimate_tokens(&input_text);
        let status = if let Some(hint) = self.state.hint() {
            hint.to_string()
        } else if self.state.busy.is_some() {
            BUSY_HINT.to_string()
        } else if self.last_usage.is_empty() {
            HINT.to_string()
        } else {
            self.last_usage.clone()
        };
        let input_label = self.state.input_label();
        let input_active = self.state.input_active();
        terminal.draw(|f| {
            view::draw(
                f,
                Props {
                    model: &cfg.model,
                    thinking: &thinking,
                    cwd: &self.cwd,
                    transcript: &self.state.transcript,
                    input: &input,
                    cursor,
                    input_label,
                    input_active,
                    choice: choice.as_ref(),
                    focus_last: self.state.review.is_some(),
                    busy: self.state.busy.as_deref(),
                    tick: self.state.tick,
                    status: &status,
                    input_tokens,
                    scroll: &mut self.state.scroll,
                },
            );
        })?;
        Ok(())
    }

    /// 处理一条消息；返回 true 表示退出
    fn handle(&mut self, msg: Msg, dirty: &mut bool) -> bool {
        match msg {
            Msg::Tick => {
                if self.state.busy.is_some() {
                    self.state.on_tick();
                    *dirty = true;
                }
                return false;
            }
            Msg::Term(Event::Key(key)) => match self.state.on_key(key) {
                Action::None => {}
                Action::Quit => return true,
                Action::Chat(text) => self.start_chat(text),
                Action::Command(parts) => self.command(parts),
                Action::Cancel => self.cancel(),
                Action::SaveModel(settings) => match self.save_settings(&settings) {
                    Ok(()) => self.state.close_form(),
                    // 保存失败：表单保持打开，已经填好的内容不会丢
                    Err(e) => self
                        .state
                        .push(Entry::Error(format!("保存失败，配置文件没有改动：{e:#}"))),
                },
                Action::ResumeSession(id) => {
                    if let Err(e) = self.resume(&id) {
                        self.state.push(Entry::Error(format!("{e:#}")));
                    }
                }
            },
            Msg::Term(Event::Paste(text)) => self.state.on_paste(&text),
            Msg::Term(_) => {} // Resize 等：下面统一重绘
            Msg::Reasoning(req, s) if req == self.req => self.state.on_reasoning(&s),
            Msg::Content(req, s) if req == self.req => self.state.on_content(&s),
            Msg::Retry(req, s) if req == self.req => self.state.push(Entry::Warn(s)),
            Msg::ToolSuggestion { path, old_code, new_code } => {
                self.state.push(Entry::ToolSuggestion {
                    path,
                    old_code,
                    new_code,
                });
            }
            Msg::Done(req, result) if req == self.req => self.finish_chat(result),
            // 已取消请求的迟到消息
            Msg::Reasoning(..) | Msg::Content(..) | Msg::Retry(..) | Msg::Done(..) | Msg::ToolSuggestion { .. } => {
                return false;
            }
        }
        *dirty = true;
        false
    }

    /// 为新的后台任务分配编号
    fn next_req(&mut self) -> u64 {
        self.req += 1;
        self.req
    }

    fn finish_task(&mut self) {
        self.task = None;
        self.state.end_request();
    }

    // ---- AI 对话 ----

    fn start_chat(&mut self, input: String) {
        let client = match LlmClient::new(&self.app.effective()) {
            Ok(c) => c,
            Err(e) => {
                self.state.push(Entry::User(input));
                self.state.push(Entry::Error(format!("{e:#}")));
                return;
            }
        };
        let mut messages = Vec::with_capacity(self.conv.messages.len() + 2);
        messages.push(Message::system(SYSTEM_PROMPT));
        messages.extend(self.conv.messages.iter().cloned());
        messages.push(Message::user(input.as_str()));

        self.state.begin_request(&input);
        self.pending_user = Some(input);
        let req = self.next_req();
        let tx = self.tx.clone();
        let perm = self.app.file.permission_mode.clone();
        let tools = crate::tools::tool_definitions();

        self.task = Some(tokio::spawn(async move {
            let start = Instant::now();
            let events = tx.clone();

            let mut opts = ChatOptions::default();
            opts.tools = tools;

            // 工具调用循环
            let mut loop_count = 0;
            let result = loop {
                loop_count += 1;
                if loop_count > 10 {
                    break Err(anyhow::anyhow!("工具调用超过最大次数限制"));
                }

                let loop_result = client
                    .chat_stream(&messages, &opts, |ev| {
                        let _ = match ev {
                            StreamEvent::Reasoning(t) => {
                                events.send(Msg::Reasoning(req, t.to_string()))
                            }
                            StreamEvent::Content(t) => events.send(Msg::Content(req, t.to_string())),
                            StreamEvent::Retrying {
                                attempt,
                                delay,
                                reason,
                            } => events.send(Msg::Retry(
                                req,
                                format!(
                                    "请求失败，{:.1}s 后第 {attempt} 次重试：{reason}",
                                    delay.as_secs_f64()
                                ),
                            )),
                        };
                    })
                    .await;

                match loop_result {
                    Ok(res) => {
                        if res.wants_tool_calls() {
                            // 添加助理的工具调用请求
                            messages.push(Message::tool_calls(res.tool_calls.clone()));

                            // 执行工具
                            for call in &res.tool_calls {
                                let tool_name = &call.function.name;
                                let tool_args = &call.function.arguments;
                                let tool_result = crate::tools::execute_tool(tool_name, tool_args, &perm);

                                // 对于代码修改操作，显示建议卡片
                                if tool_name == "edit_file" || tool_name == "write_file" {
                                    // 提取路径和代码
                                    if let Ok(args) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(tool_args) {
                                        if let (Some(path), Some(content)) = (
                                            args.get("path").and_then(|v| v.as_str()),
                                            args.get("content").or_else(|| args.get("new_str")).and_then(|v| v.as_str()),
                                        ) {
                                            let old_code = if tool_name == "edit_file" {
                                                args.get("old_str").and_then(|v| v.as_str()).unwrap_or("").to_string()
                                            } else {
                                                String::new()
                                            };

                                            // 显示建议卡片
                                            events.send(Msg::ToolSuggestion {
                                                path: path.to_string(),
                                                old_code,
                                                new_code: content.to_string(),
                                            }).ok();
                                        }
                                    }
                                }

                                messages.push(Message::tool_result(call.id.clone(), tool_result));
                            }
                            // 继续循环获取下一个回复
                        } else {
                            break Ok((res, start.elapsed()));
                        }
                    }
                    Err(e) => break Err(e),
                }
            };
            let _ = tx.send(Msg::Done(req, result));
        }));
    }

    fn finish_chat(&mut self, result: Result<(ChatResponse, Duration)>) {
        self.finish_task();
        let user = self.pending_user.take();
        match result {
            Ok((res, elapsed)) => {
                if res.is_truncated() {
                    self.state
                        .push(Entry::Warn("回答达到 max_tokens 上限被截断".into()));
                }
                self.last_usage = ui::usage_text(&res, elapsed);
                if let Some(user) = user {
                    self.conv.model = self.app.effective().model;
                    self.conv.push_exchange(&user, &res.content, now_secs());
                    if let Err(e) = self.store.save(&self.conv) {
                        self.state
                            .push(Entry::Warn(format!("对话保存失败：{e:#}")));
                    }
                }
            }
            Err(e) => self.state.push(Entry::Error(format!("{e:#}"))),
        }
    }

    fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.next_req(); // 让已经在路上的消息作废
        self.pending_user = None;
        self.state.end_request();
        self.state.push(Entry::Warn("已取消".into()));
    }

    // ---- 模型配置 ----

    /// 打开 `/model` 表单。初始值取自配置文件，而不是环境变量覆盖后的值，
    /// 否则一保存就会把环境变量里的 Key 写进文件。
    fn open_model_form(&mut self) {
        for (key, label) in ENV_FIELDS {
            if let Some(name) = env_override(key) {
                self.state.push(Entry::Warn(format!(
                    "环境变量 {name} 已设置，会覆盖配置文件里的{label}；表单显示的是配置文件里的值"
                )));
            }
        }
        let settings = ModelSettings::from_config(&self.app.file);
        self.state.open_model_form(&settings);
    }

    /// 把设置一次写入配置文件，并撤销本次会话的临时覆盖（命令行的 --model 之类）。
    /// 失败时配置文件和内存里的配置都保持原样。
    fn save_settings(&mut self, settings: &ModelSettings) -> Result<()> {
        let next = settings.applied_to(&self.app.file)?;
        next.save(&self.app.config_path)?;
        self.app.file = next;
        self.app.overrides.model = None;
        self.app.overrides.thinking = None;

        self.state.push(Entry::Success("配置已保存".into()));
        let lines = saved_lines(&ui::tilde(&self.app.config_path), settings);
        self.state.push(Entry::Block(lines));
        for (key, label) in ENV_FIELDS {
            if let Some(name) = env_override(key) {
                self.state.push(Entry::Warn(format!(
                    "环境变量 {name} 已设置，运行时它仍会覆盖刚保存的{label}"
                )));
            }
        }
        for w in self.app.effective().warnings() {
            self.state.push(Entry::Warn(w));
        }
        Ok(())
    }

    // ---- 对话管理 ----

    fn new_conversation(&mut self) {
        let had_messages = !self.conv.messages.is_empty();
        self.conv = fresh_conversation(&self.store, &self.app.effective().model);
        self.state.transcript.clear();
        self.last_usage.clear();
        if had_messages {
            self.state.push(Entry::Success(
                "已开始新对话，之前的对话已保存，可用 /resume 或 /sessions 找回".into(),
            ));
        } else {
            self.state.push(Entry::Success("已开始新对话".into()));
        }
    }

    fn resume(&mut self, id: &str) -> Result<()> {
        if id == self.conv.id {
            self.state.push(Entry::Info("已经在这个对话里了".into()));
            return Ok(());
        }
        let conv = self.store.load(id)?;
        self.load_conversation(conv);
        Ok(())
    }

    fn resume_latest(&mut self) -> Result<()> {
        match self.store.latest(Some(&self.conv.id))? {
            Some(conv) => self.load_conversation(conv),
            None => self
                .state
                .push(Entry::Info("还没有可以恢复的历史对话".into())),
        }
        Ok(())
    }

    /// 用历史对话替换当前对话，并把消息重新画到对话区
    fn load_conversation(&mut self, conv: Conversation) {
        self.state.transcript.clear();
        self.last_usage.clear();
        self.state.push(Entry::Info(format!(
            "已恢复对话「{}」· {} 轮 · {}",
            conv.title,
            conv.turns(),
            format_ago(conv.updated, now_secs())
        )));
        for m in &conv.messages {
            match m.role {
                Role::User => self.state.push(Entry::User(m.content.clone())),
                // 恢复时只回显最终的文字回复；中间的工具调用和结果不再重放
                Role::Assistant if !m.content.is_empty() => self.state.push(Entry::Assistant {
                    reasoning: String::new(),
                    content: m.content.clone(),
                }),
                Role::Assistant | Role::System | Role::Tool => {}
            }
        }
        self.conv = conv;
    }

    fn list_sessions(&mut self) -> Result<()> {
        let listing = self.store.list()?;
        if listing.skipped > 0 {
            self.state.push(Entry::Warn(format!(
                "有 {} 个对话文件无法读取，已跳过（{}）",
                listing.skipped,
                ui::tilde(self.store.dir())
            )));
        }
        let now = now_secs();
        let items: Vec<SessionItem> = listing
            .items
            .into_iter()
            .filter(|s| s.id != self.conv.id)
            .map(|s| SessionItem {
                id: s.id,
                when: format_ago(s.updated, now),
                text: format!("{} · {} 轮 · {}", s.title, s.turns, s.model),
            })
            .collect();
        if items.is_empty() {
            self.state.push(Entry::Info("还没有历史对话".into()));
            return Ok(());
        }
        self.state.push(Entry::Info(format!(
            "共 {} 个历史对话，↑↓ 选择，Enter 恢复",
            items.len()
        )));
        self.state.open_sessions("选择对话", items);
        Ok(())
    }

    fn search_sessions(&mut self, query: &str) -> Result<()> {
        if query.trim().is_empty() {
            bail!("用法：/search 关键词");
        }
        let result = self.store.search(query)?;
        if result.skipped > 0 {
            self.state.push(Entry::Warn(format!(
                "有 {} 个对话文件无法读取，已跳过",
                result.skipped
            )));
        }
        if result.hits.is_empty() {
            self.state
                .push(Entry::Info(format!("没有找到包含「{query}」的对话")));
            return Ok(());
        }
        let now = now_secs();
        self.state.push(Entry::Info(format!(
            "找到 {} 个对话包含「{query}」，↑↓ 选择，Enter 恢复",
            result.hits.len()
        )));
        let items = result
            .hits
            .into_iter()
            .map(|h| {
                let who = if h.role == Role::User { "你" } else { "AI" };
                SessionItem {
                    id: h.summary.id,
                    when: format_ago(h.summary.updated, now),
                    text: format!("{} · {who}：{}", h.summary.title, h.snippet),
                }
            })
            .collect();
        self.state.open_sessions(&format!("搜索「{query}」"), items);
        Ok(())
    }

    // ---- 斜杠命令 ----

    fn command(&mut self, parts: Vec<String>) {
        self.state.push(Entry::User(format!("/{}", parts.join(" "))));
        if let Err(e) = self.run_command(&parts) {
            self.state.push(Entry::Error(format!("{e:#}")));
        }
    }

    fn run_command(&mut self, parts: &[String]) -> Result<()> {
        let args: Vec<&str> = parts.iter().skip(1).map(String::as_str).collect();
        match parts.first().map(String::as_str).unwrap_or("") {
            "help" | "h" | "?" => self.state.push(Entry::Block(help_lines())),
            "demo" => start_demo(&mut self.state),
            "clear" | "new" => self.new_conversation(),
            "resume" => self.resume_latest()?,
            "sessions" | "history" => self.list_sessions()?,
            "search" => self.search_sessions(&args.join(" "))?,
            "config" => self.config(&args)?,
            // API 地址、API Key、模型、思考力度都在同一个表单里；旧命令名保留为别名
            "model" | "apikey" | "baseurl" | "url" => {
                let is_model = parts[0] == "model";
                match args.as_slice() {
                    [] => self.open_model_form(),
                    // /model <模型名>：只换模型，其余沿用配置文件里的值，立即保存
                    [name] if is_model => {
                        let mut settings = ModelSettings::from_config(&self.app.file);
                        settings.model = name.to_string();
                        self.save_settings(&settings)?;
                    }
                    _ => bail!("用法：/model 打开配置表单，或 /model <模型名> 直接换模型"),
                }
            }
            "thinking" => match args.as_slice() {
                [] => self.state.push(Entry::Info(format!(
                    "当前思考模式：{}（/model 可以连同模型一起保存）",
                    self.app.effective().thinking
                ))),
                [mode] => {
                    let mode: Thinking = mode.parse().map_err(anyhow::Error::msg)?;
                    self.app.overrides.thinking = Some(mode);
                    self.state
                        .push(Entry::Success(format!("本次会话思考模式：{mode}")));
                }
                _ => bail!("用法：/thinking [auto|off|low|high|max]"),
            },
            "permissions" => match args.as_slice() {
                [] => self.show_permissions(),
                [mode] => {
                    self.set_permission_mode(mode)?;
                }
                _ => bail!("用法：/permissions [readonly|editable|full] [工作目录]"),
            },
            name @ ("scan" | "fix" | "report") => {
                self.state.push(Entry::Warn(format!("/{name} 尚在开发中")));
            }
            "" => bail!("请输入命令，/help 查看可用命令"),
            other => bail!("未知命令 /{other}，/help 查看可用命令"),
        }
        Ok(())
    }

    fn config(&mut self, args: &[&str]) -> Result<()> {
        match args {
            [] | ["show"] => {
                let lines = self.config_lines();
                self.state.push(Entry::Block(lines));
            }
            ["init"] | ["set", "api_key"] | ["set", "base_url"] | ["set", "model"]
            | ["set", "thinking"] => self.open_model_form(),
            ["set", _] => bail!("请提供 value：/config set <key> <value>"),
            ["set", key, value @ ..] => {
                let value = value.join(" ");
                self.app.file.set(key, &value)?;
                self.app.save()?;
                let cfg = self.app.effective();
                self.state.push(Entry::Success(format!(
                    "已保存 {key} = {}",
                    display_value(&self.app.file, key)
                )));
                if let Some(name) = env_override(key) {
                    self.state.push(Entry::Warn(format!(
                        "环境变量 {name} 已设置，它会覆盖配置文件中的 {key}"
                    )));
                }
                for w in cfg.warnings() {
                    self.state.push(Entry::Warn(w));
                }
            }
            ["unset", key] => {
                self.app.file.unset(key)?;
                self.app.save()?;
                self.state.push(Entry::Success(format!(
                    "{key} 已恢复默认值：{}",
                    display_value(&self.app.file, key)
                )));
            }
            _ => bail!("用法：/config [show|set <key> <value>|unset <key>]"),
        }
        Ok(())
    }

    fn config_lines(&self) -> Vec<Line<'static>> {
        let dim = Style::new().add_modifier(Modifier::DIM);
        let cfg = self.app.effective();
        let mut out = vec![Line::from(vec![
            Span::styled("配置文件 ", Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(ui::tilde(&self.app.config_path)),
            Span::styled(
                if self.app.config_path.exists() {
                    ""
                } else {
                    "（尚未创建，使用默认值）"
                },
                dim,
            ),
        ])];
        for &key in config::KEYS {
            let mut spans = vec![
                Span::styled(format!("  {key:<13} "), dim),
                Span::raw(display_value(&cfg, key)),
            ];
            if let Some(src) = source_label(self.app, &cfg, key) {
                spans.push(Span::styled(format!("  ← {src}"), dim));
            }
            out.push(Line::from(spans));
        }
        out.push(Line::from(vec![
            Span::styled("  对话目录      ", dim),
            Span::raw(ui::tilde(self.store.dir())),
        ]));
        out
    }

    fn show_permissions(&mut self) {
        let cfg = self.app.effective();
        let lines = vec![
            Line::from(vec![
                Span::styled("权限设置", Style::new().add_modifier(Modifier::BOLD)),
            ]),
            Line::raw(""),
            Line::from(vec![
                Span::styled("  权限模式           ", Style::new().add_modifier(Modifier::DIM)),
                Span::raw(cfg.permission_mode.label()),
            ]),
            Line::from(vec![
                Span::styled("  工作目录           ", Style::new().add_modifier(Modifier::DIM)),
                Span::raw(
                    cfg.workspace_root
                        .as_ref()
                        .map_or_else(|| "默认（启动时的当前目录）".to_string(), |p| p.display().to_string())
                ),
            ]),
            Line::raw(""),
            Line::styled("用法：", Style::new().add_modifier(Modifier::DIM)),
            Line::raw("  /permissions readonly           只读，不能修改文件或执行命令"),
            Line::raw("  /permissions editable           可编辑，能修改文件，不能执行命令"),
            Line::raw("  /permissions full               全权限，能修改文件和执行命令（删除除外）"),
            Line::raw("  /permissions <模式> <目录>      同时设置权限模式和工作目录"),
        ];
        self.state.push(Entry::Block(lines));
    }

    fn set_permission_mode(&mut self, mode: &str) -> Result<()> {
        self.app.file.set("permission_mode", mode)?;
        self.app.save()?;
        self.state.push(Entry::Success(format!(
            "权限模式已保存：{}",
            display_value(&self.app.effective(), "permission_mode")
        )));
        Ok(())
    }
}

/// 表单里会被环境变量覆盖的配置项：（配置键, 表单里的名字）
const ENV_FIELDS: [(&str, &str); 3] = [
    ("api_key", "API Key"),
    ("base_url", "API 地址"),
    ("model", "模型"),
];

/// 保存成功后显示的摘要。Key 只显示首尾，完整内容在配置文件里。
fn saved_lines(path: &str, s: &ModelSettings) -> Vec<Line<'static>> {
    let dim = Style::new().add_modifier(Modifier::DIM);
    let row = |label: &str, value: String| {
        let pad = " ".repeat(9usize.saturating_sub(label.width()));
        Line::from(vec![
            Span::styled(format!("  {label}{pad}"), dim),
            Span::raw(value),
        ])
    };
    vec![
        row("API 地址", s.base_url.clone()),
        row(
            "API Key",
            s.api_key
                .as_deref()
                .map_or_else(|| "未设置".to_string(), mask_key),
        ),
        row("模型", s.model.clone()),
        row("思考力度", s.thinking.to_string()),
        Line::styled(format!("  已写入 {path}，以后每次启动都会使用"), dim),
    ]
}

fn help_lines() -> Vec<Line<'static>> {
    let name_w = COMMANDS.iter().map(|c| c.name.len()).max().unwrap_or(0) + 2;
    let dim = Style::new().add_modifier(Modifier::DIM);
    let mut out: Vec<Line<'static>> = COMMANDS
        .iter()
        .map(|c| {
            Line::from(vec![
                Span::styled(
                    format!("  {:<name_w$}", c.name),
                    Style::new().fg(Color::Cyan),
                ),
                Span::raw(c.desc),
            ])
        })
        .collect();
    out.push(Line::raw(""));
    out.push(Line::styled(
        "  /model 表单：↑↓ 切换项 · 直接输入修改 · ←→ 调整思考力度 · Ctrl-S 保存 · Esc 取消",
        dim,
    ));
    out.push(Line::styled(
        "  Ctrl-J 换行 · PgUp/PgDn 滚动 · Ctrl-B 到顶部 · Ctrl-G 到底部",
        dim,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_summary_masks_the_key_and_aligns_the_values() {
        let s = ModelSettings {
            base_url: "https://api.deepseek.com".into(),
            api_key: Some("sk-1234567890abcdef".into()),
            model: "m".into(),
            thinking: Thinking::High,
        };
        let lines: Vec<String> = saved_lines("~/c.toml", &s)
            .iter()
            .map(crate::chat::line_text)
            .collect();
        assert!(lines.iter().all(|l| !l.contains("sk-1234567890abcdef")));
        assert!(lines[4].contains("~/c.toml") && lines[4].contains("以后每次启动都会使用"));
        // 四行的值从同一显示列开始（中文占两列）
        let values = ["https://api.deepseek.com", "sk-…cdef", "m", "high"];
        for (line, value) in lines.iter().zip(values) {
            let at = line.find(value).unwrap_or_else(|| panic!("{value} in {line:?}"));
            assert_eq!(line[..at].width(), 11, "{line:?}");
        }
    }

    #[test]
    fn unset_key_is_summarised_as_unset() {
        let s = ModelSettings {
            base_url: "https://x.example".into(),
            api_key: None,
            model: "m".into(),
            thinking: Thinking::Auto,
        };
        let lines: Vec<String> = saved_lines("p", &s)
            .iter()
            .map(crate::chat::line_text)
            .collect();
        assert!(lines[1].contains("未设置"));
    }

    #[test]
    fn help_lists_every_command() {
        let text: Vec<String> = help_lines().iter().map(crate::chat::line_text).collect();
        for c in COMMANDS {
            assert!(text.iter().any(|l| l.contains(c.name)), "missing {}", c.name);
        }
    }
}
