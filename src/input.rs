//! 输入框：行内渲染（不进入备用屏幕），基于 crossterm 的原始模式。
//! 编辑逻辑在 `editor.rs`，这里负责换行布局、绘制和按键分发。

use crate::editor::Editor;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use console::{measure_text_width, style, truncate_str};
use unicode_width::UnicodeWidthChar;

/// 文本按显示宽度折行后的结果
#[derive(Debug, PartialEq, Eq)]
pub struct Layout {
    pub rows: Vec<String>,
    /// 光标所在的（行, 显示列），列不含输入框的前缀
    pub cursor: (usize, usize),
}

/// 按显示宽度折行：中文占 2 列，`\n` 强制换行。
/// 光标位于一整行末尾时，会落到下一行行首。
pub fn layout(chars: &[char], cursor: usize, width: usize) -> Layout {
    let width = width.max(2);
    let mut rows = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    let mut pos = None;

    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            if i == cursor {
                pos = Some((rows.len(), cur_w));
            }
            rows.push(std::mem::take(&mut cur));
            cur_w = 0;
            continue;
        }
        let w = c.width().unwrap_or(0);
        if cur_w + w > width {
            rows.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        if i == cursor {
            pos = Some((rows.len(), cur_w));
        }
        cur.push(c);
        cur_w += w;
    }

    if cursor >= chars.len() {
        if cur_w >= width {
            rows.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        pos = Some((rows.len(), cur_w));
    }
    rows.push(cur);

    Layout {
        rows,
        cursor: pos.unwrap_or((0, 0)),
    }
}

/// 要画到终端上的一帧：输入框 + 下方的命令菜单或快捷键提示
#[derive(Debug, Clone)]
pub struct Frame {
    pub lines: Vec<String>,
    /// 终端光标应停在的（行, 显示列），行号相对 `lines` 的第一行
    pub cursor: (usize, usize),
}

const MENU_ROWS: usize = 6;
const HINT: &str = "Enter 发送 · Ctrl-J 换行 · / 命令 · Ctrl-D 退出";

/// 构建一帧。`width` 是整个输入框的显示宽度（含左右边框）。
/// `show_menu` 为 false 时不画菜单与提示（提交后用来收起它们）。
pub fn frame(ed: &Editor, width: usize, show_menu: bool) -> Frame {
    let width = width.max(20);
    let text_w = width - 6;
    let lay = layout(ed.chars(), ed.cursor(), text_w);
    let edge = |s: &str| style(s).cyan().to_string();
    let bar = "─".repeat(width - 2);

    let mut lines = vec![edge(&format!("╭{bar}╮"))];
    for (i, row) in lay.rows.iter().enumerate() {
        let lead = if i == 0 {
            style("›").cyan().bold().to_string()
        } else {
            " ".to_string()
        };
        let pad = text_w.saturating_sub(measure_text_width(row));
        lines.push(format!(
            "{} {} {}{} {}",
            edge("│"),
            lead,
            row,
            " ".repeat(pad),
            edge("│")
        ));
    }
    lines.push(edge(&format!("╰{bar}╯")));

    if show_menu {
        let matches = ed.matches();
        if matches.is_empty() {
            let hint = truncate_str(&format!("  {HINT}"), width, "…").to_string();
            lines.push(style(hint).dim().to_string());
        } else {
            let sel = ed.menu_selected();
            let start = sel
                .saturating_sub(MENU_ROWS - 1)
                .min(matches.len().saturating_sub(MENU_ROWS));
            let end = (start + MENU_ROWS).min(matches.len());
            let name_w = matches.iter().map(|c| c.name.len()).max().unwrap_or(0) + 2;
            for (i, c) in matches.iter().enumerate().take(end).skip(start) {
                let line = format!("  {:<name_w$}{}", c.name, c.desc);
                let line = truncate_str(&line, width, "…").to_string();
                lines.push(if i == sel {
                    style(line).cyan().bold().to_string()
                } else {
                    style(line).dim().to_string()
                });
            }
        }
    }

    Frame {
        lines,
        cursor: (1 + lay.cursor.0, 4 + lay.cursor.1),
    }
}

/// 一次按键处理后，读取循环该做什么
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// 继续读取，需要时重绘
    Continue,
    /// 用户提交了一段输入
    Submit(String),
    /// 空输入时按 Ctrl-D
    Eof,
    /// 空输入时按 Ctrl-C
    Interrupt,
}

/// 把一次按键应用到编辑器。纯逻辑，不碰终端。
pub fn handle_key(ed: &mut Editor, key: KeyEvent) -> Outcome {
    if key.kind == KeyEventKind::Release {
        return Outcome::Continue;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        KeyCode::Enter if shift || alt => ed.newline(),
        KeyCode::Enter => {
            if let Some(text) = ed.submit() {
                return Outcome::Submit(text);
            }
        }
        KeyCode::Tab => ed.complete(),
        KeyCode::Backspace => ed.backspace(),
        KeyCode::Delete => ed.delete(),
        KeyCode::Left if ctrl || alt => ed.word_left(),
        KeyCode::Right if ctrl || alt => ed.word_right(),
        KeyCode::Left => ed.left(),
        KeyCode::Right => ed.right(),
        KeyCode::Home => ed.home(),
        KeyCode::End => ed.end(),
        KeyCode::Up if ed.menu_open() => ed.menu_move(-1),
        KeyCode::Down if ed.menu_open() => ed.menu_move(1),
        KeyCode::Up => ed.up(),
        KeyCode::Down => ed.down(),
        KeyCode::Esc => ed.clear(),
        KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
            'a' => ed.home(),
            'e' => ed.end(),
            'b' => ed.left(),
            'f' => ed.right(),
            'w' => ed.delete_word_back(),
            'u' => ed.kill_to_line_start(),
            'k' => ed.kill_to_line_end(),
            'j' => ed.newline(),
            'd' if ed.is_empty() => return Outcome::Eof,
            'd' => ed.delete(),
            'c' if ed.is_empty() => return Outcome::Interrupt,
            'c' => ed.clear(),
            _ => {}
        },
        KeyCode::Char(c) if !alt => ed.insert(c),
        _ => {}
    }
    Outcome::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lay(s: &str, cursor: usize, width: usize) -> Layout {
        let chars: Vec<char> = s.chars().collect();
        layout(&chars, cursor, width)
    }

    use crate::editor::Candidate;

    const CMDS: &[Candidate] = &[
        Candidate { name: "/help", desc: "帮助" },
        Candidate { name: "/clear", desc: "清空" },
    ];

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn with(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    fn type_str(ed: &mut Editor, s: &str) {
        for c in s.chars() {
            handle_key(ed, key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typing_then_enter_submits() {
        let mut ed = Editor::new(CMDS);
        type_str(&mut ed, "你好");
        assert_eq!(
            handle_key(&mut ed, key(KeyCode::Enter)),
            Outcome::Submit("你好".into())
        );
        assert!(ed.is_empty());
    }

    #[test]
    fn shift_enter_and_ctrl_j_insert_newline() {
        let mut ed = Editor::new(CMDS);
        type_str(&mut ed, "a");
        handle_key(&mut ed, with(KeyCode::Enter, KeyModifiers::SHIFT));
        type_str(&mut ed, "b");
        handle_key(&mut ed, with(KeyCode::Char('j'), KeyModifiers::CONTROL));
        type_str(&mut ed, "c");
        assert_eq!(ed.text(), "a\nb\nc");
    }

    #[test]
    fn ctrl_d_and_ctrl_c_depend_on_empty_input() {
        let mut ed = Editor::new(CMDS);
        assert_eq!(
            handle_key(&mut ed, with(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Outcome::Eof
        );
        assert_eq!(
            handle_key(&mut ed, with(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Outcome::Interrupt
        );
        type_str(&mut ed, "abc");
        assert_eq!(
            handle_key(&mut ed, with(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Outcome::Continue
        );
        assert!(ed.is_empty());
    }

    #[test]
    fn up_down_navigate_menu_when_open_else_history() {
        let mut ed = Editor::new(CMDS);
        type_str(&mut ed, "hi");
        handle_key(&mut ed, key(KeyCode::Enter));

        type_str(&mut ed, "/");
        assert!(ed.menu_open());
        handle_key(&mut ed, key(KeyCode::Down));
        assert_eq!(ed.menu_selected(), 1);
        assert_eq!(ed.text(), "/");

        handle_key(&mut ed, key(KeyCode::Esc));
        handle_key(&mut ed, key(KeyCode::Up));
        assert_eq!(ed.text(), "hi");
    }

    #[test]
    fn tab_completes_selected_command() {
        let mut ed = Editor::new(CMDS);
        type_str(&mut ed, "/c");
        handle_key(&mut ed, key(KeyCode::Tab));
        assert_eq!(ed.text(), "/clear ");
    }

    #[test]
    fn release_events_and_alt_chars_are_ignored() {
        let mut ed = Editor::new(CMDS);
        let mut release = key(KeyCode::Char('x'));
        release.kind = KeyEventKind::Release;
        handle_key(&mut ed, release);
        handle_key(&mut ed, with(KeyCode::Char('x'), KeyModifiers::ALT));
        assert!(ed.is_empty());
    }

    #[test]
    fn ctrl_shortcuts_edit_text() {
        let mut ed = Editor::new(CMDS);
        type_str(&mut ed, "foo bar");
        handle_key(&mut ed, with(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(ed.text(), "foo ");
        handle_key(&mut ed, with(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(ed.cursor(), 0);
        handle_key(&mut ed, with(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(ed.text(), "");
    }

    #[test]
    fn frame_box_rows_have_equal_display_width_with_cjk() {
        let mut ed = Editor::new(CMDS);
        ed.insert_str("你好 world\n第二行 with 中文 and a long tail that wraps around");
        let f = frame(&ed, 30, false);
        let box_lines = &f.lines;
        assert!(box_lines.len() >= 4);
        for l in box_lines {
            assert_eq!(measure_text_width(l), 30, "line: {l:?}");
        }
        assert!(box_lines[0].starts_with('╭') || box_lines[0].contains('╭'));
    }

    #[test]
    fn frame_cursor_points_inside_text_area() {
        let mut ed = Editor::new(CMDS);
        ed.insert_str("ab");
        let f = frame(&ed, 30, false);
        // 第 0 行是上边框，文本从第 1 行开始；列 = 4 个前缀列 + 2 个字符
        assert_eq!(f.cursor, (1, 6));
    }

    #[test]
    fn frame_shows_hint_when_no_menu_and_menu_when_slash() {
        let mut ed = Editor::new(CMDS);
        let f = frame(&ed, 60, true);
        assert!(f.lines.last().unwrap().contains("Enter"));

        ed.insert('/');
        let f = frame(&ed, 60, true);
        let tail: Vec<_> = f.lines.iter().skip(3).collect();
        assert_eq!(tail.len(), 2);
        assert!(tail[0].contains("/help"));
        assert!(tail[1].contains("/clear"));

        let f = frame(&ed, 60, false);
        assert_eq!(f.lines.len(), 3);
    }

    #[test]
    fn empty_input_has_one_row() {
        let l = lay("", 0, 10);
        assert_eq!(l.rows, [""]);
        assert_eq!(l.cursor, (0, 0));
    }

    #[test]
    fn ascii_wraps_at_width() {
        let l = lay("abcdefg", 7, 3);
        assert_eq!(l.rows, ["abc", "def", "g"]);
        assert_eq!(l.cursor, (2, 1));
    }

    #[test]
    fn cjk_counts_two_columns() {
        let l = lay("你好啊", 3, 5);
        assert_eq!(l.rows, ["你好", "啊"]);
        assert_eq!(l.cursor, (1, 2));
    }

    #[test]
    fn wide_char_moves_whole_to_next_row() {
        let l = lay("a你b", 3, 2);
        assert_eq!(l.rows, ["a", "你", "b"]);
        assert_eq!(l.cursor, (2, 1));
    }

    #[test]
    fn newline_starts_new_row() {
        let l = lay("ab\ncd", 5, 10);
        assert_eq!(l.rows, ["ab", "cd"]);
        assert_eq!(l.cursor, (1, 2));
    }

    #[test]
    fn cursor_on_newline_sits_at_end_of_its_row() {
        let l = lay("ab\ncd", 2, 10);
        assert_eq!(l.cursor, (0, 2));
    }

    #[test]
    fn trailing_newline_adds_empty_row() {
        let l = lay("ab\n", 3, 10);
        assert_eq!(l.rows, ["ab", ""]);
        assert_eq!(l.cursor, (1, 0));
    }

    #[test]
    fn cursor_after_full_row_goes_to_next_row() {
        let l = lay("abc", 3, 3);
        assert_eq!(l.rows, ["abc", ""]);
        assert_eq!(l.cursor, (1, 0));
    }

    #[test]
    fn cursor_in_middle_of_text() {
        let l = lay("abcdef", 4, 3);
        assert_eq!(l.rows, ["abc", "def"]);
        assert_eq!(l.cursor, (1, 1));
    }
}
