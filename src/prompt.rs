//! 输入框的终端部分：原始模式、重绘与事件循环。
//! 编辑逻辑在 `editor.rs`，布局与按键处理在 `input.rs`。

use crate::editor::Editor;
use crate::input::{Frame, Outcome, frame, handle_key};
use crate::ui;
use anyhow::Result;
use crossterm::cursor::{MoveDown, MoveToColumn, MoveUp};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use crossterm::style::Print;
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};
use std::io::{self, BufRead, IsTerminal, Write};

/// 进入原始模式并开启括号粘贴；离开作用域（含出错、panic）时自动还原
struct RawMode;

impl RawMode {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let _ = execute!(io::stdout(), EnableBracketedPaste);
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        let _ = terminal::disable_raw_mode();
    }
}

/// 记住上一帧光标所在的行，重绘时先回到框顶
#[derive(Default)]
struct Painter {
    cursor_row: usize,
    last_frame: Option<Frame>,
}

impl Painter {
    fn draw(&mut self, f: &Frame) -> io::Result<()> {
        // 跳过完全相同的帧
        if let Some(ref last) = self.last_frame {
            if last.lines == f.lines && last.cursor == f.cursor {
                return Ok(());
            }
        }
        self.last_frame = Some(f.clone());

        let mut out = io::stdout().lock();
        if self.cursor_row > 0 {
            queue!(out, MoveUp(self.cursor_row as u16))?;
        }
        queue!(out, MoveToColumn(0), Clear(ClearType::FromCursorDown))?;
        for (i, line) in f.lines.iter().enumerate() {
            if i > 0 {
                queue!(out, Print("\r\n"))?;
            }
            queue!(out, Print(line))?;
        }
        let last = f.lines.len() - 1;
        if last > f.cursor.0 {
            queue!(out, MoveUp((last - f.cursor.0) as u16))?;
        }
        queue!(out, MoveToColumn(f.cursor.1 as u16))?;
        self.cursor_row = f.cursor.0;
        out.flush()
    }

    /// 画最后一帧，并把光标移到框下方，后续输出接在它后面
    fn finish(&mut self, f: &Frame) -> io::Result<()> {
        self.draw(f)?;
        let mut out = io::stdout().lock();
        let last = f.lines.len() - 1;
        if last > f.cursor.0 {
            queue!(out, MoveDown((last - f.cursor.0) as u16))?;
        }
        queue!(out, Print("\r\n"))?;
        out.flush()
    }
}

/// 读取一次输入。stdin 或 stdout 不是终端（管道、重定向）时退化为按行读取。
pub fn read_input(ed: &mut Editor) -> Result<Outcome> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return read_plain();
    }

    let _raw = RawMode::enter()?;
    let mut painter = Painter::default();
    let width = ui::term_width();

    loop {
        painter.draw(&frame(ed, width, true))?;
        match event::read()? {
            Event::Key(key) => {
                let outcome = handle_key(ed, key);
                if outcome == Outcome::Continue {
                    continue;
                }
                let mut shown = Editor::new(&[]);
                if let Outcome::Submit(text) = &outcome {
                    shown.insert_str(text);
                }
                painter.finish(&frame(&shown, width, false))?;
                return Ok(outcome);
            }
            Event::Paste(text) => ed.insert_str(&text),
            Event::Resize(_, _) => {
                // 终端宽度改变，重新缓存
                let new_width = ui::term_width();
                if new_width != width {
                    painter.last_frame = None;
                }
            }
            _ => {}
        }
    }
}

fn read_plain() -> Result<Outcome> {
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line)? == 0 {
        return Ok(Outcome::Eof);
    }
    let text = line.trim();
    Ok(if text.is_empty() {
        Outcome::Continue
    } else {
        Outcome::Submit(text.to_string())
    })
}
