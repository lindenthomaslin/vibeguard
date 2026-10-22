//! 超低延迟输入框：只做必要的终端操作

use crate::editor::Editor;
use crate::input::{Outcome, handle_key};
use crossterm::event::{self, Event, DisableBracketedPaste, EnableBracketedPaste};
use crossterm::terminal::{self, ClearType};
use crossterm::cursor::{MoveTo, Show, Hide};
use crossterm::style::Print;
use crossterm::{execute, queue};
use std::io::{self, BufRead, IsTerminal, Write};
use anyhow::Result;
use console::style;

pub fn read_input_fast(ed: &mut Editor) -> Result<Outcome> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return read_plain();
    }

    // 进入原始模式
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();

    execute!(stdout, EnableBracketedPaste, Hide)?;

    // 立即渲染初始框
    render_frame(&mut stdout, ed)?;

    let result = loop {
        // 轮询事件（不阻塞）
        if event::poll(std::time::Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    let outcome = handle_key(ed, key);

                    // 立即渲染改变
                    render_frame(&mut stdout, ed)?;

                    if outcome != Outcome::Continue {
                        break outcome;
                    }
                }
                Event::Paste(text) => {
                    ed.insert_str(&text);
                    render_frame(&mut stdout, ed)?;
                }
                Event::Resize(_, _) => {
                    execute!(stdout, crossterm::terminal::Clear(ClearType::All))?;
                    render_frame(&mut stdout, ed)?;
                }
                _ => {}
            }
        }
    };

    // 清理
    execute!(stdout, DisableBracketedPaste, Show)?;
    terminal::disable_raw_mode()?;

    Ok(result)
}

fn render_frame(stdout: &mut io::Stdout, ed: &Editor) -> io::Result<()> {
    let text = ed.text();
    let cursor = ed.cursor();

    // 移到左上角
    queue!(stdout, MoveTo(0, 0))?;

    // 最小框线
    let border_len = (text.len() + 6).min(80);
    queue!(stdout, Print("╭"))?;
    for _ in 0..border_len {
        queue!(stdout, Print("─"))?;
    }
    queue!(stdout, Print("╮\r\n"))?;

    // 输入行
    queue!(stdout, Print("│ › "))?;
    queue!(stdout, Print(&text))?;
    for _ in text.len()..border_len.saturating_sub(4) {
        queue!(stdout, Print(" "))?;
    }
    queue!(stdout, Print(" │\r\n"))?;

    // 下框线
    queue!(stdout, Print("╰"))?;
    for _ in 0..border_len {
        queue!(stdout, Print("─"))?;
    }
    queue!(stdout, Print("╯"))?;

    // 菜单
    if ed.menu_open() {
        queue!(stdout, Print("\r\n"))?;
        let matches = ed.matches();
        let sel = ed.menu_selected();
        for (i, c) in matches.iter().take(6).enumerate() {
            if i > 0 {
                queue!(stdout, Print("\r\n"))?;
            }
            let line = format!("  {:<20}{}", c.name, c.desc);
            if i == sel {
                queue!(stdout, Print(style(line).cyan().bold().to_string()))?;
            } else {
                queue!(stdout, Print(style(line).dim().to_string()))?;
            }
        }
    }

    // 光标位置
    queue!(stdout, MoveTo((4 + cursor as u16).min(border_len as u16 - 2), 1))?;
    stdout.flush()?;

    Ok(())
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
