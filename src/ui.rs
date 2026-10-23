//! 终端输出：Claude Code 风格的框线、状态符号、spinner 与流式渲染。

use crate::config::Config;
use crate::i18n::{pick, tr};
use crate::llm::{ChatResponse, StreamEvent};
use crate::theme::Theme;
use console::{Color, Term, measure_text_width, style, truncate_str};
use indicatif::{ProgressBar, ProgressStyle};
use std::fmt::Display;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

/// 非交互模式没有地方传 `Look`，主题放一个进程级开关就够了：
/// 一个进程只服务一次配置，`/theme` 改了立刻生效。
static LIGHT: AtomicU8 = AtomicU8::new(0);

pub fn set_theme(theme: Theme) {
    LIGHT.store(u8::from(theme == Theme::Light), Ordering::Relaxed);
}

fn light() -> bool {
    LIGHT.load(Ordering::Relaxed) == 1
}

/// 主色。浅色底上青色看不清，换成深蓝
fn accent() -> Color {
    if light() { Color::Blue } else { Color::Cyan }
}

/// 警告色。`Yellow` 压在浅色底上几乎看不见，换成洋红
fn warning() -> Color {
    if light() {
        Color::Magenta
    } else {
        Color::Yellow
    }
}

pub fn term_width() -> usize {
    let (_, cols) = Term::stdout().size();
    (cols as usize).clamp(40, 100)
}

/// 把主目录替换为 `~`
pub fn tilde(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return Path::new("~").join(rest).display().to_string();
    }
    path.display().to_string()
}

/// 圆角框，按显示宽度对齐（兼容中文与 ANSI 颜色）
pub fn boxed(lines: &[String]) {
    let max_inner = term_width() - 4;
    let inner = lines
        .iter()
        .map(|l| measure_text_width(l))
        .max()
        .unwrap_or(0)
        .min(max_inner);
    let edge = |s: String| style(s).fg(accent()).to_string();
    println!("{}", edge(format!("╭{}╮", "─".repeat(inner + 2))));
    for line in lines {
        let line = truncate_str(line, inner, "…");
        let pad = inner.saturating_sub(measure_text_width(&line));
        println!(
            "{} {}{} {}",
            edge("│".into()),
            line,
            " ".repeat(pad),
            edge("│".into())
        );
    }
    println!("{}", edge(format!("╰{}╯", "─".repeat(inner + 2))));
}

pub fn banner(cfg: &Config) {
    let cwd = std::env::current_dir()
        .map(|p| tilde(&p))
        .unwrap_or_default();
    boxed(&[
        format!(
            "{} {}",
            style("✻").fg(accent()).bold(),
            style(format!("VibeGuard v{}", env!("CARGO_PKG_VERSION"))).bold()
        ),
        style(crate::i18n::t(
            "AI 生成代码的安全扫描与修复",
            "Security review for AI-written code",
        ))
        .dim()
        .to_string(),
        String::new(),
        format!(
            "{} {} · thinking {}",
            style("model").dim(),
            cfg.model,
            cfg.thinking
        ),
        format!("{} {}", style("cwd  ").dim(), cwd),
    ]);
}

pub fn info(msg: &str) {
    println!("{} {msg}", style("●").fg(accent()));
}

pub fn success(msg: &str) {
    println!("{} {msg}", style("✓").green().bold());
}

pub fn warn(msg: &str) {
    eprintln!(
        "{} {}",
        style("!").fg(warning()).bold(),
        style(msg).fg(warning())
    );
}

pub fn error(msg: &str) {
    eprintln!("{} {}", style("✗").red().bold(), style(msg).red());
}

pub fn hint(msg: &str) {
    println!("{}", style(msg).dim());
}

pub fn kv(key: &str, value: impl Display) {
    println!("  {} {value}", style(format!("{key:<13}")).dim());
}

pub fn spinner(msg: impl Into<String>) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg:.dim} {elapsed:.dim}")
            .expect("valid template")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "✓"]),
    );
    pb.set_message(msg.into());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    Reasoning,
    Content,
}

/// 把 LLM 流式事件渲染到终端：等待时显示 spinner，思考过程灰色显示，正文正常显示
pub struct StreamPrinter {
    spinner: Option<ProgressBar>,
    phase: Phase,
    show_reasoning: bool,
}

impl StreamPrinter {
    pub fn new(msg: impl Into<String>, show_reasoning: bool) -> Self {
        Self {
            spinner: Some(spinner(msg)),
            phase: Phase::Waiting,
            show_reasoning,
        }
    }

    pub fn on_event(&mut self, ev: StreamEvent<'_>) {
        match ev {
            StreamEvent::Retrying {
                attempt,
                delay,
                reason,
            } => {
                let line = pick(
                    format!(
                        "请求失败，{:.1}s 后第 {attempt} 次重试：{reason}",
                        delay.as_secs_f64()
                    ),
                    format!(
                        "Request failed, retrying in {:.1}s (attempt {attempt}): {reason}",
                        delay.as_secs_f64()
                    ),
                );
                match &self.spinner {
                    Some(pb) => pb.suspend(|| warn(&line)),
                    None => warn(&line),
                }
            }
            StreamEvent::Reasoning(text) => {
                if !self.show_reasoning {
                    if let Some(pb) = &self.spinner {
                        pb.set_message(tr("思考中…"));
                    }
                    return;
                }
                self.enter(Phase::Reasoning);
                print!("{}", style(text).dim());
                let _ = io::stdout().flush();
            }
            StreamEvent::Content(text) => {
                self.enter(Phase::Content);
                print!("{text}");
                let _ = io::stdout().flush();
            }
        }
    }

    fn enter(&mut self, next: Phase) {
        if self.phase == next {
            return;
        }
        if let Some(pb) = self.spinner.take() {
            pb.finish_and_clear();
        }
        match (self.phase, next) {
            (_, Phase::Reasoning) => println!("{}", style(tr("✻ 思考过程")).dim().italic()),
            (Phase::Reasoning, Phase::Content) => println!("\n"),
            _ => {}
        }
        self.phase = next;
    }

    /// 结束渲染（成功或失败都应调用）
    pub fn finish(&mut self) {
        if let Some(pb) = self.spinner.take() {
            pb.finish_and_clear();
        }
        if self.phase != Phase::Waiting {
            println!();
        }
    }
}

/// 一行灰色的模型 / token / 耗时统计
pub fn usage_text(res: &ChatResponse, elapsed: Duration) -> String {
    let mut parts = Vec::new();
    if let Some(model) = &res.model {
        parts.push(model.clone());
    }
    if let Some(u) = &res.usage {
        let mut input = pick(
            format!("输入 {}", u.prompt_tokens),
            format!("Input {}", u.prompt_tokens),
        );
        if let Some(hit) = u.prompt_cache_hit_tokens.filter(|&n| n > 0) {
            input.push_str(&pick(
                format!("（缓存命中 {hit}）"),
                format!("(cache hit {hit})"),
            ));
        }
        parts.push(input);
        let mut output = pick(
            format!("输出 {}", u.completion_tokens),
            format!("Output {}", u.completion_tokens),
        );
        if let Some(r) = u.reasoning_tokens().filter(|&n| n > 0) {
            output.push_str(&pick(format!("（思考 {r}）"), format!("(thinking {r})")));
        }
        parts.push(output);
    }
    parts.push(format!("{:.1}s", elapsed.as_secs_f64()));
    parts.join(" · ")
}

/// 打印一行灰色的统计
pub fn usage_line(res: &ChatResponse, elapsed: Duration) {
    println!("{}", style(usage_text(res, elapsed)).dim());
}
