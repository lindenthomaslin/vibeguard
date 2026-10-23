//! 交互模式：直接运行 `vibeguard` 进入。普通输入与 AI 对话，`/` 开头为命令。

use crate::app::App;
use crate::commands;
use crate::editor::{Candidate, Editor};
use crate::i18n::{pick, tr};
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

pub const SYSTEM_PROMPT_EN: &str = "You are VibeGuard, a code-audit assistant focused on \
application security. The user's code is mostly written by AI (vibe coding); the recurring \
problems are hardcoded secrets, injection, missing authorization and unsafe defaults. Be \
concrete and actionable: point at the code, explain the risk, and give the fixed code. Reply in \
the language the user writes in; if their message is language-neutral, reply in English.";

/// 系统提示词：跟着界面语言走
pub fn system_prompt() -> &'static str {
    crate::i18n::t(SYSTEM_PROMPT, SYSTEM_PROMPT_EN)
}

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
        ui::warn(tr(
            "尚未配置 API Key：输入 /config init，或设置环境变量 DEEPSEEK_API_KEY",
        ));
    }
    for w in cfg.warnings() {
        ui::warn(&w);
    }
    ui::hint(tr("直接输入问题与 AI 对话，/help 查看命令"));

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
                ui::hint(tr("再按一次 Ctrl-C 退出"));
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
        let system = format!("{}{}", system_prompt(), mode.instruction());
        messages.push(Message::system(&system));
        messages.extend(history.iter().cloned());
        messages.push(Message::user(input));

        let mut printer = ui::StreamPrinter::new(tr("等待响应…"), true);
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
                    ui::warn(tr("回答达到 max_tokens 上限被截断"));
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
            ui::success(tr("已清空对话历史"));
        }
        "config" => match args {
            [] | ["show"] => commands::show_config(app),
            ["init"] => commands::wizard(app).await?,
            ["set", key] => commands::config_set(app, key, None)?,
            ["set", key, value @ ..] => commands::config_set(app, key, Some(value.join(" ")))?,
            ["unset", key] => commands::config_unset(app, key)?,
            _ => bail!(
                "{}",
                tr("用法：/config [show|init|set <key> [value]|unset <key>]")
            ),
        },
        "model" => match args {
            [] => ui::info(&pick(
                format!("当前模型：{}", app.effective().model),
                format!("Current model: {}", app.effective().model),
            )),
            [model] => {
                app.overrides.model = Some(model.to_string());
                ui::success(&pick(
                    format!("本次会话改用模型 {model}（持久保存请用 /config set model）"),
                    format!(
                        "Switched this session to model {model} (use /config set model to persist)"
                    ),
                ));
                for w in app.effective().warnings() {
                    ui::warn(&w);
                }
            }
            _ => bail!("{}", tr("用法：/model [name]")),
        },
        "mode" => match args {
            [] => ui::info(&pick(
                format!("当前模式：「{}」{}", mode.title(), mode.describe()),
                format!("Current mode: \"{}\"{}", mode.title(), mode.describe()),
            )),
            [name] => match Mode::parse(name) {
                Some(next) => {
                    *mode = next;
                    ui::success(&pick(
                        format!("已切换到「{}」模式：{}", mode.title(), mode.describe()),
                        format!("Switched to \"{}\" mode: {}", mode.title(), mode.describe()),
                    ));
                }
                None => bail!("{}", tr("模式可选值为 plan / confirm / auto")),
            },
            _ => bail!("{}", tr("用法：/mode [plan|confirm|auto]")),
        },
        "thinking" => match args {
            [] => ui::info(&pick(
                format!("当前思考模式：{}", app.effective().thinking),
                format!("Current thinking mode: {}", app.effective().thinking),
            )),
            [mode] => {
                let mode = mode.parse().map_err(anyhow::Error::msg)?;
                app.overrides.thinking = Some(mode);
                ui::success(&pick(
                    format!("本次会话思考模式：{mode}"),
                    format!("Thinking mode for this session: {mode}"),
                ));
            }
            _ => bail!("{}", tr("用法：/thinking [auto|off|low|high|max]")),
        },
        "theme" => match args {
            [] => ui::info(&pick(
                format!("当前主题：{}", app.effective().theme),
                format!("Current theme: {}", app.effective().theme),
            )),
            [theme] => {
                let theme = theme.to_lowercase();
                if matches!(theme.as_str(), "dark" | "light") {
                    app.overrides.theme = Some(theme.clone());
                    ui::set_theme(crate::theme::Theme::parse(&theme).unwrap_or_default());
                    ui::success(&pick(
                        format!("界面主题已切换为：{theme}"),
                        format!("Theme switched to: {theme}"),
                    ));
                } else {
                    bail!("{}", tr("主题可选值为 dark / light"));
                }
            }
            _ => bail!("{}", tr("用法：/theme [dark|light]")),
        },
        "font" => match args {
            [] => ui::info(&pick(
                format!("当前字体风格：{}", app.effective().font_style),
                format!("Current font style: {}", app.effective().font_style),
            )),
            [style] => {
                let style = style.to_lowercase();
                if matches!(style.as_str(), "default" | "anime") {
                    app.overrides.font_style = Some(style.clone());
                    ui::success(&pick(
                        format!("字体风格已切换为：{style}"),
                        format!("Font style switched to: {style}"),
                    ));
                } else {
                    bail!("{}", tr("字体风格可选值为 default / anime"));
                }
            }
            _ => bail!("{}", tr("用法：/font [default|anime]")),
        },
        "ping" => commands::ping(app).await?,
        "" => bail!("{}", tr("请输入命令，/help 查看可用命令")),
        _ => bail!(
            "{}",
            pick(
                format!("未知命令 /{name}，/help 查看可用命令"),
                format!("Unknown command /{name}; /help lists the commands")
            )
        ),
    }
    Ok(())
}
