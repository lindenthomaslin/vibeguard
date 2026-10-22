//! 输入框的编辑状态：纯逻辑，不碰终端，便于单元测试。

/// 斜杠命令补全候选
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub name: &'static str,
    pub desc: &'static str,
}

pub struct Editor {
    buf: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    hist_pos: Option<usize>,
    draft: Vec<char>,
    commands: &'static [Candidate],
    menu_sel: usize,
    cached_matches: (String, Vec<Candidate>),
}

impl Editor {
    pub fn new(commands: &'static [Candidate]) -> Self {
        Self {
            buf: Vec::new(),
            cursor: 0,
            history: Vec::new(),
            hist_pos: None,
            draft: Vec::new(),
            commands,
            menu_sel: 0,
            cached_matches: (String::new(), Vec::new()),
        }
    }

    pub fn chars(&self) -> &[char] {
        &self.buf
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    fn line_start(&self, pos: usize) -> usize {
        let mut i = pos;
        while i > 0 && self.buf[i - 1] != '\n' {
            i -= 1;
        }
        i
    }

    fn line_end(&self, pos: usize) -> usize {
        let mut i = pos;
        while i < self.buf.len() && self.buf[i] != '\n' {
            i += 1;
        }
        i
    }

    fn set(&mut self, s: &str) {
        self.buf = s.chars().collect();
        self.cursor = self.buf.len();
        self.menu_sel = 0;
    }

    // ---- 编辑 ----

    pub fn insert(&mut self, c: char) {
        if c.is_control() && c != '\n' {
            return;
        }
        self.buf.insert(self.cursor, c);
        self.cursor += 1;
        self.menu_sel = 0;
    }

    /// 粘贴：统一换行符，制表符展开为空格
    pub fn insert_str(&mut self, s: &str) {
        let s = s.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ");
        for c in s.chars() {
            self.insert(c);
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.buf.remove(self.cursor);
            self.menu_sel = 0;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.buf.len() {
            self.buf.remove(self.cursor);
            self.menu_sel = 0;
        }
    }

    fn word_start(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && self.buf[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.buf[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    fn word_end(&self) -> usize {
        let mut i = self.cursor;
        while i < self.buf.len() && self.buf[i].is_whitespace() {
            i += 1;
        }
        while i < self.buf.len() && !self.buf[i].is_whitespace() {
            i += 1;
        }
        i
    }

    /// Ctrl-W：删除光标前一个词
    pub fn delete_word_back(&mut self) {
        let start = self.word_start();
        self.buf.drain(start..self.cursor);
        self.cursor = start;
        self.menu_sel = 0;
    }

    /// Ctrl-U：删除到行首
    pub fn kill_to_line_start(&mut self) {
        let start = self.line_start(self.cursor);
        self.buf.drain(start..self.cursor);
        self.cursor = start;
        self.menu_sel = 0;
    }

    /// Ctrl-K：删除到行尾
    pub fn kill_to_line_end(&mut self) {
        let end = self.line_end(self.cursor);
        self.buf.drain(self.cursor..end);
        self.menu_sel = 0;
    }

    // ---- 光标移动 ----

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.buf.len());
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start();
    }

    pub fn word_right(&mut self) {
        self.cursor = self.word_end();
    }

    pub fn home(&mut self) {
        self.cursor = self.line_start(self.cursor);
    }

    pub fn end(&mut self) {
        self.cursor = self.line_end(self.cursor);
    }

    /// 在多行内向上移动；已在第一行时翻到上一条历史
    pub fn up(&mut self) {
        let start = self.line_start(self.cursor);
        if start == 0 {
            self.history_prev();
            return;
        }
        let col = self.cursor - start;
        let prev_start = self.line_start(start - 1);
        let prev_len = start - 1 - prev_start;
        self.cursor = prev_start + col.min(prev_len);
    }

    /// 在多行内向下移动；已在最后一行时翻到下一条历史
    pub fn down(&mut self) {
        let end = self.line_end(self.cursor);
        if end >= self.buf.len() {
            self.history_next();
            return;
        }
        let col = self.cursor - self.line_start(self.cursor);
        let next_start = end + 1;
        let next_len = self.line_end(next_start) - next_start;
        self.cursor = next_start + col.min(next_len);
    }

    // ---- 历史 ----

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.hist_pos {
            None => {
                self.draft = self.buf.clone();
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.hist_pos = Some(next);
        let entry = self.history[next].clone();
        self.set(&entry);
    }

    fn history_next(&mut self) {
        let Some(i) = self.hist_pos else { return };
        if i + 1 < self.history.len() {
            self.hist_pos = Some(i + 1);
            let entry = self.history[i + 1].clone();
            self.set(&entry);
        } else {
            self.hist_pos = None;
            self.buf = std::mem::take(&mut self.draft);
            self.cursor = self.buf.len();
            self.menu_sel = 0;
        }
    }

    // ---- 斜杠命令菜单 ----

    /// 输入以 `/` 开头且还没有空白时，返回匹配的命令
    pub fn matches(&self) -> Vec<Candidate> {
        if self.buf.first() != Some(&'/') || self.buf.iter().any(|c| c.is_whitespace()) {
            return Vec::new();
        }
        let typed = self.text();
        if self.cached_matches.0 == typed {
            return self.cached_matches.1.clone();
        }
        self.commands
            .iter()
            .copied()
            .filter(|c| c.name.starts_with(&typed))
            .collect()
    }

    pub fn menu_open(&self) -> bool {
        !self.matches().is_empty()
    }

    /// 当前高亮项在 `matches()` 中的下标
    pub fn menu_selected(&self) -> usize {
        self.menu_sel.min(self.matches().len().saturating_sub(1))
    }

    /// 菜单选中项上下移动，首尾循环
    pub fn menu_move(&mut self, delta: isize) {
        let n = self.matches().len() as isize;
        if n == 0 {
            return;
        }
        self.menu_sel = (self.menu_selected() as isize + delta).rem_euclid(n) as usize;
    }

    /// Tab：把高亮的命令补全到输入框（末尾带空格，便于继续输入参数）
    pub fn complete(&mut self) {
        let m = self.matches();
        if let Some(c) = m.get(self.menu_selected()) {
            let full = format!("{} ", c.name);
            self.set(&full);
        }
    }

    // ---- 提交 ----

    pub fn text(&self) -> String {
        self.buf.iter().collect()
    }

    pub fn newline(&mut self) {
        self.insert('\n');
    }

    /// Enter：菜单打开且输入还不是完整命令时先补全并返回 `None`；
    /// 否则提交并清空输入框。内容为空时返回 `None`。
    pub fn submit(&mut self) -> Option<String> {
        let m = self.matches();
        if let Some(c) = m.get(self.menu_selected())
            && self.text() != c.name
        {
            self.complete();
            return None;
        }
        let text = self.text().trim().to_string();
        if text.is_empty() {
            return None;
        }
        if self.history.last() != Some(&text) {
            self.history.push(text.clone());
        }
        self.clear();
        Some(text)
    }

    /// 清空输入并退出历史浏览（不影响已保存的历史）
    pub fn clear(&mut self) {
        self.buf.clear();
        self.cursor = 0;
        self.hist_pos = None;
        self.draft.clear();
        self.menu_sel = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMDS: &[Candidate] = &[
        Candidate { name: "/config", desc: "配置" },
        Candidate { name: "/clear", desc: "清空" },
        Candidate { name: "/help", desc: "帮助" },
    ];

    fn ed(s: &str) -> Editor {
        let mut e = Editor::new(CMDS);
        e.insert_str(s);
        e
    }

    #[test]
    fn insert_and_backspace_handle_cjk() {
        let mut e = ed("你好a");
        e.backspace();
        assert_eq!(e.text(), "你好");
        e.left();
        e.insert('，');
        assert_eq!(e.text(), "你，好");
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn control_chars_are_ignored_but_paste_is_normalized() {
        let mut e = Editor::new(CMDS);
        e.insert('\x1b');
        assert!(e.is_empty());
        e.insert_str("a\r\nb\tc");
        assert_eq!(e.text(), "a\nb    c");
    }

    #[test]
    fn word_and_line_kills() {
        let mut e = ed("foo bar baz");
        e.delete_word_back();
        assert_eq!(e.text(), "foo bar ");
        e.kill_to_line_start();
        assert_eq!(e.text(), "");

        let mut e = ed("hello world");
        for _ in 0..5 {
            e.left();
        }
        e.kill_to_line_end();
        assert_eq!(e.text(), "hello ");
    }

    #[test]
    fn word_movement() {
        let mut e = ed("foo bar");
        e.word_left();
        assert_eq!(e.cursor(), 4);
        e.word_left();
        assert_eq!(e.cursor(), 0);
        e.word_right();
        assert_eq!(e.cursor(), 3);
        e.word_right();
        assert_eq!(e.cursor(), 7);
    }

    #[test]
    fn multiline_cursor_moves_and_clamps_column() {
        let mut e = ed("abcd\nab\nabcdef");
        e.up();
        assert_eq!(e.cursor(), 7); // 第二行较短，列被截断到行尾
        e.up();
        assert_eq!(e.cursor(), 2);
        e.down();
        assert_eq!(e.cursor(), 7);
        e.down();
        assert_eq!(e.cursor(), 10);
        e.home();
        assert_eq!(e.cursor(), 8);
        e.end();
        assert_eq!(e.cursor(), 14);
    }

    #[test]
    fn history_walk_restores_draft() {
        let mut e = ed("first");
        assert_eq!(e.submit().as_deref(), Some("first"));
        e.insert_str("second");
        assert_eq!(e.submit().as_deref(), Some("second"));

        e.insert_str("draft");
        e.up();
        assert_eq!(e.text(), "second");
        e.up();
        assert_eq!(e.text(), "first");
        e.up();
        assert_eq!(e.text(), "first");
        e.down();
        assert_eq!(e.text(), "second");
        e.down();
        assert_eq!(e.text(), "draft");
    }

    #[test]
    fn duplicate_submit_is_not_stored_twice() {
        let mut e = ed("same");
        e.submit();
        e.insert_str("same");
        e.submit();
        e.up();
        e.up();
        assert_eq!(e.text(), "same");
        assert_eq!(e.history.len(), 1);
    }

    #[test]
    fn empty_submit_returns_none() {
        let mut e = ed("   ");
        assert_eq!(e.submit(), None);
    }

    #[test]
    fn menu_filters_and_only_opens_for_command_prefix() {
        assert!(!ed("hello").menu_open());
        assert!(!ed("/config set").menu_open());
        let names: Vec<_> = ed("/c").matches().iter().map(|c| c.name).collect();
        assert_eq!(names, ["/config", "/clear"]);
        assert_eq!(ed("/").matches().len(), 3);
        assert!(!ed("/zzz").menu_open());
    }

    #[test]
    fn menu_selection_wraps_and_tab_completes() {
        let mut e = ed("/c");
        e.menu_move(1);
        assert_eq!(e.menu_selected(), 1);
        e.menu_move(1);
        assert_eq!(e.menu_selected(), 0);
        e.menu_move(-1);
        e.complete();
        assert_eq!(e.text(), "/clear ");
        assert!(!e.menu_open());
    }

    #[test]
    fn enter_completes_partial_command_then_submits() {
        let mut e = ed("/he");
        assert_eq!(e.submit(), None);
        assert_eq!(e.text(), "/help ");
        assert_eq!(e.submit().as_deref(), Some("/help"));
        assert!(e.is_empty());
    }

    #[test]
    fn enter_submits_exact_command_without_completing() {
        let mut e = ed("/help");
        assert_eq!(e.submit().as_deref(), Some("/help"));
    }
}