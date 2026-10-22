//! 全屏界面的绘制：把状态画到 ratatui 的 `Frame` 上。

use super::layout::areas;
use crate::chat::{Scroll, Transcript};
use crate::{input, spider};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use unicode_width::UnicodeWidthStr;

const DIM: Style = Style::new().add_modifier(Modifier::DIM);
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// 逐条确认时，终端低于这个高度就把图标栏收成一行，空间留给卡片和 diff
const REVIEW_FULL_HEADER_MIN_HEIGHT: u16 = 30;

/// 选择区：斜杠命令菜单、逐条确认的选项等
pub struct Choice {
    pub title: String,
    /// （名称, 说明）
    pub items: Vec<(String, String)>,
    pub selected: usize,
}

pub struct Props<'a> {
    pub model: &'a str,
    pub thinking: &'a str,
    pub cwd: &'a str,
    pub transcript: &'a Transcript,
    pub input: &'a [char],
    pub cursor: usize,
    /// 输入框边框上的标签（`/model` 表单里正在改哪一项）
    pub input_label: Option<&'a str>,
    /// 输入框现在能不能输入；不能时不显示光标
    pub input_active: bool,
    pub choice: Option<&'a Choice>,
    /// 逐条确认中：对话区停在最后一个条目的开头，小终端上图标栏收成一行
    pub focus_last: bool,
    /// 正在等待 AI 时的提示文字
    pub busy: Option<&'a str>,
    /// 动画帧计数
    pub tick: usize,
    pub status: &'a str,
    /// 已输入文本的 token 数估算
    pub input_tokens: usize,
    pub scroll: &'a mut Scroll,
}

fn accent() -> Style {
    Style::new().fg(Color::Cyan)
}

fn clamp_u16(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// 让 `selected` 所在行落在 `height` 行的窗口里，返回窗口起点
fn window_start(selected: usize, height: usize) -> usize {
    (selected + 1).saturating_sub(height.max(1))
}

/// 按显示宽度补空格（中文占 2 列），`format!` 的对齐只数字符个数
fn pad_to(s: &str, width: usize) -> String {
    format!("{s}{}", " ".repeat(width.saturating_sub(s.width())))
}

pub fn draw(frame: &mut Frame, mut p: Props<'_>) {
    let area = frame.area();
    // 输入框：左右边框各 1 列、内边距各 1 列，前缀 "› " 2 列
    let text_w = (area.width as usize).saturating_sub(6).max(2);
    let lay = input::layout(p.input, p.cursor, text_w);
    let choice_rows = match p.choice {
        Some(c) if !c.items.is_empty() => clamp_u16(c.items.len() + 2),
        _ => 0,
    };
    let compact = p.focus_last && area.height < REVIEW_FULL_HEADER_MIN_HEIGHT;
    let a = areas(area, clamp_u16(lay.rows.len()), choice_rows, compact);

    draw_header(frame, a.header, &p);
    draw_body(frame, a.body, &mut p);
    if let Some(c) = p.choice {
        draw_choice(frame, a.choice, c);
    }
    draw_input(
        frame,
        a.input,
        &lay,
        p.busy.is_some(),
        p.input_label,
        p.input_active,
    );
    if a.status.height > 0 {
        let status_text = if p.input_tokens > 0 {
            format!("{} · Tokens: {}", p.status, p.input_tokens)
        } else {
            p.status.to_string()
        };
        frame.render_widget(
            Paragraph::new(Line::styled(format!(" {}", status_text), DIM)),
            a.status,
        );
    }
}

fn draw_header(frame: &mut Frame, rect: Rect, p: &Props<'_>) {
    if rect.height == 0 || rect.width == 0 {
        return;
    }
    let title = format!("VibeGuard v{}", env!("CARGO_PKG_VERSION"));
    if rect.height < spider::HEIGHT as u16 + 2 {
        let line = Line::from(vec![
            Span::styled("✻ ", accent().add_modifier(Modifier::BOLD)),
            Span::styled(title, Style::new().add_modifier(Modifier::BOLD)),
            Span::styled(format!(" · {} · thinking {}", p.model, p.thinking), DIM),
        ]);
        frame.render_widget(Paragraph::new(line), Rect { height: 1, ..rect });
        return;
    }

    let block = Block::bordered().border_style(accent().add_modifier(Modifier::DIM));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let icon = Rect::new(inner.x + 1, inner.y, spider::WIDTH as u16, spider::HEIGHT as u16)
        .intersection(inner);
    frame.render_widget(Paragraph::new(spider::lines()), icon);

    let text_x = inner.x + 1 + spider::WIDTH as u16 + 2;
    let text = Rect::new(text_x, inner.y, inner.right().saturating_sub(text_x), inner.height);
    let lines = vec![
        Line::styled(title, Style::new().add_modifier(Modifier::BOLD)),
        Line::styled("AI 生成代码的安全扫描与修复", DIM),
        Line::from(vec![
            Span::styled("model ", DIM),
            Span::raw(p.model.to_string()),
            Span::styled(" · thinking ", DIM),
            Span::raw(p.thinking.to_string()),
        ]),
        Line::from(vec![Span::styled("cwd   ", DIM), Span::raw(p.cwd.to_string())]),
    ];
    frame.render_widget(Paragraph::new(lines), text);
}

fn draw_body(frame: &mut Frame, rect: Rect, p: &mut Props<'_>) {
    if rect.height == 0 || rect.width == 0 {
        return;
    }
    let (mut lines, last_start) = p.transcript.lines_marked(rect.width as usize);
    if let Some(msg) = p.busy {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(vec![
            Span::styled(SPINNER[p.tick % SPINNER.len()], accent()),
            Span::styled(format!(" {msg}"), DIM),
        ]));
    }
    let height = rect.height as usize;
    let mut top = p.scroll.resolve(lines.len(), height);
    // 逐条确认时让当前这一条的开头可见，而不是滚到它的末尾；之后用户仍可自行滚动
    if p.focus_last && p.scroll.is_following() {
        p.scroll.pin(last_start);
        top = p.scroll.resolve(lines.len(), height);
    }
    let end = (top + height).min(lines.len());
    frame.render_widget(Paragraph::new(lines[top..end].to_vec()), rect);
}

fn draw_choice(frame: &mut Frame, rect: Rect, c: &Choice) {
    if rect.height < 3 {
        return;
    }
    let block = Block::bordered()
        .border_style(accent())
        .title(Span::styled(format!(" {} ", c.title), accent()));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let name_w = c.items.iter().map(|(n, _)| n.width()).max().unwrap_or(0) + 2;
    let start = window_start(c.selected, inner.height as usize);
    let lines: Vec<Line> = c
        .items
        .iter()
        .enumerate()
        .skip(start)
        .take(inner.height as usize)
        .map(|(i, (name, desc))| {
            let text = format!("{}{desc}", pad_to(name, name_w));
            if i == c.selected {
                Line::styled(
                    format!("› {text}"),
                    accent().add_modifier(Modifier::BOLD),
                )
            } else {
                Line::styled(format!("  {text}"), DIM)
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_input(
    frame: &mut Frame,
    rect: Rect,
    lay: &input::Layout,
    busy: bool,
    label: Option<&str>,
    active: bool,
) {
    if rect.height == 0 {
        return;
    }
    let border = if busy || !active { DIM } else { accent() };
    let mut block = Block::bordered()
        .border_style(border)
        .padding(Padding::horizontal(1));
    if let Some(label) = label {
        block = block.title(Span::styled(format!(" {label} "), accent()));
    }
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 || inner.width < 3 {
        return;
    }

    let start = window_start(lay.cursor.0, inner.height as usize);
    let lines: Vec<Line> = lay
        .rows
        .iter()
        .enumerate()
        .skip(start)
        .take(inner.height as usize)
        .map(|(i, row)| {
            let lead = if i == 0 {
                Span::styled("› ", accent().add_modifier(Modifier::BOLD))
            } else {
                Span::raw("  ")
            };
            Line::from(vec![lead, Span::raw(row.clone())])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);

    if active {
        let x = (inner.x + 2 + clamp_u16(lay.cursor.1)).min(inner.right().saturating_sub(1));
        let y = inner.y + clamp_u16(lay.cursor.0 - start);
        frame.set_cursor_position((x, y));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::Entry;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    struct Fixture {
        transcript: Transcript,
        scroll: Scroll,
        input: Vec<char>,
        choice: Option<Choice>,
        focus_last: bool,
        input_label: Option<&'static str>,
        input_active: bool,
        busy: Option<String>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                transcript: Transcript::default(),
                scroll: Scroll::default(),
                input: Vec::new(),
                choice: None,
                focus_last: false,
                input_label: None,
                input_active: true,
                busy: None,
            }
        }

        fn render(&mut self, w: u16, h: u16) -> Terminal<TestBackend> {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| {
                draw(
                    f,
                    Props {
                        model: "deepseek-chat",
                        thinking: "auto",
                        cwd: "~/proj",
                        transcript: &self.transcript,
                        input: &self.input,
                        cursor: self.input.len(),
                        choice: self.choice.as_ref(),
                        focus_last: self.focus_last,
                        input_label: self.input_label,
                        input_active: self.input_active,
                        busy: self.busy.as_deref(),
                        tick: 0,
                        status: "Enter 发送",
                        input_tokens: 0,
                        scroll: &mut self.scroll,
                    },
                );
            })
            .unwrap();
            term
        }
    }

    /// 屏幕内容，每行一个字符串。宽字符后面的补位格会让行里多出空格，所以比较时只用 ASCII 片段。
    fn screen(term: &Terminal<TestBackend>) -> Vec<String> {
        let buf = term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn full_layout_has_header_pinned_input_and_status() {
        let mut fx = Fixture::new();
        let term = fx.render(80, 30);
        let rows = screen(&term);
        assert!(rows[0].starts_with('┌'));
        assert!(rows.iter().take(6).any(|r| r.contains("VibeGuard v")));
        assert!(rows.iter().take(6).any(|r| r.contains("deepseek-chat")));
        assert!(rows.iter().take(6).any(|r| r.contains('█') || r.contains('▀')));
        // 状态栏在最后一行，输入框（上边框、文字、下边框）占倒数第 2 到第 4 行
        assert!(rows[26].starts_with('┌'));
        assert!(rows[27].contains('›'));
        assert!(rows[28].starts_with('└'));
        assert!(rows[29].contains("Enter"));
    }

    #[test]
    fn input_box_and_status_do_not_move_when_content_grows() {
        let mut fx = Fixture::new();
        let before = screen(&fx.render(80, 30));
        for i in 0..100 {
            fx.transcript.push(Entry::Info(format!("message {i}")));
        }
        let after = screen(&fx.render(80, 30));
        assert_eq!(before[26..], after[26..]);
        assert!(after.iter().any(|r| r.contains("message 99")));
        assert!(!after.iter().any(|r| r.contains("message 0 ")));
    }

    #[test]
    fn scrolling_up_shows_older_messages_and_keeps_input_fixed() {
        let mut fx = Fixture::new();
        for i in 0..100 {
            fx.transcript.push(Entry::Info(format!("message {i}")));
        }
        fx.render(80, 30);
        fx.scroll.to_top();
        let rows = screen(&fx.render(80, 30));
        assert!(rows.iter().any(|r| r.contains("message 0")));
        assert!(!rows.iter().any(|r| r.contains("message 99")));
        assert!(rows[27].contains('›'));
    }

    #[test]
    fn choice_area_lists_items_and_marks_selection() {
        let mut fx = Fixture::new();
        fx.choice = Some(Choice {
            title: "commands".into(),
            items: vec![("/help".into(), "help".into()), ("/exit".into(), "quit".into())],
            selected: 1,
        });
        let rows = screen(&fx.render(80, 30));
        assert!(rows.iter().any(|r| r.contains("commands")));
        assert!(rows.iter().any(|r| r.contains("  /help") && r.contains("help")));
        assert!(rows.iter().any(|r| r.contains("› /exit")));
        // 输入框仍然在最底部
        assert!(rows[27].contains('›'));
    }

    #[test]
    fn long_choice_list_scrolls_to_keep_selection_visible() {
        let mut fx = Fixture::new();
        fx.choice = Some(Choice {
            title: "t".into(),
            items: (0..30).map(|i| (format!("item{i}"), String::new())).collect(),
            selected: 29,
        });
        let rows = screen(&fx.render(80, 30));
        assert!(rows.iter().any(|r| r.contains("› item29")));
    }

    #[test]
    fn typed_text_appears_and_cursor_follows_it() {
        let mut fx = Fixture::new();
        fx.input = "hello".chars().collect();
        let mut term = fx.render(80, 30);
        let rows = screen(&term);
        assert!(rows[27].contains("› hello"));
        let pos = term.get_cursor_position().unwrap();
        // 左边框 1 列 + 内边距 1 列 + 前缀 "› " 2 列 + 已输入 5 列
        assert_eq!((pos.x, pos.y), (1 + 1 + 2 + 5, 27));
        assert!(rows[27].starts_with("│ ›"), "prompt should not touch the border");
    }

    #[test]
    fn choice_descriptions_align_with_cjk_names() {
        let mut fx = Fixture::new();
        fx.choice = Some(Choice {
            title: "t".into(),
            items: vec![
                ("应用".into(), "写入".into()),
                ("应用此条及之后全部".into(), "写入".into()),
                ("abc".into(), "写入".into()),
            ],
            selected: 0,
        });
        let rows = screen(&fx.render(80, 30));
        // 每个格子对应字符串里的一个字符（宽字符后面的补位格是空格）
        let cols: Vec<usize> = rows
            .iter()
            .filter(|r| r.contains('写'))
            .map(|r| r.chars().position(|c| c == '写').unwrap())
            .collect();
        assert_eq!(cols.len(), 3);
        assert!(cols.windows(2).all(|w| w[0] == w[1]), "{cols:?}");
    }

    fn review_fixture(focus_last: bool) -> Fixture {
        let (findings, fixes) = crate::review::demo_data();
        let mut fx = Fixture::new();
        fx.transcript.push(Entry::Summary(findings.clone()));
        fx.transcript.push(Entry::Card {
            finding: Box::new(findings[0].clone()),
            fix: Box::new(fixes[0].clone()),
            index: 1,
            total: 3,
        });
        fx.focus_last = focus_last;
        fx.choice = Some(Choice {
            title: "review".into(),
            items: vec![("a".into(), String::new()); 4],
            selected: 0,
        });
        fx
    }

    #[test]
    fn review_keeps_the_top_of_a_tall_card_visible() {
        // 22 行：图标栏收成 1 行后对话区只有 11 行，比卡片（12 行）矮
        let rows = screen(&review_fixture(true).render(80, 22));
        assert!(rows.iter().any(|r| r.contains("VG-001") && r.contains("(1/3)")));
        assert!(rows[0].contains("VibeGuard") && !rows[0].starts_with('┌'));
        // 输入框依然固定在底部
        assert!(rows[18].starts_with('┌') && rows[19].contains('›') && rows[20].starts_with('└'));
        assert!(rows[21].contains("Enter"));
    }

    #[test]
    fn without_focus_the_body_follows_the_bottom_and_clips_the_card_top() {
        let rows = screen(&review_fixture(false).render(80, 22));
        assert!(!rows.iter().any(|r| r.contains("VG-001")));
    }

    #[test]
    fn short_card_still_fits_entirely_on_a_24_row_screen() {
        let rows = screen(&review_fixture(true).render(80, 24));
        assert!(rows.iter().any(|r| r.contains("VG-001")));
        assert!(rows.iter().any(|r| r.contains("│ + API_KEY")));
    }

    #[test]
    fn input_label_is_shown_on_the_box_border_and_text_is_plain() {
        let mut fx = Fixture::new();
        fx.input = "sk-visible".chars().collect();
        fx.input_label = Some("API Key");
        let term = fx.render(80, 30);
        let rows = screen(&term);
        assert!(rows[26].contains("API Key"), "{:?}", rows[26]);
        assert!(rows[27].contains("› sk-visible"), "{:?}", rows[27]);
    }

    #[test]
    fn inactive_input_has_no_label_when_unset_and_hides_the_cursor() {
        let mut fx = Fixture::new();
        fx.input_active = false;
        let mut term = fx.render(80, 30);
        let rows = screen(&term);
        assert!(!rows[26].contains('['), "no label expected: {:?}", rows[26]);
        assert!(rows[26].starts_with('┌'));
        let pos = term.get_cursor_position().unwrap();
        assert_eq!((pos.x, pos.y), (0, 0), "no cursor is placed while the input is inactive");
    }

    #[test]
    fn busy_shows_spinner_message_in_body() {
        let mut fx = Fixture::new();
        fx.busy = Some("等待响应".into());
        let rows = screen(&fx.render(80, 30));
        assert!(rows.iter().any(|r| r.contains('⠋')));
    }

    #[test]
    fn short_terminal_uses_compact_header() {
        let mut fx = Fixture::new();
        let rows = screen(&fx.render(80, 12));
        assert!(rows[0].contains("VibeGuard"));
        assert!(!rows[0].starts_with('┌'));
        assert!(rows[9].contains('›'));
    }

    #[test]
    fn drawing_never_panics_on_tiny_or_odd_sizes() {
        let mut fx = Fixture::new();
        fx.input = "你好 world 这是一段很长的输入".chars().collect();
        fx.choice = Some(Choice {
            title: "t".into(),
            items: vec![("a".into(), "b".into()); 8],
            selected: 3,
        });
        fx.busy = Some("x".into());
        fx.transcript.push(Entry::User("hi".into()));
        for w in [1, 2, 5, 20, 49, 50, 120] {
            for h in [1, 2, 3, 4, 5, 8, 19, 20, 40] {
                fx.render(w, h);
            }
        }
    }
}
