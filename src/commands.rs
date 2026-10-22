//! `config` 与 `ping` 命令的实现，CLI 与 REPL 共用。

use crate::app::App;
use crate::config::{
    self, API_KEY_ENVS, BASE_URL_ENV, Config, KNOWN_MODELS, KeySource, MODEL_ENV, Thinking,
    mask_key,
};
use crate::llm::{ChatOptions, LlmClient, Message};
use crate::ui;
use anyhow::Result;
use console::style;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Password, Select};
use std::time::Instant;

fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

/// 当前生效、会覆盖配置文件中该项的环境变量
pub(crate) fn env_override(key: &str) -> Option<&'static str> {
    let names: &[&'static str] = match key {
        "api_key" => API_KEY_ENVS,
        "base_url" => &[BASE_URL_ENV],
        "model" => &[MODEL_ENV],
        _ => &[],
    };
    names
        .iter()
        .copied()
        .find(|name| config::std_env(name).is_some_and(|v| !v.trim().is_empty()))
}

/// 用于展示的值，api_key 会被遮蔽
pub(crate) fn display_value(cfg: &Config, key: &str) -> String {
    match key {
        "api_key" => cfg
            .api_key
            .as_deref()
            .map_or_else(|| "未设置".to_string(), mask_key),
        _ => cfg.get(key).unwrap_or_default(),
    }
}

pub(crate) fn source_label(app: &App, cfg: &Config, key: &str) -> Option<String> {
    match key {
        "api_key" => (cfg.key_source != KeySource::Unset).then(|| cfg.key_source.label()),
        "model" if app.overrides.model.is_some() => Some("本次运行覆盖".into()),
        "thinking" if app.overrides.thinking.is_some() => Some("本次运行覆盖".into()),
        _ => env_override(key).map(|name| format!("环境变量 {name}")),
    }
}

pub fn show_config(app: &App) {
    let cfg = app.effective();
    let note = if app.config_path.exists() {
        ""
    } else {
        "（尚未创建，使用默认值）"
    };
    println!(
        "{} {}{}",
        style("配置文件").bold(),
        ui::tilde(&app.config_path),
        style(note).dim()
    );
    for &key in config::KEYS {
        let value = display_value(&cfg, key);
        match source_label(app, &cfg, key) {
            Some(src) => ui::kv(key, format!("{value}  {}", style(format!("← {src}")).dim())),
            None => ui::kv(key, value),
        }
    }
    for w in cfg.warnings() {
        ui::warn(&w);
    }
}

pub fn config_get(app: &App, key: &str) -> Result<()> {
    let cfg = app.effective();
    cfg.get(key)?; // 校验 key
    println!("{}", display_value(&cfg, key));
    Ok(())
}

pub fn config_set(app: &mut App, key: &str, value: Option<String>) -> Result<()> {
    let current = app.file.get(key)?; // 同时校验 key
    let value = match value {
        Some(v) => {
            if key == "api_key" {
                ui::hint(
                    "提示：命令行里的 API Key 可能被记录到 shell 历史，省略 value 可改为交互输入",
                );
            }
            v
        }
        None if key == "api_key" => Password::with_theme(&theme())
            .with_prompt("API Key")
            .interact()?,
        None => Input::<String>::with_theme(&theme())
            .with_prompt(key)
            .default(current)
            .interact_text()?,
    };
    app.file.set(key, &value)?;
    app.save()?;
    ui::success(&format!("已保存 {key} = {}", display_value(&app.file, key)));
    if let Some(name) = env_override(key) {
        ui::warn(&format!(
            "环境变量 {name} 已设置，它会覆盖配置文件中的 {key}"
        ));
    }
    for w in app.file.warnings() {
        ui::warn(&w);
    }
    Ok(())
}

pub fn config_unset(app: &mut App, key: &str) -> Result<()> {
    app.file.unset(key)?;
    app.save()?;
    ui::success(&format!(
        "{key} 已恢复默认值：{}",
        display_value(&app.file, key)
    ));
    Ok(())
}

/// 交互式配置向导
pub async fn wizard(app: &mut App) -> Result<()> {
    let theme = theme();
    ui::info(&format!(
        "配置 VibeGuard，保存到 {}",
        ui::tilde(&app.config_path)
    ));

    // API Key
    let env_key = env_override("api_key");
    let prompt = match (&app.file.api_key, env_key) {
        (Some(k), _) => format!("API Key（当前 {}，回车保留）", mask_key(k)),
        (None, Some(name)) => format!("API Key（当前使用环境变量 {name}，回车跳过）"),
        (None, None) => "API Key（在 platform.deepseek.com 获取）".to_string(),
    };
    let key = Password::with_theme(&theme)
        .with_prompt(prompt)
        .allow_empty_password(true)
        .interact()?;
    if !key.trim().is_empty() {
        app.file.set("api_key", &key)?;
    }

    // 模型
    let mut models: Vec<String> = KNOWN_MODELS.iter().map(|m| m.to_string()).collect();
    if !models.contains(&app.file.model) {
        models.push(app.file.model.clone());
    }
    let custom = models.len();
    let mut items = models.clone();
    items.push("自定义…".to_string());
    let default = models
        .iter()
        .position(|m| *m == app.file.model)
        .unwrap_or(0);
    let idx = Select::with_theme(&theme)
        .with_prompt("模型")
        .items(&items)
        .default(default)
        .interact()?;
    let model = if idx == custom {
        Input::<String>::with_theme(&theme)
            .with_prompt("模型名称")
            .interact_text()?
    } else {
        models[idx].clone()
    };
    app.file.set("model", &model)?;

    // 思考模式
    let labels: Vec<String> = Thinking::ALL
        .iter()
        .map(|t| format!("{:<5} {}", t.as_str(), t.describe()))
        .collect();
    let default = Thinking::ALL
        .iter()
        .position(|t| *t == app.file.thinking)
        .unwrap_or(0);
    let idx = Select::with_theme(&theme)
        .with_prompt("思考模式")
        .items(&labels)
        .default(default)
        .interact()?;
    app.file.thinking = Thinking::ALL[idx];

    // API 地址
    let url: String = Input::with_theme(&theme)
        .with_prompt("API 地址")
        .default(app.file.base_url.clone())
        .validate_with(|v: &String| -> Result<(), String> {
            let mut probe = Config::default();
            probe.set("base_url", v).map_err(|e| e.to_string())
        })
        .interact_text()?;
    app.file.set("base_url", &url)?;

    app.save()?;
    ui::success(&format!("配置已保存到 {}", ui::tilde(&app.config_path)));
    if let Some(name) = env_key {
        ui::warn(&format!(
            "环境变量 {name} 已设置，它会覆盖配置文件中的 api_key"
        ));
    }
    for w in app.file.warnings() {
        ui::warn(&w);
    }

    if app.effective().api_key.is_none() {
        ui::warn("尚未设置 API Key，扫描和修复功能将无法使用");
        return Ok(());
    }
    if Confirm::with_theme(&theme)
        .with_prompt("现在测试 API 连接？")
        .default(true)
        .interact()?
    {
        ping(app).await?;
    }
    Ok(())
}

/// 发送一个最小请求，验证 API Key、地址和模型是否可用
pub async fn ping(app: &App) -> Result<()> {
    let mut cfg = app.effective();
    let mut opts = ChatOptions::default();
    // 连通性测试不需要思考，除非用户显式指定了 --thinking
    if app.overrides.thinking.is_none() {
        cfg.thinking = Thinking::Off;
        opts.max_tokens = Some(16);
    }
    let client = LlmClient::new(&cfg)?;
    let messages = [
        Message::system("This is a connectivity check. Reply with the single word: pong"),
        Message::user("ping"),
    ];

    let mut printer = ui::StreamPrinter::new(
        format!("连接 {}（{}）", cfg.api_base(), client.model()),
        false,
    );
    let start = Instant::now();
    let result = client
        .chat_stream(&messages, &opts, |ev| printer.on_event(ev))
        .await;
    printer.finish();
    let res = result?;
    ui::success("API 连接正常");
    ui::usage_line(&res, start.elapsed());
    Ok(())
}
