mod app;
mod chat;
mod commands;
mod config;
mod editor;
mod finding;
mod graphics;
mod icon;
mod input;
mod llm;
mod mode;
mod prompt;
mod repl;
mod review;
mod scan;
mod scan_ai;
mod scan_ui;
mod sessions;
mod theme;
mod tokens;
mod tools;
mod tui;
mod ui;
mod vuln_checklist;

use anyhow::Result;
use app::{App, Overrides};
use clap::{Parser, Subcommand};
use config::Thinking;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "vibeguard",
    version,
    about = "扫描并修复 AI 生成代码（vibe coding）中的安全漏洞",
    long_about = "扫描并修复 AI 生成代码（vibe coding）中的安全漏洞。\n不带子命令运行时进入交互模式。"
)]
struct Cli {
    /// 配置文件路径（默认 ~/.config/vibeguard/config.toml）
    #[arg(long, global = true, env = "VIBEGUARD_CONFIG", value_name = "FILE")]
    config: Option<PathBuf>,

    /// 本次运行使用的模型，不写入配置
    #[arg(long, short, global = true, value_name = "NAME")]
    model: Option<String>,

    /// 本次运行的思考模式：auto / off / low / high / max
    #[arg(long, global = true, value_name = "MODE")]
    thinking: Option<Thinking>,

    /// 内部用：查询终端的图形能力，把结果写进这个文件后退出
    #[arg(long, hide = true, value_name = "FILE")]
    probe_graphics: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 查看或修改配置；不带子命令时进入配置向导
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// 测试 API 连接
    Ping,
    /// 用示例数据预览扫描与修复界面，不读取也不修改任何文件
    #[command(hide = true)]
    Demo,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// 显示当前生效的配置
    Show,
    /// 交互式配置向导
    Init,
    /// 打印配置文件路径
    Path,
    /// 读取单个配置项
    Get { key: String },
    /// 设置配置项；省略 value 时交互输入（设置 api_key 时推荐，避免写入 shell 历史）
    Set { key: String, value: Option<String> },
    /// 将配置项恢复为默认值
    Unset { key: String },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // 图形能力查询在子进程里跑：它可能卡在读 stdin 上，父进程要能直接把它杀掉
    if let Some(path) = &cli.probe_graphics {
        graphics::probe_child(path);
        return ExitCode::SUCCESS;
    }
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            ui::error(&format!("{e:#}"));
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    let path = match cli.config {
        Some(p) => p,
        None => config::default_path()?,
    };
    let overrides = Overrides {
        model: cli.model,
        thinking: cli.thinking,
        theme: None,
        font_style: None,
    };
    let mut app = App::load(path, overrides)?;

    match cli.command {
        // 交互式终端进入全屏界面；管道、重定向等非终端环境退化为按行读取
        None if console::user_attended() && console::user_attended_stderr() => {
            tui::run(&mut app).await
        }
        None => repl::run(&mut app).await,
        Some(Command::Ping) => commands::ping(&app).await,
        Some(Command::Demo) => review::run_demo(),
        Some(Command::Config { action }) => match action {
            None if console::user_attended() => commands::wizard(&mut app).await,
            None | Some(ConfigAction::Show) => {
                commands::show_config(&app);
                Ok(())
            }
            Some(ConfigAction::Init) => commands::wizard(&mut app).await,
            Some(ConfigAction::Path) => {
                println!("{}", app.config_path.display());
                Ok(())
            }
            Some(ConfigAction::Get { key }) => commands::config_get(&app, &key),
            Some(ConfigAction::Set { key, value }) => commands::config_set(&mut app, &key, value),
            Some(ConfigAction::Unset { key }) => commands::config_unset(&mut app, &key),
        },
    }
}
