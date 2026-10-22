//! 终端原生图片：能识别图形协议的终端（iTerm2 / Kitty / Sixel）直接显示 `icon.jpg`
//! 原图，而不是用半块字符拼出来的像素画。
//!
//! 能力靠一次 stdio 查询确定：向终端发一串控制序列，读它的回应。这个查询会**阻塞**
//! 读 stdin，直到终端回话为止；万一终端不理它，读线程就会一直卡在那里，把用户
//! 之后敲的键全吃掉。所以查询放在**子进程**里做：父进程等一小会儿，超时就把子进程
//! 杀掉，键盘始终归父进程。
//!
//! 终端不支持时 `Graphics` 是空的，调用方退回字标（`view::draw_splash`）。
//! 探测不准的时候可以用 `VIBEGUARD_GFX` 手动指定或关掉。

use crate::icon;
use ratatui_image::FontSize;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use std::io::IsTerminal;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 子进程查询最多等这么久。终端通常几毫秒就回，等太久会白白拖慢启动。
const PROBE_TIMEOUT: Duration = Duration::from_millis(300);
/// 子进程命令行开关，只在内部用
const PROBE_FLAG: &str = "--probe-graphics";
/// 手动指定协议：`off` 关掉原生图片，`iterm2` / `kitty` / `sixel` 强制使用。
/// tmux、screen 里探测常常问不出结果，这时可以自己指定。
const GFX_ENV: &str = "VIBEGUARD_GFX";
/// 强制协议时假设的字符尺寸（像素），一般终端都在这个量级
const ASSUMED_FONT: FontSize = FontSize { width: 10, height: 20 };

/// 进程内唯一的原生图片能力。没有可用协议时所有字段都是 `None`。
#[derive(Default)]
pub struct Graphics {
    /// 标题栏的小头像（圆形）
    avatar: Option<StatefulProtocol>,
    /// 启动画面的品牌图（方形）
    logo: Option<StatefulProtocol>,
}

impl Graphics {
    /// 查询终端能力并准备好图片协议。要在进入全屏之前调用：
    /// 子进程会临时切换终端模式，父进程这时还没接管终端。
    pub fn detect() -> Self {
        let found = match std::env::var(GFX_ENV).ok() {
            // 显式指定（包括关掉）时不去问终端，省一次子进程
            Some(v) => forced(&v),
            None => probe(),
        };
        let Some((protocol, font)) = found else {
            return Self::default();
        };
        let Some(face) = icon::logo_image() else {
            return Self::default();
        };
        let picker = picker(protocol, font);
        Self {
            avatar: icon::avatar_image()
                .map(|img| picker.new_resize_protocol(img.clone())),
            logo: Some(picker.new_resize_protocol(face.clone())),
        }
    }

    /// 一次借出两张图。两张图要同时画在同一帧上，分开借会撞借用检查。
    pub fn split(
        &mut self,
    ) -> (
        Option<&mut StatefulProtocol>,
        Option<&mut StatefulProtocol>,
    ) {
        (self.avatar.as_mut(), self.logo.as_mut())
    }
}

/// 用查询到的协议和字号搭一个 `Picker`。
#[allow(deprecated)] // 只有 `from_fontsize` 能在拿到真实字号后直接构造
fn picker(protocol: ProtocolType, font: FontSize) -> Picker {
    let mut picker = Picker::from_fontsize(font);
    picker.set_protocol_type(protocol);
    picker
}

/// `VIBEGUARD_GFX` 的取值：`off`（或认不出的值）表示不要原生图片，
/// `iterm2` / `kitty` / `sixel` 表示强制用这个协议。
fn forced(value: &str) -> Option<(ProtocolType, FontSize)> {
    let protocol = match value.trim().to_ascii_lowercase().as_str() {
        "iterm2" => ProtocolType::Iterm2,
        "kitty" => ProtocolType::Kitty,
        "sixel" => ProtocolType::Sixel,
        _ => return None,
    };
    Some((protocol, ASSUMED_FONT))
}

/// 在子进程里问终端，超时或问不出结果就当没有原生图片。
fn probe() -> Option<(ProtocolType, FontSize)> {
    // 输出被重定向时问不到任何东西，别白起一个进程
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let out = std::env::temp_dir().join(format!("vibeguard-graphics-{}", std::process::id()));
    let mut child = Command::new(exe)
        .arg(PROBE_FLAG)
        .arg(&out)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + PROBE_TIMEOUT;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => {
                let _ = child.kill();
                return None;
            }
        }
    }
    // 还没退出就是卡在读 stdin 上了：杀掉，别让它把键盘抢走
    let _ = child.kill();
    let _ = child.wait();

    let text = std::fs::read_to_string(&out).ok()?;
    let _ = std::fs::remove_file(&out);
    parse_probe(&text)
}

/// 子进程写出来的格式：`<协议> <字宽> <字高>`，例如 `iterm2 10 20`
fn parse_probe(text: &str) -> Option<(ProtocolType, FontSize)> {
    let mut parts = text.split_whitespace();
    let protocol = match parts.next()? {
        "kitty" => ProtocolType::Kitty,
        "iterm2" => ProtocolType::Iterm2,
        "sixel" => ProtocolType::Sixel,
        // 半块字符我们自己画，不需要图形协议
        _ => return None,
    };
    let w = parts.next()?.parse().ok()?;
    let h = parts.next()?.parse().ok()?;
    Some((protocol, FontSize::new(w, h)))
}

/// 子进程入口：查一次终端能力，把结果写进 `path` 就退出。
/// 父进程只读这个文件，不关心子进程是怎么死的。
pub fn probe_child(path: &Path) {
    let result = Picker::from_query_stdio().ok().map(|p| {
        let font = p.font_size();
        (p.protocol_type(), font.width, font.height)
    });
    let text = match result {
        Some((ProtocolType::Kitty, w, h)) => format!("kitty {w} {h}"),
        Some((ProtocolType::Iterm2, w, h)) => format!("iterm2 {w} {h}"),
        Some((ProtocolType::Sixel, w, h)) => format!("sixel {w} {h}"),
        _ => "none".to_string(),
    };
    let _ = std::fs::write(path, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试环境里 stdin/stdout 不是终端，必须老老实实退回字符画
    #[test]
    fn falls_back_to_character_art_when_not_a_terminal() {
        let mut g = Graphics::detect();
        let (avatar, banner) = g.split();
        assert!(avatar.is_none() && banner.is_none());
    }

    #[test]
    fn probe_output_is_parsed_into_a_protocol_and_font_size() {
        let (p, f) = parse_probe("kitty 8 17").unwrap();
        assert_eq!(p, ProtocolType::Kitty);
        assert_eq!((f.width, f.height), (8, 17));
        assert!(matches!(parse_probe("iterm2 10 20").unwrap().0, ProtocolType::Iterm2));
        assert!(matches!(parse_probe("sixel 9 18").unwrap().0, ProtocolType::Sixel));
    }

    /// 终端说不支持，或者子进程写了个残缺的结果，都要当没有
    #[test]
    fn unsupported_or_malformed_probe_output_means_no_native_image() {
        assert!(parse_probe("none").is_none());
        assert!(parse_probe("").is_none());
        assert!(parse_probe("kitty").is_none());
        assert!(parse_probe("kitty 8").is_none());
        assert!(parse_probe("kitty x y").is_none());
        assert!(parse_probe("halfblocks 10 20").is_none());
    }

    /// 环境变量能强制指定协议，也能一键关掉
    #[test]
    fn the_environment_can_force_or_disable_native_images() {
        assert!(matches!(forced("iterm2").unwrap().0, ProtocolType::Iterm2));
        assert!(matches!(forced(" KITTY ").unwrap().0, ProtocolType::Kitty));
        assert!(matches!(forced("sixel").unwrap().0, ProtocolType::Sixel));
        assert!(forced("off").is_none());
        assert!(forced("").is_none());
        assert!(forced("nonsense").is_none());
    }

    /// 真把图片交给终端时，缓冲区里应当出现 iTerm2 的图片转义序列，
    /// 而不是半块字符——这是「不用像素、直接显示原图」的关键一步。
    #[test]
    fn iterm2_protocol_emits_the_image_escape_sequence() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui_image::StatefulImage;

        let Some(face) = icon::logo_image() else {
            return;
        };
        let mut proto =
            picker(ProtocolType::Iterm2, FontSize::new(10, 20)).new_resize_protocol(face.clone());
        let mut term = Terminal::new(TestBackend::new(44, 22)).unwrap();
        term.draw(|f| {
            f.render_stateful_widget(StatefulImage::new(), f.area(), &mut proto);
        })
        .unwrap();

        let buf = term.backend().buffer();
        let text: String = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(text.contains("1337;File="), "缓冲区里没有 iTerm2 图片序列");
        assert!(!text.contains('▀'), "不该退回半块字符画");
    }
}
