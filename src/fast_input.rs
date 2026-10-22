//! 高性能输入框：最小化每帧的计算开销

use crate::editor::Editor;
use crate::input::{Outcome, handle_key};
use crate::ui;
use anyhow::Result;
use crossterm::cursor::{MoveDown, MoveToColumn, MoveUp};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use crossterm::style::Print;
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};
use std::io::{self, BufRead, IsTerminal, Write};
use console::style;

struct FastPainter {
    cursor_row: usize,
    last_text: String,
    last_cursor: usize,
    last_show_menu: bool,
}

impl FastPainter {
    fn new() -> Self {
        Self {
            cursor_row: 0,
            last_text: String::new(),
            last_cursor: 0,
            last_show_menu: false,
        }
    }

    fn draw_fast(&mut self, ed: &Editor, width: usize, show_menu: bool) -> io::Result<()> {
        let text = ed.text();
        let cursor = ed.cursor();

        // 跳过完全相同的帧
        if text == self.last_text && cursor == self.last_cursor && show_menu == self.last_show_menu {
            return Ok(());
        }

        self.last_text = text.clone();
        self.last_cursor = cursor;
        self.last_show_menu = show_menu;

        let mut out = io::stdout().lock();

        // 快速绘制：最小化终端操作
        if self.cursor_row > 0 {
            queue!(out, MoveUp(self.cursor_row as u16))?;
        }
        queue!(out, MoveToColumn(0), Clear(ClearType::FromCursorDown))?;

        // 简化框线
        let bar = "─".repeat(width - 2);
        let edge = |s: &str| style(s).cyan().to_string();

        queue!(out, Print(edge(&format!("╭{bar}╮\r\n"))))?;
        queue!(out, Print(format!("{} › {} {}\r\n", edge("│"), text, edge("│"))))?;
        queue!(out, Print(edge(&format!("╰{bar}╯"))))?;

        if show_menu {
            let matches = ed.matches();
            if !matches.is_empty() {
                queue!(out, Print("\r\n"))?;
                let sel = ed.menu_selected();
                for (i, c) in matches.iter().enumerate().take(6) {
                    let line = format!("  {:<20}{}", c.name, c.desc);
                    queue!(out, Print(
                        if i == sel {
                            format!("{}\r\n", style(line).cyan().bold())
                        } else {
                            format!("{}\r\n", style(line).dim())
                        }
                    ))?;
                }
                self.cursor_row = 4 + matches.len().min(6);
            } else {
                self.cursor_row = 2;
            }
        } else {
            self.cursor_row = 2;
        }

        queue!(out, MoveUp((self.cursor_row - 1) as u16))?;
        queue!(out, MoveToColumn((4 + cursor) as u16))?;

        out.flush()
    }

    fn finish(&mut self, ed: &Editor, width: usize) -> io::Result<()> {
        self.draw_fast(ed, width, false)?;
        let mut out = io::stdout().lock();
        queue!(out, MoveDown(1 as u16))?;
        queue!(out, Print("\r\n"))?;
        out.flush()
    }
}

pub fn read_input_fast(ed: &mut Editor) -> Result<Outcome> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return read_plain();
    }

    terminal::enable_raw_mode()?;
    let _ = execute!(io::stdout(), EnableBracketedPaste);

    let mut painter = FastPainter::new();
    let width = ui::term_width();

    let result = loop {
        painter.draw_fast(ed, width, true)?;

        match event::read()? {
            Event::Key(key) => {
                let outcome = handle_key(ed, key);
                if outcome != Outcome::Continue {
                    painter.finish(ed, width)?;
                    break outcome;
                }
            }
            Event::Paste(text) => ed.insert_str(&text),
            _ => {}
        }
    };

    let _ = execute!(io::stdout(), DisableBracketedPaste);
    terminal::disable_raw_mode()?;

    Ok(result)
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
