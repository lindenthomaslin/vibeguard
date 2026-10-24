//! 全屏界面：顶部图标栏、中间对话与选择区、固定在底部的输入框和状态栏。
//!
//! 键盘线程、AI 流式任务和定时器都往同一个通道发消息，主循环每批处理完再重绘一次，
//! 流式输出再快也不会逐 token 重绘。

mod form;
mod layout;
mod sessions;
mod state;
mod view;

use crate::app::App;
use crate::chat::Entry;
use crate::commands::{display_value, env_override, source_label};
use crate::config::{self, Thinking, mask_key};
use crate::finding::{Finding, Fix};
use crate::graphics::Graphics;
use crate::i18n::{pick, tr};
use crate::llm::{ChatOptions, ChatResponse, LlmClient, Message, Role, StreamEvent};
use crate::mode::Mode;
use crate::sessions::{Conversation, Store, format_ago};
use crate::theme::{FontStyle, Look, Theme};
use crate::ui;
use anyhow::{Context, Result, bail};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use crossterm::execute;
use form::ModelSettings;
use ratatui::DefaultTerminal;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use sessions::SessionItem;
use state::{Action, Answer, AppState, COMMANDS, start_demo};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use unicode_width::UnicodeWidthStr;
use view::Props;

/// 底部那行快捷键提示
fn hint() -> &'static str {
    crate::i18n::t(
        "Enter 发送 · Ctrl-J 换行 · Shift-Tab 换模式 · / 命令 · Ctrl-D 退出",
        "Enter send · Ctrl-J newline · Shift-Tab mode · / commands · Ctrl-D quit",
    )
}

fn busy_hint() -> &'static str {
    crate::i18n::t("Esc 取消 · PgUp/PgDn 滚动", "Esc cancel · PgUp/PgDn scroll")
}

/// 一轮对话的最终结果
struct ChatDone {
    res: ChatResponse,
    elapsed: Duration,
    /// 提前停下的原因（轮数用尽、原地打转）。有值时要单独提示，别让它看起来像正常结束
    stopped: Option<String>,
}

/// 后台任务发回的消息都带着发起时的请求编号；取消后编号变化，迟到的消息会被丢弃
enum Msg {
    Term(Event),
    Reasoning(u64, String),
    Content(u64, String),
    Retry(u64, String),
    Done(u64, Result<ChatDone>),
    /// 一次工具调用：让用户在等待期间看得见它在干什么
    Tool {
        req: u64,
        name: String,
        detail: String,
        ok: bool,
    },
    /// 自动模式下已经写好的改动，回放给用户看
    Diff {
        req: u64,
        path: String,
        old: String,
        new: String,
    },
    /// 逐条确认模式下，一处改动等着用户点头。任务会停在这里等 `reply`
    Proposal {
        req: u64,
        path: String,
        old: String,
        new: String,
        reply: oneshot::Sender<bool>,
    },
    /// 用户确认之后写入的结果
    Applied {
        req: u64,
        path: String,
        ok: bool,
    },
    /// 本地规则扫描跑完了
    ScanDone(u64, crate::scan::LocalReport),
    /// 第二段的进度：第几批、判什么
    ScanProgress(u64, String),
    /// 第二段跑完了（判定、失败批次、这次依据的仓库快照）
    ScanAiDone(
        u64,
        Vec<crate::scan_ai::Judgement>,
        Vec<String>,
        crate::scan_ai::ScanDigest,
    ),
    /// `/fix` 的进度
    FixProgress(u64, String),
    /// `/fix` 生成好了待确认的修复
    FixesReady(u64, Vec<Finding>, Vec<Fix>),
    Tick,
}

/// 进入全屏界面，直到用户退出。终端状态在返回前（含出错、panic）都会还原。
pub async fn run(app: &mut App) -> Result<()> {
    // 查询终端图形协议要在接管终端之前做完：子进程会临时切换终端模式
    let gfx = Graphics::detect();
    let mut terminal = ratatui::try_init()?;
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    let result = event_loop(app, &mut terminal, gfx).await;
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
    /// 逐条确认模式下，还在等用户答复的那个后台任务
    pending_reply: Option<oneshot::Sender<bool>>,
    /// 用户选了「本次全部应用」：这一轮请求剩下的改动不再逐条询问
    always_apply: Arc<AtomicBool>,
    last_usage: String,
    cwd: String,
    /// `/scan` 扫的目录；`/fix` 写文件时以它为基准，不能用错目录
    scan_root: std::path::PathBuf,
    /// 这次扫描的存档编号（同一轮的两段共用，第二段覆盖第一段）
    scan_id: Option<String>,
    /// 这次扫描开始的时间（Unix 秒），存档里用它当 created
    scan_started: i64,
    /// 终端原生图片（不支持时是空的，启动画面退回字标）
    gfx: Graphics,
    /// 启动画面只在还没开口时显示
    show_banner: bool,
    /// 需要整屏重绘：原生图片从画面上消失后，旧图会留在终端里
    repaint: bool,
}

async fn event_loop(app: &mut App, terminal: &mut DefaultTerminal, gfx: Graphics) -> Result<()> {
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
        pending_reply: None,
        always_apply: Arc::new(AtomicBool::new(false)),
        last_usage: String::new(),
        cwd,
        scan_root: std::env::current_dir().unwrap_or_else(|_| ".".into()),
        scan_id: None,
        scan_started: 0,
        gfx,
        show_banner: true,
        repaint: false,
    };
    session.welcome();
    // 第一次在这个目录里启动：先问要不要信任。
    // 仓库里的注释、文档、测试夹具都可能被模型当成「指令」读进去，
    // 没点过头之前不主动放开写入和执行。
    session.maybe_ask_trust();

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
    /// 启动画面不写进对话记录，由 `view` 画在对话区顶部；这里只准备下面的提示
    fn welcome(&mut self) {
        if self.app.effective().api_key.is_none() {
            self.state.push(Entry::Warn(
                crate::i18n::t(
                    "尚未配置 API Key：输入 /model 填写 API 地址、Key 和模型，或设置环境变量 DEEPSEEK_API_KEY",
                    "No API key yet: run /model to set the endpoint, key and model, or export DEEPSEEK_API_KEY",
                )
                .into(),
            ));
        }
        for w in self.app.effective().warnings() {
            self.state.push(Entry::Warn(w));
        }
        let mut tip = crate::i18n::t(
            "直接输入问题与 AI 对话，输入 / 查看命令",
            "Type a question to talk to the model, or / for commands",
        )
        .to_string();
        if let Ok(Some(last)) = self.store.latest(None) {
            tip.push_str(&crate::i18n::pick(
                format!(
                    "；/resume 可恢复上一次对话「{}」（{}）",
                    last.title,
                    format_ago(last.updated, now_secs())
                ),
                format!(
                    "; /resume reopens “{}” ({})",
                    last.title,
                    format_ago(last.updated, now_secs())
                ),
            ));
        }
        self.state.push(Entry::Info(tip));
    }

    fn draw(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        // 原生图片是终端直接画在屏幕上的，位置变了不会自己消失，得整屏重绘一次。
        // 清屏前 ratatui 会问一次光标位置，个别终端不回这个报告：那就算了，
        // 顶多留下一张没擦干净的图，不能因为一张图退出。
        if std::mem::take(&mut self.repaint) {
            let _ = terminal.clear();
        }
        let cfg = self.app.effective();
        let look = Look::from_config(&cfg);
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
            busy_hint().to_string()
        } else if self.last_usage.is_empty() {
            hint().to_string()
        } else {
            self.last_usage.clone()
        };
        let input_label = self.state.input_label();
        let input_active = self.state.input_active();
        // 逐条确认时任务停在等答复上，说「等待响应」会让人以为在等网络
        let busy = if self.state.is_confirming() {
            Some(tr("等你确认…").to_string())
        } else {
            self.state.busy.clone()
        };
        let (avatar, banner) = self.gfx.split();
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
                    sessions: self.state.sessions.as_ref(),
                    // 确认和逐条修复时把卡片顶部钉住，矮终端上图标栏也收起来让位
                    focus_last: self.state.review.is_some() || self.state.is_confirming(),
                    busy: busy.as_deref(),
                    tick: self.state.tick,
                    status: &status,
                    mode: self.state.mode,
                    look,
                    input_tokens,
                    scroll: &mut self.state.scroll,
                    avatar,
                    banner,
                    show_banner: self.show_banner,
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
                Action::Chat(text) => {
                    self.set_banner(false);
                    self.start_chat(text);
                }
                Action::Command(parts) => {
                    self.set_banner(false);
                    self.command(parts);
                }
                Action::Cancel => self.cancel(),
                Action::SaveModel(settings) => match self.save_settings(&settings) {
                    Ok(()) => self.state.close_form(),
                    // 保存失败：表单保持打开，已经填好的内容不会丢
                    Err(e) => self.state.push(Entry::Error(pick(
                        format!("保存失败，配置文件没有改动：{e:#}"),
                        format!("Save failed, the config file is unchanged: {e:#}"),
                    ))),
                },
                Action::ResumeSession(id) => {
                    if let Err(e) = self.resume(&id) {
                        self.state.push(Entry::Error(format!("{e:#}")));
                    }
                }
                Action::DeleteSession(id) => self.delete_session(&id),
                Action::ApplyFixes(fixes) => self.apply_fixes(fixes),
                Action::Answer(answer) => self.answer(answer),
                Action::CycleTheme => {
                    let theme = Theme::parse(&self.app.effective().theme).unwrap_or_default();
                    self.set_theme(theme.next());
                }
                Action::SetLang(lang) => self.apply_lang(lang),
                Action::Trust(yes) => self.answer_trust(yes),
                Action::SetAccent(accent) => self.set_accent(accent),
                Action::SetTheme(theme) => self.set_theme(theme),
                Action::SetFont(font) => self.set_font(font),
                Action::SetMode(mode) => self.state.set_mode(mode),
                Action::SetPermission(mode) => {
                    if let Err(e) = self.set_permission_mode(mode.code()) {
                        self.state.push(Entry::Error(format!("{e:#}")));
                    }
                }
            },
            Msg::Term(Event::Paste(text)) => self.state.on_paste(&text),
            // 窗口尺寸变了，启动画面可能换档或者被收起，顺带整屏重绘一次
            Msg::Term(Event::Resize(..)) => self.repaint = true,
            Msg::Term(_) => {} // 其余事件：下面统一重绘
            Msg::Reasoning(req, s) if req == self.req => self.state.on_reasoning(&s),
            Msg::Content(req, s) if req == self.req => self.state.on_content(&s),
            Msg::Retry(req, s) if req == self.req => self.state.push(Entry::Warn(s)),
            Msg::Diff {
                req,
                path,
                old,
                new,
            } if req == self.req => {
                self.state.push(Entry::Diff { path, old, new });
            }
            Msg::Proposal {
                req,
                path,
                old,
                new,
                reply,
            } if req == self.req => {
                self.pending_reply = Some(reply);
                self.state.ask_confirm(path, old, new);
            }
            Msg::Applied { req, path, ok } if req == self.req => {
                let text = if ok {
                    Entry::Success(pick(format!("已写入 {path}"), format!("Wrote {path}")))
                } else {
                    Entry::Warn(pick(
                        format!("{path} 写入失败，原因见下面的工具返回"),
                        format!("{path} write failed; see the tool result below"),
                    ))
                };
                self.state.push(text);
            }
            Msg::Done(req, result) if req == self.req => self.finish_chat(result),
            Msg::Tool {
                req,
                name,
                detail,
                ok,
            } if req == self.req => {
                self.state.push(Entry::Tool { name, detail, ok });
            }
            Msg::FixProgress(req, line) if req == self.req => {
                self.state.push(Entry::Info(line));
            }
            Msg::FixesReady(req, findings, fixes) if req == self.req => {
                self.finish_task();
                if fixes.is_empty() {
                    self.state.push(Entry::Warn(
                        tr("没有生成出可用的修复：模型要么没改，要么返回的内容不完整").into(),
                    ));
                } else {
                    // 计划模式或只读权限下照常给出 diff，但绝不落盘
                    let reason = if self.state.mode == Mode::Plan {
                        Some(tr("计划模式只读").to_string())
                    } else if matches!(
                        self.app.effective().permission_mode,
                        crate::tools::PermissionMode::ReadOnly
                    ) {
                        Some(tr("权限模式是 readonly").to_string())
                    } else {
                        None
                    };
                    self.state.start_review(&findings, &fixes, reason);
                }
            }
            Msg::ScanProgress(req, line) if req == self.req => {
                self.state.push(Entry::Info(line));
            }
            Msg::ScanAiDone(req, judgements, failed, digest) if req == self.req => {
                self.finish_task();
                self.state.digest = Some(digest);
                // 两段的结果合成一份，`/fix` 以后只看 state.findings
                let seq = self.state.findings.len();
                let list = crate::vuln_checklist::VulnChecklist::new();
                // 模型给的「文件:行号」先核实再收：核实不了的宁可不报，
                // 一条指错地方的高危发现比没有更浪费时间
                let merged = crate::scan_ai::to_findings(&self.scan_root, &judgements, &list, seq);
                self.state.findings.extend(merged.findings);
                self.state.unverified = merged
                    .unverified
                    .iter()
                    .map(|(id, ev, why)| format!("{id} {ev}（{why}）"))
                    .collect();
                if !self.state.unverified.is_empty() {
                    let shown: Vec<&str> = self
                        .state
                        .unverified
                        .iter()
                        .take(3)
                        .map(String::as_str)
                        .collect();
                    self.state.push(Entry::Warn(pick(format!(
                        "{} 条判定的证据核实不了，已剔除（{}）",
                        self.state.unverified.len(),
                        shown.join("；")
                    ), format!(
                        "{} verdicts could not be verified against the code and were dropped ({})",
                        self.state.unverified.len(),
                        shown.join("；")
                    ))));
                }
                self.state.judgements = judgements.clone();
                self.state.failed_batches = failed.clone();
                self.state.push(Entry::ScanAi { judgements, failed });
                self.save_scan();
            }
            Msg::ScanDone(req, report) if req == self.req => {
                self.finish_task();
                self.state.files_scanned = report.files_scanned;
                self.state.findings = report.findings.clone();
                self.state.push(Entry::Scan(Box::new(report)));
                self.save_scan();
            }
            // 已取消请求的迟到消息
            Msg::Reasoning(..)
            | Msg::Content(..)
            | Msg::Retry(..)
            | Msg::Done(..)
            | Msg::Diff { .. }
            | Msg::Proposal { .. }
            | Msg::Applied { .. }
            | Msg::ScanDone(..)
            | Msg::ScanProgress(..)
            | Msg::ScanAiDone(..)
            | Msg::Tool { .. }
            | Msg::FixProgress(..)
            | Msg::FixesReady(..) => {
                return false;
            }
        }
        *dirty = true;
        false
    }

    /// 收起或恢复启动画面。原生图片不会自己消失，切换时要整屏重绘一次。
    fn set_banner(&mut self, show: bool) {
        if self.show_banner != show {
            self.show_banner = show;
            self.repaint = true;
        }
    }

    /// 为新的后台任务分配编号
    fn next_req(&mut self) -> u64 {
        self.req += 1;
        self.req
    }

    fn finish_task(&mut self) {
        self.task = None;
        self.pending_reply = None;
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
        let mode = self.state.mode;
        let system = format!("{}{}", crate::repl::system_prompt(), mode.instruction());
        let mut messages = Vec::with_capacity(self.conv.messages.len() + 2);
        messages.push(Message::system(&system));
        messages.extend(self.conv.messages.iter().cloned());
        messages.push(Message::user(input.as_str()));

        self.state.begin_request(&input);
        self.pending_user = Some(input);
        let req = self.next_req();
        let tx = self.tx.clone();
        // 计划模式一律只读：配置里就算给了全权限，也不该在这档里动文件
        let perm = match mode {
            Mode::Plan => crate::tools::PermissionMode::ReadOnly,
            Mode::Confirm | Mode::Auto => self.app.file.permission_mode,
        };
        // 轮数上限来自配置：扫陌生项目几十轮很常见，写死一个数总会有人被砍断
        let max_rounds = self.app.effective().max_tool_rounds;
        // 「本次全部应用」只对当前这一轮有效，下一轮重新问
        self.always_apply.store(false, Ordering::Relaxed);
        let always = self.always_apply.clone();
        let tools = crate::tools::tool_definitions();

        self.task = Some(tokio::spawn(async move {
            let start = Instant::now();
            let events = tx.clone();

            let opts = ChatOptions {
                tools,
                ..ChatOptions::default()
            };

            // 工具调用循环
            let mut rounds = 0usize;
            // 每一轮的文字都攒起来：多步任务里模型常常「说一段 → 调工具 → 再说一段」，
            // 只留最后一轮的话，前面说过的话就丢了
            let mut answer = String::new();
            let mut last_signature: Option<String> = None;
            let mut repeats = 0usize;

            let result: Result<(ChatResponse, Duration, Option<String>)> = loop {
                rounds += 1;
                if rounds > max_rounds {
                    let note = pick(
                        format!(
                            "这一轮已经调用工具 {max_rounds} 次，先停在这里。\
                         接着说「继续」我就往下做，或者 `/config set max_tool_rounds 300` 调高上限"
                        ),
                        format!(
                            "This round already made {max_rounds} tool calls, so I am stopping here. Say \"continue\" and I will carry on, or raise the limit with `/config set max_tool_rounds 300`"
                        ),
                    );
                    break Ok((
                        ChatResponse {
                            content: String::new(),
                            reasoning: String::new(),
                            finish_reason: None,
                            usage: None,
                            model: None,
                            tool_calls: Vec::new(),
                        },
                        start.elapsed(),
                        Some(note),
                    ));
                }

                let loop_result = client
                    .chat_stream(&messages, &opts, |ev| {
                        let _ = match ev {
                            StreamEvent::Reasoning(t) => {
                                events.send(Msg::Reasoning(req, t.to_string()))
                            }
                            StreamEvent::Content(t) => {
                                events.send(Msg::Content(req, t.to_string()))
                            }
                            StreamEvent::Retrying {
                                attempt,
                                delay,
                                reason,
                            } => events.send(Msg::Retry(
                                req,
                                pick(format!(
                                    "请求失败，{:.1}s 后第 {attempt} 次重试：{reason}",
                                    delay.as_secs_f64()
                                ), format!(
                                    "Request failed, retrying in {:.1}s (attempt {attempt}): {reason}",
                                    delay.as_secs_f64()
                                )),
                            )),
                        };
                    })
                    .await;

                match loop_result {
                    Ok(res) => {
                        answer.push_str(&res.content);
                        if !res.wants_tool_calls() {
                            break Ok((res, start.elapsed(), None));
                        }

                        // 添加助理的工具调用请求
                        messages.push(Message::tool_calls(res.tool_calls.clone()));

                        for call in &res.tool_calls {
                            let tool_name = &call.function.name;
                            let tool_args = &call.function.arguments;
                            let (label, detail) = tool_label(tool_name, tool_args);

                            // 同样的工具 + 同样的参数连着来三次，基本就是在原地打转
                            let signature = format!("{tool_name}\u{1}{tool_args}");
                            if last_signature.as_deref() == Some(signature.as_str()) {
                                repeats += 1;
                            } else {
                                repeats = 1;
                                last_signature = Some(signature);
                            }

                            let is_write = matches!(tool_name.as_str(), "edit_file" | "write_file");
                            let (path, old_code, new_code) = write_preview(tool_name, tool_args);

                            // 计划模式：写操作直接挡回去，让模型把建议写进报告
                            if is_write && mode == Mode::Plan {
                                messages.push(Message::tool_result(
                                    call.id.clone(),
                                    tr("计划模式下不能修改文件：请把修复建议写进报告，\
                                     由用户切换到逐条确认或全自动模式后再动手。")
                                    .to_string(),
                                ));
                                // 挡回去这件事要让用户看见，否则模型反复尝试时屏幕是静的
                                let _ = events.send(Msg::Tool {
                                    req,
                                    name: label,
                                    detail: pick(
                                        format!("{detail}（计划模式，已挡回）"),
                                        format!("{detail} (plan mode, blocked)"),
                                    ),
                                    ok: false,
                                });
                                continue;
                            }

                            // 逐条确认：先把改动摊给用户看，等他点头再写
                            if is_write && mode == Mode::Confirm && !always.load(Ordering::Relaxed)
                            {
                                let (reply, verdict) = oneshot::channel();
                                let sent = events.send(Msg::Proposal {
                                    req,
                                    path: path.clone(),
                                    old: old_code.clone(),
                                    new: new_code.clone(),
                                    reply,
                                });
                                // 发不出去说明界面已经关了，这个任务也没有存在的意义
                                if sent.is_err() {
                                    return;
                                }
                                if !verdict.await.unwrap_or(false) {
                                    messages.push(Message::tool_result(
                                        call.id.clone(),
                                        tr("用户拒绝了这个改动。不要重试，继续处理下一个问题。")
                                            .to_string(),
                                    ));
                                    let _ = events.send(Msg::Tool {
                                        req,
                                        name: label,
                                        detail: pick(
                                            format!("{detail}（你拒绝了）"),
                                            format!("{detail} (you declined)"),
                                        ),
                                        ok: false,
                                    });
                                    continue;
                                }
                            }

                            // 先报「正在做什么」，再执行：慢命令（跑测试）期间屏幕不能是静的
                            let _ = events.send(Msg::Tool {
                                req,
                                name: label.clone(),
                                detail: detail.clone(),
                                ok: true,
                            });

                            let mut tool_result =
                                crate::tools::execute_tool(tool_name, tool_args, &perm);
                            let denied = tool_result.starts_with(tr("错误：权限不足"));
                            // plan 是硬性只读，和 permission_mode 无关。不把这一点说清楚，
                            // 用户会以为「我明明把权限设成 full 了」
                            if denied && mode == Mode::Plan {
                                // 整段换掉而不是追加：原话里的「改成 full」在 plan 下是错建议，
                                // 模型会把两条都念给用户，越说越乱
                                tool_result = tr("错误：当前工作模式是 plan，它强制只读，\
                                     改 permission_mode 不会生效。要执行这类命令，\
                                     需要用户按 Shift-Tab 切到 confirm 或 auto；\
                                     只想看代码的话用 rg / find / cat。")
                                .to_string();
                            }
                            let ok = !tool_result.starts_with(tr("错误"));
                            if !ok {
                                // 界面上一句话就够，完整原因在工具返回里给模型看
                                let detail = if denied {
                                    if mode == Mode::Plan {
                                        tr("plan 模式强制只读 · Shift-Tab 切到 confirm/auto")
                                            .to_string()
                                    } else {
                                        tr("permission_mode 是 readonly · 用 /config set 改成 full")
                                            .to_string()
                                    }
                                } else {
                                    tool_result.lines().next().unwrap_or("").to_string()
                                };
                                let _ = events.send(Msg::Tool {
                                    req,
                                    name: pick(
                                        format!("{label} 被拒绝"),
                                        format!("{label} declined"),
                                    ),
                                    detail,
                                    ok: false,
                                });
                            }

                            if is_write {
                                if mode == Mode::Confirm {
                                    let _ = events.send(Msg::Applied {
                                        req,
                                        path: path.clone(),
                                        ok,
                                    });
                                } else {
                                    let _ = events.send(Msg::Diff {
                                        req,
                                        path: path.clone(),
                                        old: old_code.clone(),
                                        new: new_code.clone(),
                                    });
                                }
                            }

                            messages.push(Message::tool_result(call.id.clone(), tool_result));
                        }

                        if repeats >= 3 {
                            break Ok((
                                res,
                                start.elapsed(),
                                Some(pick(
                                    format!(
                                        "模型连着 {repeats} 次用同样的参数调用同一个工具，\
                                     判断为原地打转，先停下来。换个说法，或者 /scan 看看现状"
                                    ),
                                    format!(
                                        "The model called the same tool with the same arguments {repeats} times in a row, which looks like spinning in place, so I am stopping. Rephrase, or run /scan to see where things stand"
                                    ),
                                )),
                            ));
                        }
                    }
                    Err(e) => break Err(e),
                }
            };

            let _ = tx.send(Msg::Done(
                req,
                result.map(|(mut res, elapsed, stopped)| {
                    if let Some(note) = &stopped {
                        if !answer.is_empty() && !answer.ends_with('\n') {
                            answer.push('\n');
                        }
                        answer.push_str(&format!("\n> {note}"));
                    }
                    res.content = answer;
                    ChatDone {
                        res,
                        elapsed,
                        stopped,
                    }
                }),
            ));
        }));
    }

    /// 逐条确认的答复：关掉卡片，把结果送回还在等着的后台任务
    fn answer(&mut self, answer: Answer) {
        if answer == Answer::Always {
            self.always_apply.store(true, Ordering::Relaxed);
        }
        self.state.close_confirm(answer);
        if let Some(reply) = self.pending_reply.take() {
            // 任务已经结束（比如用户刚取消）时发送会失败，忽略即可
            let _ = reply.send(answer.approved());
        }
    }

    fn finish_chat(&mut self, result: Result<ChatDone>) {
        self.finish_task();
        let user = self.pending_user.take();
        match result {
            Ok(done) => {
                if done.res.is_truncated() {
                    self.state
                        .push(Entry::Warn(tr("回答达到 max_tokens 上限被截断").into()));
                }
                // 提前停下（轮数用尽 / 原地打转）不是错误，但必须说清楚，
                // 否则用户会以为「它就干到这里为止」
                if let Some(reason) = done.stopped {
                    self.state.push(Entry::Warn(reason));
                }
                self.last_usage = ui::usage_text(&done.res, done.elapsed);
                // 一个字都没说出来就别往历史里塞一条空的
                if let Some(user) = user
                    && !done.res.content.trim().is_empty()
                {
                    self.conv.model = self.app.effective().model;
                    self.conv
                        .push_exchange(&user, &done.res.content, now_secs());
                    if let Err(e) = self.store.save(&self.conv) {
                        self.state.push(Entry::Warn(pick(
                            format!("对话保存失败：{e:#}"),
                            format!("Saving the conversation failed: {e:#}"),
                        )));
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
        self.pending_reply = None;
        self.state.end_request();
        self.state
            .push(Entry::Warn(crate::i18n::t("已取消", "Cancelled").into()));
    }

    // ---- 模型配置 ----

    /// 打开 `/model` 表单。初始值取自配置文件，而不是环境变量覆盖后的值，
    /// 否则一保存就会把环境变量里的 Key 写进文件。
    fn open_model_form(&mut self) {
        for (key, label) in ENV_FIELDS.map(|(k, l)| (k, tr(l))) {
            if let Some(name) = env_override(key) {
                self.state.push(Entry::Warn(pick(format!(
                    "环境变量 {name} 已设置，会覆盖配置文件里的{label}；表单显示的是配置文件里的值"
                ), format!(
                    "Env var {name} is set and overrides the {label} in the config file; the form shows the config value"
                ))));
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

        self.state.push(Entry::Success(tr("配置已保存").into()));
        let lines = saved_lines(&ui::tilde(&self.app.config_path), settings);
        self.state.push(Entry::Block(lines));
        for (key, label) in ENV_FIELDS.map(|(k, l)| (k, tr(l))) {
            if let Some(name) = env_override(key) {
                self.state.push(Entry::Warn(pick(
                    format!("环境变量 {name} 已设置，运行时它仍会覆盖刚保存的{label}"),
                    format!(
                        "Env var {name} is set and will still override the {label} you just saved"
                    ),
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
        // 空对话重新显示启动画面
        self.set_banner(true);
        if had_messages {
            self.state.push(Entry::Success(
                tr("已开始新对话，之前的对话已保存，可用 /resume 或 /sessions 找回").into(),
            ));
        } else {
            self.state.push(Entry::Success(tr("已开始新对话").into()));
        }
    }

    fn resume(&mut self, id: &str) -> Result<()> {
        if id == self.conv.id {
            self.state
                .push(Entry::Info(tr("已经在这个对话里了").into()));
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
                .push(Entry::Info(tr("还没有可以恢复的历史对话").into())),
        }
        Ok(())
    }

    /// 用历史对话替换当前对话，并把消息重新画到对话区
    fn load_conversation(&mut self, conv: Conversation) {
        self.state.transcript.clear();
        self.last_usage.clear();
        // 已经有内容了，不再显示启动画面
        self.set_banner(false);
        self.state.push(Entry::Info(pick(
            format!(
                "已恢复对话「{}」· {} 轮 · {}",
                conv.title,
                conv.turns(),
                format_ago(conv.updated, now_secs())
            ),
            format!(
                "Resumed \"{}\" · {} rounds · {}",
                conv.title,
                conv.turns(),
                format_ago(conv.updated, now_secs())
            ),
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

    /// `/sessions` 和 `/search` 共用：读一遍目录，把列表和正文开头交给面板，
    /// 之后过滤都在内存里做
    /// 把当前结果存档。两段跑完各存一次：先存本地规则的结果，
    /// 模型判完再覆盖同一份，这样中途退出也留得下东西。
    fn save_scan(&mut self) {
        let Some(id) = self.scan_id.clone() else {
            return;
        };
        // 指纹按「现在磁盘上的样子」算：这就是这次扫描看到的版本
        let mut fingerprints = std::collections::BTreeMap::new();
        for f in &self.state.findings {
            if let Some(fp) = crate::scan_store::Fingerprint::of(&self.scan_root.join(&f.file)) {
                fingerprints.insert(f.file.clone(), fp);
            }
        }
        self.state.fingerprints = fingerprints.clone();

        let scan = crate::scan_store::StoredScan {
            id: id.clone(),
            root: self.scan_root.to_string_lossy().to_string(),
            created: self.scan_started,
            version: env!("CARGO_PKG_VERSION").to_string(),
            files_scanned: self.state.files_scanned,
            findings: self.state.findings.clone(),
            judgements: self.state.judgements.clone(),
            failed_batches: self.state.failed_batches.clone(),
            fingerprints,
            digest_hash: self.state.digest.as_ref().map(|d| d.hash).unwrap_or(0),
            digest_files: self
                .state
                .digest
                .as_ref()
                .map(|d| d.files.clone())
                .unwrap_or_default(),
        };
        let store = crate::scan_store::ScanStore::beside_config(&self.app.config_path);
        match store.save(&scan) {
            Ok(path) => self.state.push(Entry::Info(pick(
                format!("结果已存档：{}", ui::tilde(&path)),
                format!("Results archived: {}", ui::tilde(&path)),
            ))),
            // 存档失败不影响这次扫描，只是下次 /findings 找不回来
            Err(e) => self.state.push(Entry::Warn(pick(
                format!("存档失败（结果只在这次会话里）：{e:#}"),
                format!("Archiving failed (results only live in this session): {e:#}"),
            ))),
        }
    }

    /// `/findings`：载入这个目录上次的扫描结果，不重新扫、不花钱
    fn load_findings(&mut self) -> Result<()> {
        let store = crate::scan_store::ScanStore::beside_config(&self.app.config_path);
        let Some(scan) = store.latest_for(&self.scan_root)? else {
            let others = store.list().map(|v| v.len()).unwrap_or(0);
            if others > 0 {
                bail!(
                    "{}",
                    pick(
                        format!(
                            "这个目录还没有扫描记录：{}（存档里有 {others} 条其他目录的记录）",
                            ui::tilde(self.scan_root.as_path())
                        ),
                        format!(
                            "No scan record for this directory: {} (the archive has {others} records for other directories)",
                            ui::tilde(self.scan_root.as_path())
                        )
                    )
                );
            }
            bail!(
                "{}",
                pick(
                    format!(
                        "还没有任何扫描记录：{}",
                        ui::tilde(self.scan_root.as_path())
                    ),
                    format!(
                        "No scan records yet: {}",
                        ui::tilde(self.scan_root.as_path())
                    )
                )
            );
        };

        let ago = format_ago(scan.created, now_secs());
        self.state.findings = scan.findings.clone();
        self.state.judgements = scan.judgements.clone();
        self.state.failed_batches = scan.failed_batches.clone();
        self.state.files_scanned = scan.files_scanned;
        self.state.fingerprints = scan.fingerprints.clone();
        self.state.digest = Some(crate::scan_ai::ScanDigest {
            hash: scan.digest_hash,
            files: scan.digest_files.clone(),
        });
        self.scan_id = Some(scan.id.clone());
        self.scan_started = scan.created;

        self.state.push(Entry::Info(pick(
            format!(
                "载入 {ago}的扫描结果（{}）· 共 {} 个问题",
                ui::tilde(self.scan_root.as_path()),
                scan.findings.len()
            ),
            format!(
                "Loading the scan from {ago} ({}) · {} issues",
                ui::tilde(self.scan_root.as_path()),
                scan.findings.len()
            ),
        )));

        // 把结果按「一次扫描」的样子重新画一遍
        let files_with_hits = scan
            .findings
            .iter()
            .map(|f| f.file.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        self.state
            .push(Entry::Scan(Box::new(crate::scan::LocalReport {
                findings: scan.findings.clone(),
                files_scanned: scan.files_scanned,
                files_skipped: 0,
                files_with_hits,
                truncated: false,
                rules: crate::scan::covered_ids().len(),
                checklist_total: crate::vuln_checklist::VulnChecklist::new().len(),
            })));

        let stale = scan.stale_files(&self.scan_root);
        if stale.is_empty() {
            self.state
                .push(Entry::Info(tr("文件都没变，可以直接 /fix").into()));
        } else {
            let shown: Vec<&str> = stale.iter().take(5).map(String::as_str).collect();
            let more = stale.len().saturating_sub(shown.len());
            self.state.push(Entry::Warn(pick(
                format!(
                    "有 {} 个文件在扫描后改过（{}{}）：相关发现可能过期，/fix 会跳过它们",
                    stale.len(),
                    shown.join("、"),
                    if more > 0 {
                        pick(format!(" 等 {more} 个"), format!(" and {more} more"))
                    } else {
                        String::new()
                    }
                ),
                format!(
                    "{} files changed since the scan ({}{}): related findings may be stale, /fix will skip them",
                    stale.len(),
                    shown.join("、"),
                    if more > 0 {
                        pick(format!(" 等 {more} 个"), format!(" and {more} more"))
                    } else {
                        String::new()
                    }
                ),
            )));
        }
        Ok(())
    }

    /// `/report [路径]`
    ///
    /// 把最近一次扫描落成两份文件：`.md` 给人看，`.json` 给 CI 看。
    /// 传路径时按「去掉扩展名的主干」处理，两份都写在它旁边。
    fn write_report(&mut self, args: &[&str]) -> Result<()> {
        if self.state.findings.is_empty() && self.state.judgements.is_empty() {
            bail!("{}", tr("还没有扫描结果，先跑 /scan 或 /scan --ai"));
        }
        let base = match args.first() {
            Some(p) => {
                let mut path = std::path::PathBuf::from(p);
                path.set_extension("");
                path
            }
            None => self.scan_root.join("vibeguard-report"),
        };
        if let Some(dir) = base.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| {
                pick(
                    format!("创建目录失败：{}", dir.display()),
                    format!("Creating the directory failed: {}", dir.display()),
                )
            })?;
        }

        let meta = crate::report::Meta {
            root: ui::tilde(&self.scan_root),
            when: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            local_rules: crate::scan::covered_ids().len(),
            checklist_total: crate::vuln_checklist::VulnChecklist::new().len(),
            files_scanned: self.state.files_scanned,
            failed: self.state.failed_batches.clone(),
            unverified: self.state.unverified.clone(),
            skipped_items: self.state.skipped_items,
        };
        let md = crate::report::markdown(&self.state.findings, &self.state.judgements, &meta);
        let js = crate::report::json(&self.state.findings, &self.state.judgements, &meta);

        let md_path = base.with_extension("md");
        let json_path = base.with_extension("json");
        std::fs::write(&md_path, md).with_context(|| {
            pick(
                format!("写入失败：{}", md_path.display()),
                format!("Write failed: {}", md_path.display()),
            )
        })?;
        std::fs::write(&json_path, js).with_context(|| {
            pick(
                format!("写入失败：{}", json_path.display()),
                format!("Write failed: {}", json_path.display()),
            )
        })?;

        self.state.push(Entry::Success(crate::i18n::pick(
            format!(
                "报告已写入 {}（共 {} 个问题）",
                ui::tilde(&md_path),
                self.state.findings.len()
            ),
            format!(
                "Report written to {} ({} findings)",
                ui::tilde(&md_path),
                self.state.findings.len()
            ),
        )));
        self.state.push(Entry::Info(crate::i18n::pick(
            format!("同一份数据的机器可读版本：{}", ui::tilde(&json_path)),
            format!("Machine-readable version: {}", ui::tilde(&json_path)),
        )));
        // 空报告最容易骗人，这里再说一遍覆盖了多少
        if self.state.findings.is_empty() {
            self.state.push(Entry::Warn(pick(format!(
                "这份报告是「没发现问题」，不是「没有问题」——它只看了清单里的 {}/{} 条",
                (meta.local_rules + self.state.judgements.len()).min(meta.checklist_total),
                meta.checklist_total
            ), format!(
                "This report says \"nothing found\", not \"nothing wrong\" — it only covered {}/{} checklist items",
                (meta.local_rules + self.state.judgements.len()).min(meta.checklist_total),
                meta.checklist_total
            ))));
        }
        Ok(())
    }

    /// `/fix [数量] [编号]`
    ///
    /// 拿 `/scan` 的结果去让模型生成修复，然后走逐条确认。真正写盘发生在
    /// 用户按 y 之后（`Action::ApplyFixes`），这里一个字节都不动。
    fn start_fix(&mut self, args: &[&str]) -> Result<()> {
        if self.state.busy.is_some() {
            bail!(
                "{}",
                crate::i18n::t(
                    "正在忙，等这次结束或者按 Esc 取消",
                    "Busy — wait for this to finish, or press Esc to cancel"
                )
            );
        }
        if self.state.findings.is_empty() {
            bail!("{}", tr("还没有扫描结果，先跑 /scan 或 /scan --ai"));
        }
        // 只读时仍然生成 diff（看方案本身有价值），但不会落盘——
        // 真正的拦截在 apply_fixes 之前：answer_review 直接返回 None
        let readonly = matches!(
            self.app.effective().permission_mode,
            crate::tools::PermissionMode::ReadOnly
        );
        if readonly || self.state.mode == Mode::Plan {
            self.state.push(Entry::Info(
                tr("当前是计划模式 / 只读权限：会给出修复方案，但不会写入文件").into(),
            ));
        }

        let limit = args
            .iter()
            .find_map(|a| a.parse::<usize>().ok())
            .unwrap_or(5)
            .clamp(1, 20);
        let wanted = args.iter().find(|a| a.starts_with("VG-")).copied();

        // 只挑文件真的存在、而且扫描之后没被改过的发现
        let mut picked: Vec<Finding> = Vec::new();
        let mut stale = 0usize;
        for f in &self.state.findings {
            if picked.len() >= limit {
                break;
            }
            if let Some(id) = wanted
                && f.id != id
            {
                continue;
            }
            let path = self.scan_root.join(&f.file);
            if !path.is_file() {
                continue;
            }
            // 文件在扫描后改过：行号可能已经不对了，宁可不修
            let fresh = crate::scan_store::is_fresh(&self.scan_root, &self.state.fingerprints, f);
            if !fresh {
                stale += 1;
                continue;
            }
            picked.push(f.clone());
        }
        if picked.is_empty() {
            if stale > 0 {
                bail!(
                    "{}",
                    pick(
                        format!(
                            "{stale} 条发现所在的文件在扫描后改过，行号可能已经不对了：先重扫再修"
                        ),
                        format!(
                            "{stale} findings live in files changed since the scan, so line numbers may be stale: rescan first"
                        )
                    )
                );
            }
            bail!("{}", tr("没有可以修的发现（文件不存在，或者编号对不上）"));
        }
        if stale > 0 {
            self.state.push(Entry::Warn(pick(format!(
                "跳过 {stale} 条：文件在扫描后改过，先重扫这些文件再修"
            ), format!(
                "Skipping {stale} findings: their files changed since the scan, rescan them before fixing"
            ))));
        }

        let client = LlmClient::new(&self.app.effective()).context(tr("无法创建模型客户端"))?;
        let req = self.next_req();
        let tx = self.tx.clone();
        let root = self.scan_root.clone();
        let total = picked.len();
        self.state.busy = Some(tr("生成修复中…").into());
        self.state.push(Entry::Info(pick(
            format!("正在为 {total} 个问题生成修复（每个问题一次请求，不会写文件）…"),
            format!("Generating fixes for {total} issues (one request each, nothing is written)…"),
        )));

        self.task = Some(tokio::spawn(async move {
            let mut fixes = Vec::new();
            let mut used: Vec<Finding> = Vec::new();
            for (i, finding) in picked.into_iter().enumerate() {
                let _ = tx.send(Msg::FixProgress(
                    req,
                    pick(
                        format!(
                            "修复 {}/{}：{} · {}:{}",
                            i + 1,
                            total,
                            finding.title,
                            finding.file,
                            finding.line
                        ),
                        format!(
                            "Fixing {}/{}: {} · {}:{}",
                            i + 1,
                            total,
                            finding.title,
                            finding.file,
                            finding.line
                        ),
                    ),
                ));
                let path = root.join(&finding.file);
                let Ok(old) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if old.len() > 120_000 {
                    let _ = tx.send(Msg::FixProgress(
                        req,
                        pick(
                            format!("跳过 {}：文件太大（{} 字节）", finding.file, old.len()),
                            format!(
                                "Skipping {}: file too large ({} bytes)",
                                finding.file,
                                old.len()
                            ),
                        ),
                    ));
                    continue;
                }
                let messages = [Message::user(crate::scan_ai::fix_prompt(&finding, &old))];
                let opts = ChatOptions {
                    json: true,
                    max_tokens: Some(8000),
                    tools: Vec::new(),
                };
                let Ok(resp) = client.chat(&messages, &opts).await else {
                    continue;
                };
                let Ok(new) = crate::scan_ai::parse_fix(&resp.content) else {
                    continue;
                };
                // 三道闸：没改、明显被截断、把文件删空，都不算修复
                if new.trim() == old.trim() {
                    let _ = tx.send(Msg::FixProgress(
                        req,
                        pick(
                            format!("跳过 {}：模型没改动任何内容", finding.file),
                            format!("Skipping {}: the model changed nothing", finding.file),
                        ),
                    ));
                    continue;
                }
                if new.len() * 10 < old.len() * 6 {
                    let _ = tx.send(Msg::FixProgress(
                        req,
                        pick(format!(
                            "跳过 {}：返回的内容只有原文的 {}%，像是被截断了",
                            finding.file,
                            new.len() * 100 / old.len().max(1)
                        ), format!(
                            "Skipping {}: the returned content is only {}% of the original, so it looks truncated",
                            finding.file,
                            new.len() * 100 / old.len().max(1)
                        )),
                    ));
                    continue;
                }
                fixes.push(Fix {
                    finding_id: finding.id.clone(),
                    file: finding.file.clone(),
                    old,
                    new,
                });
                used.push(finding);
            }
            let _ = tx.send(Msg::FixesReady(req, used, fixes));
        }));
        Ok(())
    }

    /// 用户逐条确认之后真正落盘。写之前再读一次文件：确认期间文件被改过就跳过，
    /// 不能用一份过期的 old 去覆盖别人的改动。
    fn apply_fixes(&mut self, fixes: Vec<Fix>) {
        let mut written = 0usize;
        let mut problems: Vec<String> = Vec::new();
        for fix in &fixes {
            let path = self.scan_root.join(&fix.file);
            match std::fs::read_to_string(&path) {
                Ok(current) if current != fix.old => {
                    problems.push(pick(
                        format!("{} 在确认期间被改过，已跳过", fix.file),
                        format!("{} changed while you were confirming, skipped", fix.file),
                    ));
                    continue;
                }
                Ok(_) => {}
                Err(e) => {
                    problems.push(pick(
                        format!("{} 读不出来：{e}", fix.file),
                        format!("{} cannot be read: {e}", fix.file),
                    ));
                    continue;
                }
            }
            match std::fs::write(&path, &fix.new) {
                Ok(()) => {
                    written += 1;
                    self.state.push(Entry::Success(pick(
                        format!("已修复 {}", fix.file),
                        format!("Fixed {}", fix.file),
                    )));
                }
                Err(e) => problems.push(pick(
                    format!("{} 写入失败：{e}", fix.file),
                    format!("{} write failed: {e}", fix.file),
                )),
            }
        }
        // 写过的文件里，剩下的发现也一并作废：我们自己的改动同样会让行号漂移，
        // 留着它们只会让下一次 /fix 改错地方
        let touched: Vec<&str> = fixes.iter().map(|f| f.file.as_str()).collect();
        let before = self.state.findings.len();
        self.state
            .findings
            .retain(|f| !touched.contains(&f.file.as_str()));
        let dropped = before - self.state.findings.len();
        if written > 0 {
            self.state.push(Entry::Success(pick(
                format!("共写入 {written} 个文件"),
                format!("wrote {written} files"),
            )));
        }
        if dropped > written {
            self.state.push(Entry::Info(pick(format!(
                "这些文件里另有 {} 条发现已作废（改动会让行号漂移），建议重新扫一次",
                dropped - written
            ), format!(
                "Another {} findings in these files are now void (edits shift line numbers); rescanning is recommended",
                dropped - written
            ))));
        }
        for p in problems {
            self.state.push(Entry::Warn(p));
        }
        // 存档也跟着更新，免得下次 /findings 又看见已经修过的条目
        if written > 0 {
            self.save_scan();
        }
    }

    /// `/scan [路径] [--ai]`
    ///
    /// 第一段（本地规则）永远跑，秒级、不花钱；加 `--ai` 才进第二段，
    /// 把本地规则判不了的条目分批交给模型逐条判断。两段都只读。
    fn start_scan(&mut self, args: &[&str]) -> Result<()> {
        if self.state.busy.is_some() {
            bail!(
                "{}",
                crate::i18n::t(
                    "正在忙，等这次结束或者按 Esc 取消",
                    "Busy — wait for this to finish, or press Esc to cancel"
                )
            );
        }
        let with_ai = args.contains(&"--ai");
        // 默认只判安全类分类；--all 才把工程规范类也交给模型
        let scan_all = args.contains(&"--all");
        let root = match args.iter().find(|a| !a.starts_with('-')) {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_dir()?,
        };
        if !root.is_dir() {
            bail!(
                "{}",
                pick(
                    format!("不是一个目录：{}", root.display()),
                    format!("not a directory: {}", root.display())
                )
            );
        }
        // 第二段要烧 token，先把客户端建好：配置不对就当场说，别等扫完了才报错
        let client = if with_ai {
            Some(LlmClient::new(&self.app.effective()).context(tr("无法创建模型客户端"))?)
        } else {
            None
        };

        self.scan_root = root.clone();
        self.scan_started = now_secs();
        self.scan_id = Some(crate::scan_store::new_id(
            &root,
            chrono::Local::now().timestamp_millis(),
        ));
        let req = self.next_req();
        let tx = self.tx.clone();
        let config_path = self.app.config_path.clone();
        self.state.push(Entry::Info(crate::i18n::pick(
            format!("正在扫描 {}（本地规则，只读）…", ui::tilde(&root)),
            format!("Scanning {} (local rules, read-only)…", ui::tilde(&root)),
        )));
        if with_ai && !scan_all {
            // 说清楚这一轮判什么、不判什么。少了这句，「覆盖 294/410」会让人
            // 以为是漏了，其实是故意没让模型去猜工程规范
            let list = crate::vuln_checklist::VulnChecklist::new();
            let covered = crate::scan::covered_ids();
            let skipped: usize = list
                .categories()
                .iter()
                .filter(|c| !crate::scan_ai::is_security_category(c))
                .map(|c| {
                    list.by_category(c)
                        .iter()
                        .filter(|i| !covered.contains(&i.id.as_str()))
                        .count()
                })
                .sum();
            self.state.skipped_items = skipped;
            self.state.push(Entry::Info(crate::i18n::pick(
                format!(
                    "AI 判定只跑安全类分类（A–M、P、R），跳过 N/O/Q/S 的 {skipped} 条工程规范条目；要全跑加 --all"
                ),
                format!(
                    "AI judgement covers security categories only (A–M, P, R); skipping {skipped} engineering-hygiene items. Use --all to include them."
                ),
            )));
        }
        self.state.busy = Some(if with_ai {
            crate::i18n::t("扫描中（含模型判定）…", "Scanning (with model judgement)…").into()
        } else {
            crate::i18n::t("扫描中…", "Scanning…").into()
        });

        self.task = Some(tokio::spawn(async move {
            let scan_root = root.clone();
            let report = tokio::task::spawn_blocking(move || {
                crate::scan::scan_local(&scan_root, &crate::vuln_checklist::VulnChecklist::new())
            })
            .await
            .unwrap_or_else(|_| crate::scan::LocalReport::empty());
            let _ = tx.send(Msg::ScanDone(req, report));

            let Some(client) = client else { return };
            // 建一次索引：文件树 + 所有能读的正文。之后每批按关键词从这里检索，
            // 而不是把同一份摘要发给所有批次
            let index_root = root.clone();
            let Ok(index) =
                tokio::task::spawn_blocking(move || crate::scan_ai::Index::build(&index_root))
                    .await
            else {
                let _ = tx.send(Msg::ScanProgress(
                    req,
                    tr("建立仓库索引失败，跳过模型判定").into(),
                ));
                return;
            };
            let snapshot = crate::scan_ai::ScanDigest {
                hash: index.hash(),
                files: index.file_hashes(),
            };

            // 上次的存档：决定这一轮是「什么都不用判」「只判变过的」还是「全量」
            let store = crate::scan_store::ScanStore::beside_config(&config_path);
            let prev = store.latest_for(&root).ok().flatten();

            // 只有「上次完整跑完」的存档才配复用。上次有批次失败说明覆盖不全，
            // 直接复用等于把缺口当成结论。
            if let Some(prev) = &prev
                && prev.digest_hash != 0
                && prev.digest_hash == snapshot.hash
                && !prev.judgements.is_empty()
                && prev.failed_batches.is_empty()
            {
                let _ = tx.send(Msg::ScanProgress(
                    req,
                    crate::i18n::pick(
                        format!(
                            "文件一个都没变：直接复用上次的 {} 条判定，这一轮不发任何请求",
                            prev.judgements.len()
                        ),
                        format!(
                            "Nothing changed: reusing the {} verdicts from last time, no requests sent",
                            prev.judgements.len()
                        ),
                    ),
                ));
                let _ = tx.send(Msg::ScanAiDone(
                    req,
                    prev.judgements.clone(),
                    prev.failed_batches.clone(),
                    snapshot,
                ));
                return;
            }

            // 变过的文件（新增、内容变了、被删掉）
            let mut changed: Vec<String> = Vec::new();
            let mut incremental = false;
            if let Some(prev) = &prev
                && !prev.digest_files.is_empty()
            {
                changed.extend(
                    snapshot
                        .files
                        .iter()
                        .filter(|(path, hash)| prev.digest_files.get(*path) != Some(*hash))
                        .map(|(path, _)| path.clone()),
                );
                changed.extend(
                    prev.digest_files
                        .keys()
                        .filter(|path| !snapshot.files.contains_key(*path))
                        .cloned(),
                );
                changed.sort();
                changed.dedup();
                incremental = !changed.is_empty();
            }

            // 增量时检索范围收窄到变过的文件，文件树仍然给全
            let index = if incremental {
                index.scoped(&changed)
            } else {
                index
            };

            if incremental {
                let _ = tx.send(Msg::ScanProgress(
                    req,
                    pick(format!(
                        "有 {} 个文件变过，只把它们重新交给模型（其余文件沿用上次结论）：{}",
                        changed.len(),
                        changed
                            .iter()
                            .take(3)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("、")
                    ), format!(
                        "{} files changed; only those go back to the model (the rest keep their previous verdicts): {}",
                        changed.len(),
                        changed
                            .iter()
                            .take(3)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("、")
                    )),
                ));
            }

            let checklist = crate::vuln_checklist::VulnChecklist::new();
            // 增量时摘要小，一批能多装几条，请求数也跟着降
            let size = if incremental {
                crate::scan_ai::BATCH_INCREMENTAL
            } else {
                crate::scan_ai::BATCH
            };
            let batches = crate::scan_ai::batches_with(
                &checklist,
                &crate::scan::covered_ids(),
                size,
                if scan_all {
                    &[]
                } else {
                    crate::scan_ai::SECURITY_CATEGORIES
                },
            );
            let progress_tx = tx.clone();
            let judged = crate::scan_ai::judge(
                std::sync::Arc::new(client),
                std::sync::Arc::new(index),
                batches,
                move |msg| {
                    let _ = progress_tx.send(Msg::ScanProgress(req, msg));
                },
            )
            .await;
            let judgements = judged.judgements;
            let failed = judged.failed;
            let tokens = judged.tokens;

            // 增量：这一轮只看了变过的文件，和上次的结论合起来才是完整的
            let judgements = match (&prev, incremental) {
                (Some(prev), true) => crate::scan_ai::merge(&prev.judgements, judgements, &changed),
                _ => judgements,
            };

            let _ = tx.send(Msg::ScanProgress(
                req,
                crate::i18n::pick(
                    format!("模型判定结束，共消耗 {tokens} tokens"),
                    format!("Judgement finished, {tokens} tokens used"),
                ),
            ));
            let _ = tx.send(Msg::ScanAiDone(req, judgements, failed, snapshot));
        }));
        Ok(())
    }

    /// `/sessions` 和 `/search` 共用：读一遍目录，把列表和正文开头交给面板，
    /// 之后过滤都在内存里做
    fn open_sessions_panel(&mut self, query: &str) -> Result<()> {
        let (entries, skipped) = self.store.catalog()?;
        if skipped > 0 {
            self.state.push(Entry::Warn(pick(
                format!(
                    "有 {skipped} 个对话文件无法读取，已跳过（{}）",
                    ui::tilde(self.store.dir())
                ),
                format!(
                    "{skipped} conversation files could not be read and were skipped ({})",
                    ui::tilde(self.store.dir())
                ),
            )));
        }
        let now = now_secs();
        let items: Vec<SessionItem> = entries
            .into_iter()
            .filter(|e| e.summary.id != self.conv.id)
            .map(|e| {
                let when = format_ago(e.summary.updated, now);
                SessionItem::from_catalog(e, when)
            })
            .collect();
        if items.is_empty() {
            self.state.push(Entry::Info(
                crate::i18n::t("还没有历史对话", "No history yet").into(),
            ));
            return Ok(());
        }
        self.state.open_sessions(items, query);
        Ok(())
    }

    /// 面板里删掉一个对话：先动磁盘，成功了再让面板把它从列表里去掉
    fn delete_session(&mut self, id: &str) {
        match self.store.delete(id) {
            Ok(()) => self.state.forget_session(id),
            Err(e) => self.state.sessions_notice(format!("{e:#}"), true),
        }
    }

    // ---- 斜杠命令 ----

    fn command(&mut self, parts: Vec<String>) {
        self.state
            .push(Entry::User(format!("/{}", parts.join(" "))));
        if let Err(e) = self.run_command(&parts) {
            self.state.push(Entry::Error(format!("{e:#}")));
        }
    }

    fn run_command(&mut self, parts: &[String]) -> Result<()> {
        let args: Vec<&str> = parts.iter().skip(1).map(String::as_str).collect();
        match parts.first().map(String::as_str).unwrap_or("") {
            "help" | "h" | "?" => {
                let look = Look::from_config(&self.app.effective());
                self.state.push(Entry::Block(help_lines(&look)));
            }
            "demo" => start_demo(&mut self.state),
            "clear" | "new" => self.new_conversation(),
            "resume" => self.resume_latest()?,
            "scan" => self.start_scan(&args)?,
            "fix" => self.start_fix(&args)?,
            "report" => self.write_report(&args)?,
            "findings" => self.load_findings()?,
            "sessions" | "history" => self.open_sessions_panel("")?,
            // 关键词直接带进面板的搜索框，进去还能继续改
            "search" => self.open_sessions_panel(&args.join(" "))?,
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
                    _ => bail!(
                        "{}",
                        tr("用法：/model 打开配置表单，或 /model <模型名> 直接换模型")
                    ),
                }
            }
            "thinking" => match args.as_slice() {
                [] => self.state.push(Entry::Info(pick(
                    format!(
                        "当前思考模式：{}（/model 可以连同模型一起保存）",
                        self.app.effective().thinking
                    ),
                    format!(
                        "Current thinking mode: {} (/model saves it along with the model)",
                        self.app.effective().thinking
                    ),
                ))),
                [mode] => {
                    let mode: Thinking = mode.parse().map_err(anyhow::Error::msg)?;
                    self.app.overrides.thinking = Some(mode);
                    self.state.push(Entry::Success(pick(
                        format!("本次会话思考模式：{mode}"),
                        format!("Thinking mode for this session: {mode}"),
                    )));
                }
                _ => bail!("{}", tr("用法：/thinking [auto|off|low|high|max]")),
            },
            "mode" => match args.as_slice() {
                [] => {
                    let current = self.state.mode;
                    self.state.open_mode_picker(current);
                }
                [name] => match Mode::parse(name) {
                    Some(mode) => self.state.set_mode(mode),
                    None => bail!("{}", tr("模式可选值为 plan / confirm / auto")),
                },
                _ => bail!("{}", tr("用法：/mode [plan|confirm|auto]")),
            },
            "lang" => self.set_lang(&args)?,
            "accent" => self.set_accent_cmd(&args)?,
            "trust" => self.trust_now()?,
            "theme" => match args.as_slice() {
                [] => {
                    let current = Theme::parse(&self.app.effective().theme).unwrap_or_default();
                    self.state.open_theme_picker(current);
                }
                [name] => match Theme::parse(name) {
                    Some(theme) => self.set_theme(theme),
                    None => bail!("{}", tr("主题可选值为 dark / light")),
                },
                _ => bail!("{}", tr("用法：/theme [dark|light]")),
            },
            "font" => match args.as_slice() {
                [] => {
                    let current =
                        FontStyle::parse(&self.app.effective().font_style).unwrap_or_default();
                    self.state.open_font_picker(current);
                }
                [name] => match FontStyle::parse(name) {
                    Some(font) => self.set_font(font),
                    None => bail!("{}", tr("字形可选值为 default / anime")),
                },
                _ => bail!("{}", tr("用法：/font [default|anime]")),
            },
            "permissions" => match args.as_slice() {
                [] => {
                    let current = self.app.effective().permission_mode;
                    self.state.open_permission_picker(current);
                }
                [mode] => {
                    self.set_permission_mode(mode)?;
                }
                _ => bail!(
                    "{}",
                    tr("用法：/permissions [readonly|editable|full] [工作目录]")
                ),
            },
            "" => bail!("{}", tr("请输入命令，/help 查看可用命令")),
            other => bail!(
                "{}",
                pick(
                    format!("未知命令 /{other}，/help 查看可用命令"),
                    format!("Unknown command /{other}; /help lists the commands")
                )
            ),
        }
        Ok(())
    }

    fn config(&mut self, args: &[&str]) -> Result<()> {
        match args {
            [] | ["show"] => {
                let lines = self.config_lines();
                self.state.push(Entry::Block(lines));
            }
            ["init"]
            | ["set", "api_key"]
            | ["set", "base_url"]
            | ["set", "model"]
            | ["set", "thinking"] => self.open_model_form(),
            ["set", _] => bail!("{}", tr("请提供 value：/config set <key> <value>")),
            ["set", key, value @ ..] => {
                let value = value.join(" ");
                self.app.file.set(key, &value)?;
                self.app.save()?;
                let cfg = self.app.effective();
                self.state.push(Entry::Success(pick(
                    format!("已保存 {key} = {}", display_value(&self.app.file, key)),
                    format!("Saved {key} = {}", display_value(&self.app.file, key)),
                )));
                if let Some(name) = env_override(key) {
                    self.state.push(Entry::Warn(pick(
                        format!("环境变量 {name} 已设置，它会覆盖配置文件中的 {key}"),
                        format!("Env var {name} is set and overrides {key} in the config"),
                    )));
                }
                for w in cfg.warnings() {
                    self.state.push(Entry::Warn(w));
                }
            }
            ["unset", key] => {
                self.app.file.unset(key)?;
                self.app.save()?;
                self.state.push(Entry::Success(pick(
                    format!("{key} 已恢复默认值：{}", display_value(&self.app.file, key)),
                    format!(
                        "{key} restored to its default: {}",
                        display_value(&self.app.file, key)
                    ),
                )));
            }
            _ => bail!(
                "{}",
                tr("用法：/config [show|set <key> <value>|unset <key>]")
            ),
        }
        Ok(())
    }

    fn config_lines(&self) -> Vec<Line<'static>> {
        let dim = Style::new().add_modifier(Modifier::DIM);
        let cfg = self.app.effective();
        let mut out = vec![Line::from(vec![
            Span::styled(tr("配置文件 "), Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(ui::tilde(&self.app.config_path)),
            Span::styled(
                if self.app.config_path.exists() {
                    ""
                } else {
                    tr("（尚未创建，使用默认值）")
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
            Span::styled(tr("  对话目录      "), dim),
            Span::raw(ui::tilde(self.store.dir())),
        ]));
        out
    }
    /// `/lang`：不带参数打开选择菜单，带参数直接切
    fn set_lang(&mut self, args: &[&str]) -> Result<()> {
        let Some(raw) = args.first() else {
            self.state.open_lang_picker();
            return Ok(());
        };

        let Some(lang) = crate::i18n::Lang::parse(raw) else {
            bail!(
                "{}",
                pick(
                    format!("lang 可选值为 en / zh，收到 `{raw}`"),
                    format!("lang must be en / zh, got `{raw}`")
                )
            );
        };
        self.apply_lang(lang);
        Ok(())
    }

    /// 没信任过这个目录就弹一次浮层。已经信任的直接跳过，不打扰
    fn maybe_ask_trust(&mut self) {
        let root = std::env::current_dir().unwrap_or_default();
        if self.app.effective().is_trusted(&root) {
            return;
        }
        self.state.open_trust_picker(ui::tilde(&root));
    }

    /// 信任浮层的答复。
    ///
    /// 「是」写进配置，以后不再问；「否」只改内存里的权限模式，配置文件一个字不动——
    /// 下次启动还会问一遍，用户改主意不用去翻配置文件。
    fn answer_trust(&mut self, yes: bool) {
        let root = std::env::current_dir().unwrap_or_default();
        let shown = ui::tilde(&root);
        if yes {
            if self.app.file.trust_workspace(&root).is_none() {
                self.state.push(Entry::Error(pick(
                    format!("无法解析这个路径：{shown}"),
                    format!("Cannot resolve this path: {shown}"),
                )));
                return;
            }
            if let Err(e) = self.app.save() {
                self.state.push(Entry::Error(pick(
                    format!("保存失败，配置文件没有改动：{e:#}"),
                    format!("Save failed, the config file is unchanged: {e:#}"),
                )));
                return;
            }
            self.state.push(Entry::Success(pick(
                format!("已信任 {shown}：可以执行命令、修改文件"),
                format!("Trusted {shown}: commands and edits are allowed"),
            )));
        } else {
            self.app.file.permission_mode = crate::tools::PermissionMode::ReadOnly;
            self.state.push(Entry::Warn(pick(
                format!(
                    "未信任 {shown}：本次会话只读（不写文件、不跑命令）。改主意用 /trust"
                ),
                format!(
                    "Not trusted {shown}: this session is read-only. Use /trust if you change your mind"
                ),
            )));
        }
    }

    /// 换强调色。存进配置，下次启动照样生效
    fn set_accent(&mut self, accent: crate::theme::Accent) {
        self.app.file.accent = accent.label().to_string();
        if let Err(e) = self.app.save() {
            self.state.push(Entry::Error(pick(
                format!("保存失败，配置文件没有改动：{e:#}"),
                format!("Save failed, the config file is unchanged: {e:#}"),
            )));
            return;
        }
        self.state.push(Entry::Success(pick(
            format!("强调色：{}", accent.label()),
            format!("Accent: {}", accent.label()),
        )));
    }

    /// `/accent [名字]`：不带参数打开选择，带参数直接换
    fn set_accent_cmd(&mut self, args: &[&str]) -> Result<()> {
        let Some(raw) = args.first() else {
            let current =
                crate::theme::Accent::parse(&self.app.effective().accent).unwrap_or_default();
            self.state.open_accent_picker(current);
            return Ok(());
        };
        let Some(accent) = crate::theme::Accent::parse(raw) else {
            bail!(
                "{}",
                pick(
                    format!(
                        "accent 可选值为 teal / sakura / ocean / violet / amber / lime / mono，收到 `{raw}`"
                    ),
                    format!(
                        "accent must be one of teal / sakura / ocean / violet / amber / lime / mono; got `{raw}`"
                    )
                )
            );
        };
        self.set_accent(accent);
        Ok(())
    }

    /// `/trust`：把当前目录记进信任列表
    fn trust_now(&mut self) -> Result<()> {
        self.answer_trust(true);
        Ok(())
    }

    /// 语言选择里按了 Enter：存盘、立即生效、关掉浮层
    fn apply_lang(&mut self, lang: crate::i18n::Lang) {
        let saved = self
            .app
            .file
            .set("lang", lang.code())
            .and_then(|()| self.app.save());
        if let Err(e) = saved {
            self.state.push(Entry::Error(pick(
                format!("保存失败，配置文件没有改动：{e:#}"),
                format!("Save failed, the config file is unchanged: {e:#}"),
            )));
            return;
        }
        crate::i18n::set(lang);
        self.state.close_picker();
        self.state.push(Entry::Success(pick(
            format!("界面语言：{}", lang.label()),
            format!("Language: {}", lang.label()),
        )));
    }

    fn set_theme(&mut self, theme: Theme) {
        // 立刻生效靠 override，长期生效靠配置文件——两个都写，
        // 否则「在菜单里选了 dark，重开又变回去」会显得像个 bug
        self.app.overrides.theme = Some(theme.label().to_string());
        let _ = self.app.file.set("theme", theme.label());
        let _ = self.app.save();
        self.state.push(Entry::Success(pick(
            format!("界面主题：{}", theme.label()),
            format!("Theme: {}", theme.label()),
        )));
    }

    /// 换字形（框线形状 + 标记符号）
    fn set_font(&mut self, font: FontStyle) {
        self.app.overrides.font_style = Some(font.label().to_string());
        let _ = self.app.file.set("font_style", font.label());
        let _ = self.app.save();
        let hint = match font {
            FontStyle::Default => tr("朴素框线，圆点标记"),
            FontStyle::Anime => tr("圆角框 + 星星爱心"),
        };
        self.state.push(Entry::Success(pick(
            format!("字形：{}（{hint}）", font.label()),
            format!("Font: {} ({hint})", font.label()),
        )));
    }
    fn set_permission_mode(&mut self, mode: &str) -> Result<()> {
        self.app.file.set("permission_mode", mode)?;
        self.app.save()?;
        self.state.push(Entry::Success(pick(
            format!(
                "权限模式已保存：{}",
                display_value(&self.app.effective(), "permission_mode")
            ),
            format!(
                "Permission mode saved: {}",
                display_value(&self.app.effective(), "permission_mode")
            ),
        )));
        // 设了 full 却还在 plan 里，是最容易误解的一处：说明白它此刻不生效
        if self.state.mode == Mode::Plan
            && !matches!(
                self.app.effective().permission_mode,
                crate::tools::PermissionMode::ReadOnly
            )
        {
            self.state.push(Entry::Warn(
                tr("当前工作模式是 plan（强制只读），这个权限暂时不生效：\
                 按 Shift-Tab 切到 confirm 或 auto")
                .into(),
            ));
        }
        Ok(())
    }
}

/// 从工具参数里取出路径和改动前后的代码，供确认卡片和 diff 显示
/// 工具调用的中文名和参数摘要。用中文是因为这是给人看的；
/// 具体参数（路径、命令）保持原样，方便一眼认出在动哪个文件。
fn tool_label(name: &str, args_json: &str) -> (String, String) {
    let v: serde_json::Value = serde_json::from_str(args_json).unwrap_or(serde_json::Value::Null);
    let text = |key: &str| -> String {
        v.get(key)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    match name {
        "read_file" => (tr("读取").into(), text("path")),
        "list_dir" => {
            let path = text("path");
            (
                tr("列目录").into(),
                if path.is_empty() { ".".into() } else { path },
            )
        }
        "write_file" => (tr("写入").into(), text("path")),
        "edit_file" => (tr("修改").into(), text("path")),
        "run_command" => {
            let cmd = text("cmd");
            let rest: Vec<String> = v
                .get("args")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let line = if rest.is_empty() {
                cmd
            } else {
                format!("{cmd} {}", rest.join(" "))
            };
            (tr("运行").into(), line)
        }
        other => (other.to_string(), String::new()),
    }
}

fn write_preview(tool_name: &str, args_json: &str) -> (String, String, String) {
    let Ok(args) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(args_json)
    else {
        return (String::new(), String::new(), String::new());
    };
    let text = |key: &str| {
        args.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    // write_file 用 content，edit_file 用 new_str
    let old = if tool_name == "edit_file" {
        text("old_str")
    } else {
        String::new()
    };
    let new = if tool_name == "write_file" {
        text("content")
    } else {
        text("new_str")
    };
    (text("path"), old, new)
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
        row(tr("API 地址"), s.base_url.clone()),
        row(
            "API Key",
            s.api_key
                .as_deref()
                .map_or_else(|| tr("未设置").to_string(), mask_key),
        ),
        row(tr("模型"), s.model.clone()),
        row(tr("思考力度"), s.thinking.to_string()),
        Line::styled(
            pick(
                format!("  已写入 {path}，以后每次启动都会使用"),
                format!("Wrote {path}; it will be used on every start from now on"),
            ),
            dim,
        ),
    ]
}

fn help_lines(look: &Look) -> Vec<Line<'static>> {
    let name_w = COMMANDS.iter().map(|c| c.name.len()).max().unwrap_or(0) + 2;
    let dim = Style::new().add_modifier(Modifier::DIM);
    let mut out: Vec<Line<'static>> = COMMANDS
        .iter()
        .map(|c| {
            Line::from(vec![
                Span::styled(format!("  {:<name_w$}", c.name), look.accent()),
                Span::raw(c.desc()),
            ])
        })
        .collect();
    out.push(Line::raw(""));
    out.push(Line::styled(
        tr("  /model 表单：↑↓ 切换项 · 直接输入修改 · ←→ 调整思考力度 · Ctrl-S 保存 · Esc 取消"),
        dim,
    ));
    out.push(Line::styled(
        tr("  Ctrl-J 换行 · PgUp/PgDn 滚动 · Ctrl-B 到顶部 · Ctrl-G 到底部"),
        dim,
    ));
    out.push(Line::styled(
        tr("  Shift-Tab / Ctrl-P 切换模式：plan 只读出报告 · confirm 每改一处确认 · auto 全自动"),
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
            let at = line
                .find(value)
                .unwrap_or_else(|| panic!("{value} in {line:?}"));
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
        let text: Vec<String> = help_lines(&Look::default())
            .iter()
            .map(crate::chat::line_text)
            .collect();
        for c in COMMANDS {
            assert!(
                text.iter().any(|l| l.contains(c.name)),
                "missing {}",
                c.name
            );
        }
    }

    /// 帮助面板里的命令名以前写死成 Color::Cyan，浅色主题下几乎看不见
    #[test]
    fn help_paints_the_command_names_with_the_current_accent() {
        let look = |theme: &str| {
            let cfg = crate::config::Config {
                theme: theme.into(),
                ..Default::default()
            };
            Look::from_config(&cfg)
        };
        let name_colour = |look: &Look| {
            help_lines(look)
                .into_iter()
                .find(|l| crate::chat::line_text(l).contains("/help"))
                .and_then(|l| l.spans.into_iter().next())
                .and_then(|s| s.style.fg)
        };

        let (light, dark) = (look("light"), look("dark"));
        assert_eq!(name_colour(&light), Some(light.palette.accent));
        assert_eq!(name_colour(&dark), Some(dark.palette.accent));
        assert_ne!(
            name_colour(&light),
            name_colour(&dark),
            "浅色和深色主题的命令名颜色应该不同"
        );
    }

    #[test]
    fn tool_labels_say_what_is_about_to_be_touched() {
        let cases = [
            (
                "read_file",
                r#"{"path":"src/app.py"}"#,
                "读取",
                "src/app.py",
            ),
            (
                "write_file",
                r#"{"path":"a.txt","content":"x"}"#,
                "写入",
                "a.txt",
            ),
            (
                "edit_file",
                r#"{"path":"b.rs","old_str":"a","new_str":"b"}"#,
                "修改",
                "b.rs",
            ),
            ("list_dir", r#"{}"#, "列目录", "."),
            (
                "run_command",
                r#"{"cmd":"cargo","args":["test","--quiet"]}"#,
                "运行",
                "cargo test --quiet",
            ),
        ];
        for (tool, args, want_name, want_detail) in cases {
            let (name, detail) = tool_label(tool, args);
            assert_eq!(name, want_name, "{tool}");
            assert_eq!(detail, want_detail, "{tool}");
        }
        // 参数是坏的也不能 panic：模型偶尔会吐半截 JSON
        let (name, detail) = tool_label("read_file", "{not json");
        assert_eq!(name, "读取");
        assert!(detail.is_empty());
    }
}
