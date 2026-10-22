//! 兔子图标：从 PNG 图片生成，用彩色字符显示在 TUI 中。

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use image::ImageReader;
use std::io::Cursor;

pub const WIDTH: usize = 11;
pub const HEIGHT: usize = 4;

const RABBIT_PNG: &[u8] = include_bytes!("../rabbit.png");

fn lines_from_image() -> Vec<Line<'static>> {
    match load_image() {
        Ok(lines) => lines,
        Err(_) => fallback_lines(),
    }
}

fn load_image() -> Result<Vec<Line<'static>>, Box<dyn std::error::Error>> {
    let reader = ImageReader::new(Cursor::new(RABBIT_PNG));
    let reader = reader.with_guessed_format()?;
    let img = reader.decode()?;
    let img = img.resize_exact(
        (WIDTH * 2) as u32,
        (HEIGHT * 2) as u32,
        image::imageops::FilterType::Nearest,
    );
    let img = img.to_rgba8();

    let mut result = Vec::new();
    for y in 0..HEIGHT {
        let mut spans = Vec::new();
        for x in 0..WIDTH {
            let top_px = img.get_pixel((x * 2) as u32, (y * 2) as u32);
            let bot_px = img.get_pixel((x * 2) as u32, (y * 2 + 1) as u32);

            let top_color = pixel_to_color(*top_px);
            let bot_color = pixel_to_color(*bot_px);

            let (ch, style) = half_block(top_color, bot_color);
            spans.push(Span::styled(ch.to_string(), style));
        }
        result.push(Line::from(spans));
    }

    Ok(result)
}

fn pixel_to_color(px: image::Rgba<u8>) -> Option<Color> {
    let [r, g, b, a] = px.0;
    if a < 128 {
        return None;
    }
    let brightness = ((r as u16 + g as u16 + b as u16) / 3) as u8;
    if brightness > 100 {
        Some(Color::Cyan)
    } else {
        None
    }
}

fn half_block(top: Option<Color>, bottom: Option<Color>) -> (char, Style) {
    match (top, bottom) {
        (None, None) => (' ', Style::default()),
        (Some(t), None) => ('▀', Style::default().fg(t)),
        (None, Some(b)) => ('▄', Style::default().fg(b)),
        (Some(t), Some(b)) if t == b => ('█', Style::default().fg(t)),
        (Some(t), Some(b)) => ('▀', Style::default().fg(t).bg(b)),
    }
}

fn fallback_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled("   🐰   ".to_string(), Style::default().fg(Color::Cyan))),
        Line::from(Span::styled("  /‾\\ /‾\\".to_string(), Style::default().fg(Color::Cyan))),
        Line::from(Span::styled(" (•◡•) ".to_string(), Style::default().fg(Color::Cyan))),
        Line::from(Span::styled("  ∿∿∿  ".to_string(), Style::default().fg(Color::Cyan))),
    ]
}

pub fn lines() -> Vec<Line<'static>> {
    lines_from_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_match_declared_size() {
        let ls = lines();
        assert_eq!(ls.len(), HEIGHT);
        for l in &ls {
            assert_eq!(l.spans.len(), WIDTH);
        }
    }
}
