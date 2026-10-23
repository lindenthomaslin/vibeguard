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
use std::time::Duration;

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
        if let Some(ref last) = self.last_frame
            && last.lines == f.lines
            && last.cursor == f.cursor
        {
            return Ok(());
        }
        self.last_frame = Some(f.clone());

        // 先写进内存缓冲，再一次性写出。stdout 是行缓冲的，
        // 逐行写会让每个 "\r\n" 触发一次系统调用，造成闪烁和延迟。
        let mut buf: Vec<u8> = Vec::with_capacity(1024);
        if self.cursor_row > 0 {
            queue!(buf, MoveUp(self.cursor_row as u16))?;
        }
        queue!(buf, MoveToColumn(0), Clear(ClearType::FromCursorDown))?;
        for (i, line) in f.lines.iter().enumerate() {
            if i > 0 {
                queue!(buf, Print("\r\n"))?;
            }
            queue!(buf, Print(line))?;
        }
        let last = f.lines.len() - 1;
        if last > f.cursor.0 {
            queue!(buf, MoveUp((last - f.cursor.0) as u16))?;
        }
        queue!(buf, MoveToColumn(f.cursor.1 as u16))?;
        self.cursor_row = f.cursor.0;

        let mut out = io::stdout().lock();
        out.write_all(&buf)?;
        out.flush()
    }

    /// 画最后一帧，并把光标移到框下方，后续输出接在它后面
    fn finish(&mut self, f: &Frame) -> io::Result<()> {
        self.draw(f)?;
        let mut buf: Vec<u8> = Vec::new();
        let last = f.lines.len() - 1;
        if last > f.cursor.0 {
            queue!(buf, MoveDown((last - f.cursor.0) as u16))?;
        }
        queue!(buf, Print("\r\n"))?;
        let mut out = io::stdout().lock();
        out.write_all(&buf)?;
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
    let mut width = ui::term_width();

    loop {
        painter.draw(&frame(ed, width, true))?;

        // 阻塞等第一个事件，然后把已排队的事件一次处理完再重绘，
        // 这样输入法一次提交多个字、快速连打时只画一帧，不会越积越慢。
        let mut ev = event::read()?;
        loop {
            match ev {
                Event::Key(key) => {
                    let outcome = handle_key(ed, key);
                    if outcome != Outcome::Continue {
                        let mut shown = Editor::new(&[]);
                        if let Outcome::Submit(text) = &outcome {
                            shown.insert_str(text);
                        }
                        painter.finish(&frame(&shown, width, false))?;
                        return Ok(outcome);
                    }
                }
                Event::Paste(text) => ed.insert_str(&text),
                Event::Resize(_, _) => width = ui::term_width(),
                _ => {}
            }
            if !event::poll(Duration::ZERO)? {
                break;
            }
            ev = event::read()?;
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
