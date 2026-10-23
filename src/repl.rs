//! 交互模式：直接运行 `vibeguard` 进入。普通输入与 AI 对话，`/` 开头为命令。

use crate::app::App;
use crate::commands;
use crate::editor::{Candidate, Editor};
use crate::input::Outcome;
use crate::llm::{ChatOptions, LlmClient, Message};
use crate::mode::Mode;
use crate::prompt;
use crate::ui;
use anyhow::{Result, bail};
use console::style;
use std::time::Instant;

/// 输入 `/` 时弹出的命令菜单
const COMMANDS: &[Candidate] = &[
    Candidate {
        name: "/help",
        desc: "显示帮助",
    },
    Candidate {
        name: "/config",
        desc: "查看或修改配置",
    },
    Candidate {
        name: "/model",
        desc: "查看或切换模型",
    },
    Candidate {
        name: "/mode",
        desc: "切换工作模式：计划 / 逐条确认 / 全自动",
    },
    Candidate {
        name: "/thinking",
        desc: "查看或切换思考模式",
    },
    Candidate {
        name: "/theme",
        desc: "切换主题（dark/light）",
    },
    Candidate {
        name: "/font",
        desc: "切换字体风格（default/anime）",
    },
    Candidate {
        name: "/ping",
        desc: "测试 API 连接",
    },
    Candidate {
        name: "/clear",
        desc: "清空对话历史",
    },
    Candidate {
        name: "/exit",
        desc: "退出",
    },
];

pub const SYSTEM_PROMPT: &str = "你是 VibeGuard，一名专注于应用安全的代码审计助手。\
用户的代码多由 AI 生成（vibe coding），常见问题包括硬编码密钥、注入、鉴权缺失、不安全的默认配置等。\
回答要具体、可操作：指出问题所在、说明风险、给出修复后的代码。默认使用用户提问的语言回答。";

const HELP: &[(&str, &str)] = &[
    ("/help", "显示帮助"),
    ("/config [show|init]", "查看配置 / 运行配置向导"),
    ("/config set <key> [value]", "修改并保存配置项"),
    ("/model [name]", "查看或切换本次会话的模型"),
    (
        "/mode [plan|confirm|auto]",
        "切换工作模式：计划 / 逐条确认 / 全自动",
    ),
    (
        "/thinking [mode]",
        "查看或切换思考模式：auto / off / low / high / max",
    ),
    ("/theme [dark|light]", "切换界面主题"),
    ("/font [default|anime]", "切换字体风格"),
    ("/ping", "测试 API 连接"),
    ("/clear", "清空对话历史"),
    ("/exit", "退出（也可按 Ctrl-D）"),
];

pub async fn run(app: &mut App) -> Result<()> {
    let cfg = app.effective();
    ui::set_theme(crate::theme::Theme::parse(&cfg.theme).unwrap_or_default());
    ui::banner(&cfg);
    if cfg.api_key.is_none() {
        ui::warn("尚未配置 API Key：输入 /config init，或设置环境变量 DEEPSEEK_API_KEY");
    }
    for w in cfg.warnings() {
        ui::warn(&w);
    }
    ui::hint("直接输入问题与 AI 对话，/help 查看命令");

    let mut history: Vec<Message> = Vec::new();
    let mut client: Option<LlmClient> = None;
    let mut editor = Editor::new(COMMANDS);
    let mut interrupted = false;
    let mut mode = Mode::default();

    loop {
        println!();
        let input = match prompt::read_input(&mut editor)? {
            Outcome::Submit(text) => text,
            Outcome::Eof => break,
            Outcome::Interrupt => {
                if interrupted {
                    break;
                }
                interrupted = true;
                ui::hint("再按一次 Ctrl-C 退出");
                continue;
            }
            Outcome::Continue => continue,
        };
        interrupted = false;
        let input = input.as_str();

        if let Some(cmd) = input.strip_prefix('/') {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            let name = parts.first().copied().unwrap_or("");
            if matches!(name, "exit" | "quit" | "q") {
                break;
            }
            // 配置可能被修改，下次对话时重建客户端
            if matches!(name, "config" | "model" | "thinking") {
                client = None;
            }
            if let Err(e) = slash_command(
                app,
                name,
                &parts[1.min(parts.len())..],
                &mut history,
                &mut mode,
            )
            .await
            {
                ui::error(&format!("{e:#}"));
            }
            continue;
        }

        if client.is_none() {
            match LlmClient::new(&app.effective()) {
                Ok(c) => client = Some(c),
                Err(e) => {
                    ui::error(&format!("{e:#}"));
                    continue;
                }
            }
        }
        let Some(c) = client.as_ref() else { continue };

        let mut messages = Vec::with_capacity(history.len() + 2);
        let system = format!("{SYSTEM_PROMPT}{}", mode.instruction());
        messages.push(Message::system(&system));
        messages.extend(history.iter().cloned());
        messages.push(Message::user(input));

        let mut printer = ui::StreamPrinter::new("等待响应…", true);
        let start = Instant::now();
        let result = c
            .chat_stream(&messages, &ChatOptions::default(), |ev| {
                printer.on_event(ev)
            })
            .await;
        printer.finish();
        match result {
            Ok(res) => {
                if res.is_truncated() {
                    ui::warn("回答达到 max_tokens 上限被截断");
                }
                ui::usage_line(&res, start.elapsed());
                history.push(Message::user(input));
                history.push(Message::assistant(res.content));
            }
            Err(e) => ui::error(&format!("{e:#}")),
        }
    }
    Ok(())
}

async fn slash_command(
    app: &mut App,
    name: &str,
    args: &[&str],
    history: &mut Vec<Message>,
    mode: &mut Mode,
) -> Result<()> {
    match name {
        "help" | "h" | "?" => {
            for (cmd, desc) in HELP {
                println!("  {} {}", style(format!("{cmd:<28}")).cyan(), desc);
            }
        }
        "clear" => {
            history.clear();
            ui::success("已清空对话历史");
        }
        "config" => match args {
            [] | ["show"] => commands::show_config(app),
            ["init"] => commands::wizard(app).await?,
            ["set", key] => commands::config_set(app, key, None)?,
            ["set", key, value @ ..] => commands::config_set(app, key, Some(value.join(" ")))?,
            ["unset", key] => commands::config_unset(app, key)?,
            _ => bail!("用法：/config [show|init|set <key> [value]|unset <key>]"),
        },
        "model" => match args {
            [] => ui::info(&format!("当前模型：{}", app.effective().model)),
            [model] => {
                app.overrides.model = Some(model.to_string());
                ui::success(&format!(
                    "本次会话改用模型 {model}（持久保存请用 /config set model）"
                ));
                for w in app.effective().warnings() {
                    ui::warn(&w);
                }
            }
            _ => bail!("用法：/model [name]"),
        },
        "mode" => match args {
            [] => ui::info(&format!(
                "当前模式：「{}」{}",
                mode.title(),
                mode.describe()
            )),
            [name] => match Mode::parse(name) {
                Some(next) => {
                    *mode = next;
                    ui::success(&format!(
                        "已切换到「{}」模式：{}",
                        mode.title(),
                        mode.describe()
                    ));
                }
                None => bail!("模式可选值为 plan / confirm / auto"),
            },
            _ => bail!("用法：/mode [plan|confirm|auto]"),
        },
        "thinking" => match args {
            [] => ui::info(&format!("当前思考模式：{}", app.effective().thinking)),
            [mode] => {
                let mode = mode.parse().map_err(anyhow::Error::msg)?;
                app.overrides.thinking = Some(mode);
                ui::success(&format!("本次会话思考模式：{mode}"));
            }
            _ => bail!("用法：/thinking [auto|off|low|high|max]"),
        },
        "theme" => match args {
            [] => ui::info(&format!("当前主题：{}", app.effective().theme)),
            [theme] => {
                let theme = theme.to_lowercase();
                if matches!(theme.as_str(), "dark" | "light") {
                    app.overrides.theme = Some(theme.clone());
                    ui::set_theme(crate::theme::Theme::parse(&theme).unwrap_or_default());
                    ui::success(&format!("界面主题已切换为：{theme}"));
                } else {
                    bail!("主题可选值为 dark / light");
                }
            }
            _ => bail!("用法：/theme [dark|light]"),
        },
        "font" => match args {
            [] => ui::info(&format!("当前字体风格：{}", app.effective().font_style)),
            [style] => {
                let style = style.to_lowercase();
                if matches!(style.as_str(), "default" | "anime") {
                    app.overrides.font_style = Some(style.clone());
                    ui::success(&format!("字体风格已切换为：{style}"));
                } else {
                    bail!("字体风格可选值为 default / anime");
                }
            }
            _ => bail!("用法：/font [default|anime]"),
        },
        "ping" => commands::ping(app).await?,
        "" => bail!("请输入命令，/help 查看可用命令"),
        _ => bail!("未知命令 /{name}，/help 查看可用命令"),
    }
    Ok(())
}
