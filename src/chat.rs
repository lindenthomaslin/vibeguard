//! 对话记录的数据模型：条目、折行与滚动。纯逻辑，不碰终端，便于测试。

use crate::finding::{Finding, Fix};
use crate::scan_ui;
use crate::theme::Look;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// 对话区里的一个条目
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    User(String),
    Assistant { reasoning: String, content: String },
    Info(String),
    Success(String),
    Warn(String),
    Error(String),
    /// 已经排好版的内容（配置、帮助等），原样显示
    Block(Vec<Line<'static>>),
    /// 扫描结果汇总
    Summary(Vec<Finding>),
    /// 一次本地扫描的完整结果：汇总 + 逐条列表
    Scan(Box<crate::scan::LocalReport>),
    /// 第二段：模型对剩余清单条目的判定
    ScanAi {
        judgements: Vec<crate::scan_ai::Judgement>,
        failed: Vec<String>,
    },
    /// 一条待确认的发现及其修复 diff，渲染时按当前宽度排版
    Card {
        finding: Box<Finding>,
        fix: Box<Fix>,
        index: usize,
        total: usize,
    },
    /// 一处文件改动：路径 + 改动前后的代码，按当前宽度排版成 diff
    Diff {
        path: String,
        old: String,
        new: String,
    },
}

fn text_width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// 按显示宽度折行：中文占 2 列，英文尽量在空白处断开，超长的词强制断开。
/// `\n` 强制换行，空行保留。
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(2);
    let mut rows = Vec::new();
    for line in text.split('\n') {
        wrap_line(line, width, &mut rows);
    }
    rows
}

fn wrap_line(line: &str, width: usize, rows: &mut Vec<String>) {
    let mut cur = String::new();
    let mut cur_w = 0;
    // 最近一个空白之后的字节位置：超宽时在这里断开
    let mut brk: Option<usize> = None;
    // 软换行之后丢弃新一行开头的空白；逻辑行本身开头的缩进仍然保留
    let mut skip_ws = false;
    for c in line.chars() {
        if skip_ws && c.is_whitespace() {
            continue;
        }
        let w = c.width().unwrap_or(0);
        if cur_w + w > width {
            if c.is_whitespace() {
                // 行尾溢出的空白直接丢掉，不让下一行以空白开头
                rows.push(std::mem::take(&mut cur));
                cur_w = 0;
                brk = None;
                skip_ws = true;
                continue;
            }
            match brk {
                Some(b) if !cur[..b].trim().is_empty() => {
                    let tail = cur.split_off(b);
                    rows.push(cur.trim_end().to_string());
                    cur_w = text_width(&tail);
                    cur = tail;
                }
                _ => {
                    rows.push(std::mem::take(&mut cur));
                    cur_w = 0;
                }
            }
            brk = None;
        }
        cur.push(c);
        cur_w += w;
        skip_ws = false;
        if c.is_whitespace() {
            brk = Some(cur.len());
        }
    }
    // 以溢出的空白结尾时，当前行已经推入，不再多推一个空行
    if !(cur.is_empty() && skip_ws) {
        rows.push(cur);
    }
}

fn prefixed(rows: Vec<String>, lead: Span<'static>, style: Style) -> Vec<Line<'static>> {
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| {
            let first = if i == 0 { lead.clone() } else { Span::raw("  ") };
            Line::from(vec![first, Span::styled(row, style)])
        })
        .collect()
}

fn marker(symbol: &'static str, color: Color) -> Span<'static> {
    Span::styled(
        format!("{symbol} "),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

impl Entry {
    /// 渲染成若干行；`width` 是对话区的可用列数，`look` 决定配色和标记符号
    pub fn render(&self, width: usize, look: &Look) -> Vec<Line<'static>> {
        let body_w = width.saturating_sub(2).max(2);
        let plain = Style::default();
        let g = &look.glyphs;
        match self {
            Entry::User(t) => prefixed(wrap(t, body_w), marker(g.user, look.palette.accent), plain),
            Entry::Info(t) => prefixed(wrap(t, body_w), marker(g.info, look.palette.accent), plain),
            Entry::Success(t) => {
                prefixed(wrap(t, body_w), marker(g.success, look.palette.success), plain)
            }
            Entry::Warn(t) => prefixed(wrap(t, body_w), marker(g.warn, look.palette.warn), look.warn()),
            Entry::Error(t) => {
                prefixed(wrap(t, body_w), marker(g.error, look.palette.error), look.error())
            }
            Entry::Block(lines) => lines.clone(),
            Entry::Summary(findings) => scan_ui::summary_lines(findings, look),
            Entry::Scan(report) => scan_ui::report_lines(report, look),
            Entry::ScanAi { judgements, failed } => {
                scan_ui::ai_lines(judgements, failed, look)
            }
            Entry::Card {
                finding,
                fix,
                index,
                total,
            } => {
                let mut out = scan_ui::finding_card(finding, *index, *total, width, look);
                out.push(Line::raw(""));
                out.extend(scan_ui::diff_lines(fix, 3, width, look));
                out
            }
            Entry::Diff { path, old, new } => {
                // 复用扫描界面的 diff 排版：只显示改动和前后几行上下文，
                // 整文件重写时也不会把整份文件刷两遍
                let fix = Fix {
                    finding_id: String::new(),
                    file: path.clone(),
                    old: old.clone(),
                    new: new.clone(),
                };
                scan_ui::diff_lines(&fix, 3, width, look)
            }
            Entry::Assistant { reasoning, content } => {
                let dim = look.muted();
                let mut out = Vec::new();
                if !reasoning.trim().is_empty() {
                    out.push(Line::styled(
                        format!("{} 思考过程", g.thinking),
                        dim.add_modifier(Modifier::ITALIC),
                    ));
                    for row in wrap(reasoning.trim_end(), body_w) {
                        out.push(Line::styled(format!("  {row}"), dim));
                    }
                    if !content.is_empty() {
                        out.push(Line::raw(""));
                    }
                }
                if !content.is_empty() {
                    out.extend(prefixed(
                        wrap(content.trim_end(), body_w),
                        marker(g.info, look.palette.accent),
                        plain,
                    ));
                }
                out
            }
        }
    }
}

/// 一行的纯文本（去掉样式），测试里用来断言内容
#[cfg(test)]
pub fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[derive(Debug, Default)]
pub struct Transcript {
    entries: Vec<Entry>,
}

impl Transcript {
    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// 开始一条新的 AI 回复，后续流式内容追加到它上面
    pub fn start_assistant(&mut self) {
        self.entries.push(Entry::Assistant {
            reasoning: String::new(),
            content: String::new(),
        });
    }

    fn last_assistant(&mut self) -> &mut Entry {
        if !matches!(self.entries.last(), Some(Entry::Assistant { .. })) {
            self.start_assistant();
        }
        self.entries.last_mut().expect("just ensured non-empty")
    }

    pub fn append_content(&mut self, s: &str) {
        if let Entry::Assistant { content, .. } = self.last_assistant() {
            content.push_str(s);
        }
    }

    pub fn append_reasoning(&mut self, s: &str) {
        if let Entry::Assistant { reasoning, .. } = self.last_assistant() {
            reasoning.push_str(s);
        }
    }

    /// 全部条目渲染成行，条目之间空一行；渲染为空的条目（还没收到内容的回复）会被跳过
    #[cfg(test)]
    pub fn lines(&self, width: usize) -> Vec<Line<'static>> {
        self.lines_marked(width, &Look::default()).0
    }

    /// 同 `lines`，另外返回最后一个条目的第一行在结果中的下标（没有内容时为 0）
    pub fn lines_marked(&self, width: usize, look: &Look) -> (Vec<Line<'static>>, usize) {
        let mut out: Vec<Line<'static>> = Vec::new();
        let mut last_start = 0;
        for entry in &self.entries {
            let rows = entry.render(width, look);
            if rows.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push(Line::raw(""));
            }
            last_start = out.len();
            out.extend(rows);
        }
        (out, last_start)
    }
}

/// 对话区的滚动位置。`top` 为 `None` 表示跟随底部；
/// 否则固定在某一行，新内容追加时画面不会移动。
#[derive(Debug, Default)]
pub struct Scroll {
    top: Option<usize>,
    total: usize,
    height: usize,
}

impl Scroll {
    fn max_top(&self) -> usize {
        self.total.saturating_sub(self.height)
    }

    fn current(&self) -> usize {
        self.top.unwrap_or(self.max_top()).min(self.max_top())
    }

    /// 渲染时调用：告知总行数与可见高度，返回应显示的起始行
    pub fn resolve(&mut self, total: usize, height: usize) -> usize {
        self.total = total;
        self.height = height;
        match self.top {
            Some(t) if t < self.max_top() => t,
            _ => {
                self.top = None;
                self.max_top()
            }
        }
    }

    pub fn up(&mut self, n: usize) {
        self.top = Some(self.current().saturating_sub(n));
    }

    pub fn down(&mut self, n: usize) {
        let t = self.current() + n;
        self.top = if t >= self.max_top() { None } else { Some(t) };
    }

    pub fn page(&self) -> usize {
        self.height.saturating_sub(1).max(1)
    }

    pub fn to_top(&mut self) {
        self.top = Some(0);
    }

    pub fn to_bottom(&mut self) {
        self.top = None;
    }

    pub fn is_following(&self) -> bool {
        self.top.is_none()
    }

    /// 把视图固定在第 `top` 行，让某个条目的开头可见；够得到底部时仍然跟随底部。
    /// 依赖 `resolve` 记录的总行数与高度，所以要先调用过 `resolve`。
    pub fn pin(&mut self, top: usize) {
        self.top = if top >= self.max_top() {
            None
        } else {
            Some(top)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn wrap_breaks_english_at_spaces() {
        assert_eq!(wrap("hello world foo", 11), ["hello world", "foo"]);
        assert_eq!(wrap("hello world foo", 8), ["hello", "world", "foo"]);
    }

    #[test]
    fn wrap_hard_breaks_long_words() {
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_counts_cjk_as_two_columns() {
        assert_eq!(wrap("你好世界啊", 6), ["你好世", "界啊"]);
        assert_eq!(wrap("ab你好", 4), ["ab你", "好"]);
    }

    #[test]
    fn wrap_keeps_newlines_and_blank_lines() {
        assert_eq!(wrap("a\n\nb", 10), ["a", "", "b"]);
        assert_eq!(wrap("", 10), [""]);
    }

    #[test]
    fn wrap_never_starts_a_row_with_space_or_exceeds_width() {
        for width in 2..14 {
            let rows = wrap("the quick brown fox 跳过 lazy dogs  and  more", width);
            for r in &rows {
                assert!(text_width(r) <= width, "width {width}: {r:?}");
                assert!(!r.starts_with(' '), "width {width}: {r:?}");
            }
        }
    }

    #[test]
    fn wrap_preserves_leading_indent_of_code() {
        assert_eq!(wrap("    let x = 1;", 20), ["    let x = 1;"]);
    }

    #[test]
    fn user_entry_has_marker_and_indented_continuation() {
        let rows = plain(&Entry::User("hello world".into()).render(8, &Look::default()));
        assert_eq!(rows, ["› hello", "  world"]);
    }

    #[test]
    fn assistant_shows_reasoning_then_content() {
        let e = Entry::Assistant {
            reasoning: "想一想".into(),
            content: "答案".into(),
        };
        let rows = plain(&e.render(40, &Look::default()));
        assert_eq!(rows, ["✻ 思考过程", "  想一想", "", "● 答案"]);
    }

    #[test]
    fn empty_assistant_renders_nothing_and_is_skipped_between_entries() {
        let mut t = Transcript::default();
        t.push(Entry::User("hi".into()));
        t.start_assistant();
        assert_eq!(plain(&t.lines(20)), ["› hi"]);
        t.append_content("hello");
        assert_eq!(plain(&t.lines(20)), ["› hi", "", "● hello"]);
    }

    #[test]
    fn streaming_appends_to_the_last_assistant_entry() {
        let mut t = Transcript::default();
        t.append_reasoning("a");
        t.append_reasoning("b");
        t.append_content("x");
        t.append_content("y");
        t.push(Entry::Info("done".into()));
        t.append_content("z"); // 上一条不是回复，另起一条
        let text = plain(&t.lines(40)).join("\n");
        assert!(text.contains("ab") && text.contains("xy") && text.contains("● z"));
    }

    #[test]
    fn card_entry_renders_card_and_diff_at_the_given_width() {
        let (findings, fixes) = crate::review::demo_data();
        let e = Entry::Card {
            finding: Box::new(findings[0].clone()),
            fix: Box::new(fixes[0].clone()),
            index: 1,
            total: 3,
        };
        for width in [40, 80] {
            let rows = plain(&e.render(width, &Look::default()));
            assert!(rows[0].contains("VG-001") && rows[0].contains("(1/3)"));
            assert!(rows.iter().any(|r| r.contains("│ - API_KEY")));
            assert!(rows.iter().any(|r| r.contains("│ + API_KEY")));
            for r in &rows {
                assert!(text_width(r) <= width, "width {width}: {r:?}");
            }
        }
    }

    #[test]
    fn summary_entry_renders_counts() {
        let (findings, _) = crate::review::demo_data();
        let rows = plain(&Entry::Summary(findings).render(60, &Look::default()));
        assert!(rows[0].contains("共发现 3 个问题"));
    }

    #[test]
    fn scroll_follows_bottom_by_default() {
        let mut s = Scroll::default();
        assert_eq!(s.resolve(100, 10), 90);
        assert!(s.is_following());
        assert_eq!(s.resolve(120, 10), 110);
    }

    #[test]
    fn scroll_up_pins_view_while_content_grows() {
        let mut s = Scroll::default();
        s.resolve(100, 10);
        s.up(5);
        assert_eq!(s.resolve(100, 10), 85);
        assert_eq!(s.resolve(130, 10), 85);
        assert!(!s.is_following());
    }

    #[test]
    fn scroll_down_past_end_resumes_following() {
        let mut s = Scroll::default();
        s.resolve(100, 10);
        s.up(20);
        s.down(10);
        assert_eq!(s.resolve(100, 10), 80);
        s.down(50);
        assert!(s.is_following());
        assert_eq!(s.resolve(100, 10), 90);
    }

    #[test]
    fn scroll_clamps_at_top_and_handles_short_content() {
        let mut s = Scroll::default();
        s.resolve(100, 10);
        s.up(500);
        assert_eq!(s.resolve(100, 10), 0);
        s.to_bottom();
        assert_eq!(s.resolve(5, 10), 0);
        s.up(3);
        assert_eq!(s.resolve(5, 10), 0);
        assert!(s.is_following());
    }

    #[test]
    fn lines_marked_points_at_the_first_row_of_the_last_entry() {
        let mut t = Transcript::default();
        assert_eq!(t.lines_marked(20, &Look::default()).1, 0);
        t.push(Entry::Info("one".into()));
        t.push(Entry::Info("two three four five six".into()));
        let (lines, start) = t.lines_marked(10, &Look::default());
        assert!(plain(&lines)[start].starts_with("● two"));
        // 空的回复条目不算「最后一个条目」
        t.start_assistant();
        assert_eq!(t.lines_marked(10, &Look::default()).1, start);
    }

    #[test]
    fn pin_shows_a_fixed_row_unless_it_is_already_at_the_bottom() {
        let mut s = Scroll::default();
        s.resolve(100, 10);
        s.pin(40);
        assert_eq!(s.resolve(100, 10), 40);
        assert!(!s.is_following());
        s.pin(95); // 超过最大起点：回到跟随底部
        assert!(s.is_following());
        assert_eq!(s.resolve(100, 10), 90);
    }

    #[test]
    fn page_is_height_minus_one() {
        let mut s = Scroll::default();
        s.resolve(100, 10);
        assert_eq!(s.page(), 9);
    }
}
