//! 历史对话面板：居中的浮层，上面一行搜索框，下面是列表。
//!
//! 过滤全在内存里做：打开面板时把每个对话的正文开头带进来（见
//! `sessions::Store::catalog`），之后每敲一个字只比一次字符串，不碰磁盘。
//! 这样输入不会卡，也不需要在按键路径上做防抖。

use crate::editor::Editor;
use crate::i18n::pick;
use crate::input::handle_key;
use crate::sessions::{CatalogEntry, find_folded, fold, snippet};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// 面板里的一行
#[derive(Debug, Clone)]
pub struct SessionItem {
    pub id: String,
    /// 左列：相对时间
    pub when: String,
    pub title: String,
    pub turns: usize,
    /// 正文开头一段，用来生成命中摘要
    body: String,
    title_key: Vec<char>,
    body_key: Vec<char>,
}

impl SessionItem {
    pub fn from_catalog(entry: CatalogEntry, when: String) -> Self {
        Self {
            id: entry.summary.id,
            when,
            title: entry.summary.title.clone(),
            turns: entry.summary.turns,
            title_key: fold(&entry.summary.title),
            body_key: fold(&entry.body),
            body: entry.body,
        }
    }

    /// 标题或正文里有这个词就算命中
    fn matches(&self, needle: &[char]) -> bool {
        find_folded(&self.title_key, needle).is_some()
            || find_folded(&self.body_key, needle).is_some()
    }

    /// 只命中正文时给一段摘要，标题命中就不用了（标题本身已经够说明问题）。
    /// 摘要里如果只是把标题又念了一遍也不给：标题本来就取自第一句话。
    pub fn snippet(&self, needle: &[char]) -> Option<String> {
        if needle.is_empty() || find_folded(&self.title_key, needle).is_some() {
            return None;
        }
        let at = find_folded(&self.body_key, needle)?;
        let text = snippet(&self.body, at, needle.len());
        if find_folded(&fold(&text), &self.title_key).is_some() {
            return None;
        }
        Some(text)
    }
}

/// 页脚的一行提示：刚删完、或者删除失败
pub struct Notice {
    pub text: String,
    pub error: bool,
}

/// 按键的结果，交给 `AppState` 翻译成 `Action`
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Close,
    Resume(String),
    Delete(String),
}

pub struct SessionsPanel {
    items: Vec<SessionItem>,
    /// 过滤之后还留在列表里的下标
    view: Vec<usize>,
    query: Editor,
    selected: usize,
    /// 按过一次 Del、等着第二次确认的对话编号
    pending: Option<String>,
    notice: Option<Notice>,
}

impl SessionsPanel {
    /// 列表为空时返回 `None`：没什么可选的就别弹窗
    pub fn new(items: Vec<SessionItem>) -> Option<Self> {
        if items.is_empty() {
            return None;
        }
        let mut panel = Self {
            items,
            view: Vec::new(),
            query: Editor::new(&[]),
            selected: 0,
            pending: None,
            notice: None,
        };
        panel.refilter(false);
        Some(panel)
    }

    /// 带初始关键词打开（`/search 关键词`）
    pub fn with_query(items: Vec<SessionItem>, query: &str) -> Option<Self> {
        let mut panel = Self::new(items)?;
        if !query.trim().is_empty() {
            panel.query.insert_str(query.trim());
            panel.refilter(true);
        }
        Some(panel)
    }

    pub fn total(&self) -> usize {
        self.items.len()
    }

    /// 边框上的标题：过滤时把「还剩几个」也写出来
    pub fn title(&self) -> String {
        if self.matched() == self.items.len() {
            pick(
                format!("历史对话 {}", self.items.len()),
                format!("Conversation {}", self.items.len()),
            )
        } else {
            pick(
                format!("历史对话 {}/{}", self.matched(), self.items.len()),
                format!("Conversation {}/{}", self.matched(), self.items.len()),
            )
        }
    }

    pub fn matched(&self) -> usize {
        self.view.len()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// 搜索框里的文字和光标
    pub fn query(&self) -> (&[char], usize) {
        (self.query.chars(), self.query.cursor())
    }

    /// 已经折叠成小写的关键词，用来给列表项取摘要
    pub fn needle(&self) -> Vec<char> {
        fold(self.query.text().trim())
    }

    /// 过滤后可见的行，按显示顺序
    pub fn visible(&self) -> impl Iterator<Item = &SessionItem> {
        self.view.iter().map(|&i| &self.items[i])
    }

    pub fn pending(&self) -> Option<&SessionItem> {
        let id = self.pending.as_deref()?;
        self.items.iter().find(|i| i.id == id)
    }

    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    pub fn set_notice(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            error,
        });
    }

    /// 删除成功后把这一行从列表里拿掉，选中项尽量留在原地
    pub fn forget(&mut self, id: &str) {
        let Some(pos) = self.items.iter().position(|i| i.id == id) else {
            return;
        };
        let title = self.items[pos].title.clone();
        self.items.remove(pos);
        self.pending = None;
        self.notice = Some(Notice {
            text: pick(format!("已删除「{title}」"), format!("Deleted \"{title}\"")),
            error: false,
        });
        self.refilter(true);
    }

    pub fn paste(&mut self, text: &str) {
        self.query.insert_str(text.trim_end_matches(['\r', '\n']));
        self.refilter(true);
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Event {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // 等着确认删除：只认第二次 Del，别的键都算反悔
        if self.pending.is_some() {
            match key.code {
                KeyCode::Delete => {
                    let id = self.pending.take().unwrap_or_default();
                    return Event::Delete(id);
                }
                KeyCode::Esc => {
                    self.pending = None;
                    self.notice = None;
                    return Event::None;
                }
                _ => self.pending = None,
            }
        }

        match key.code {
            KeyCode::Esc => return Event::Close,
            KeyCode::Char('c') if ctrl => return Event::Close,
            KeyCode::Enter => {
                return match self.view.get(self.selected) {
                    Some(&i) => Event::Resume(self.items[i].id.clone()),
                    None => Event::None,
                };
            }
            KeyCode::Up => {
                self.move_by(-1);
                return Event::None;
            }
            KeyCode::Down | KeyCode::Tab => {
                self.move_by(1);
                return Event::None;
            }
            // 搜索框用不着向后删字符，这个键留给「删掉选中的对话」
            KeyCode::Delete => {
                if let Some(&i) = self.view.get(self.selected) {
                    self.pending = Some(self.items[i].id.clone());
                    self.notice = None;
                }
                return Event::None;
            }
            KeyCode::Char('d') if ctrl => {
                if let Some(&i) = self.view.get(self.selected) {
                    self.pending = Some(self.items[i].id.clone());
                    self.notice = None;
                }
                return Event::None;
            }
            _ => {}
        }

        // 其余按键都归搜索框：输入、退格、左右、Home/End、Ctrl-U/K/W
        let before = self.query.text();
        handle_key(&mut self.query, key);
        if self.query.text() != before {
            self.refilter(false);
        }
        Event::None
    }

    fn move_by(&mut self, delta: isize) {
        let n = self.view.len();
        if n == 0 {
            self.selected = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.rem_euclid(n as isize) as usize;
    }

    /// 重新算一遍可见行。`keep` 为真时尽量停在原来的位置（删完一行之后），
    /// 否则回到第一行（刚改了关键词）。
    fn refilter(&mut self, keep: bool) {
        let needle = self.needle();
        self.view = if needle.is_empty() {
            (0..self.items.len()).collect()
        } else {
            self.items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.matches(&needle))
                .map(|(i, _)| i)
                .collect()
        };
        self.selected = if keep {
            self.selected.min(self.view.len().saturating_sub(1))
        } else {
            0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::Summary;

    fn item(id: &str, title: &str, body: &str) -> SessionItem {
        SessionItem::from_catalog(
            CatalogEntry {
                summary: Summary {
                    id: id.into(),
                    title: title.into(),
                    updated: 0,
                    turns: 1,
                    model: "m".into(),
                },
                body: body.into(),
            },
            "刚刚".into(),
        )
    }

    fn items() -> Vec<SessionItem> {
        vec![
            item("a", "如何防止 SQL 注入", "使用参数化查询，别拼字符串"),
            item("b", "聊聊天气", "今天不错，适合出门"),
            item("c", "XSS 修复", "对输出做转义"),
        ]
    }

    fn panel() -> SessionsPanel {
        SessionsPanel::new(items()).unwrap()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_str(p: &mut SessionsPanel, s: &str) {
        for c in s.chars() {
            p.on_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn empty_list_does_not_open() {
        assert!(SessionsPanel::new(Vec::new()).is_none());
    }

    #[test]
    fn typing_filters_by_title_and_by_body() {
        let mut p = panel();
        assert_eq!(p.matched(), 3);

        type_str(&mut p, "sql");
        assert_eq!(p.matched(), 1, "标题里就有 SQL");
        assert_eq!(p.visible().next().unwrap().id, "a");

        p.on_key(ctrl('u')); // 清空
        type_str(&mut p, "转义");
        assert_eq!(p.matched(), 1, "只出现在正文里也算");
        assert_eq!(p.visible().next().unwrap().id, "c");
        let snip = p.visible().next().unwrap().snippet(&p.needle()).unwrap();
        assert!(snip.contains("转义"), "{snip}");

        type_str(&mut p, "zzz");
        assert_eq!(p.matched(), 0);
    }

    #[test]
    fn filtering_is_case_insensitive_and_keeps_the_store_order() {
        let mut p = panel();
        type_str(&mut p, "SQL");
        assert_eq!(p.visible().next().unwrap().id, "a");
        p.on_key(ctrl('u'));
        assert_eq!(p.matched(), 3, "清空关键词恢复全部");
        let ids: Vec<_> = p.visible().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[test]
    fn arrows_wrap_and_enter_resumes_the_highlighted_one() {
        let mut p = panel();
        p.on_key(key(KeyCode::Down));
        assert_eq!(p.selected(), 1);
        p.on_key(key(KeyCode::Up));
        p.on_key(key(KeyCode::Up));
        assert_eq!(p.selected(), 2, "从第一项往上绕到最后");
        assert_eq!(p.on_key(key(KeyCode::Enter)), Event::Resume("c".into()));
    }

    #[test]
    fn esc_closes_and_typing_goes_into_the_search_box_not_the_transcript() {
        let mut p = panel();
        type_str(&mut p, "天气");
        assert_eq!(p.query().0.iter().collect::<String>(), "天气");
        assert_eq!(p.on_key(key(KeyCode::Esc)), Event::Close);
        assert_eq!(p.on_key(ctrl('c')), Event::Close, "Ctrl-C 也是退出面板");
    }

    #[test]
    fn deleting_needs_two_presses_and_esc_cancels_the_question() {
        let mut p = panel();
        p.on_key(key(KeyCode::Down));
        assert_eq!(p.on_key(key(KeyCode::Delete)), Event::None);
        assert_eq!(p.pending().unwrap().id, "b");
        // 反悔：Esc 只取消这次删除，不关面板
        assert_eq!(p.on_key(key(KeyCode::Esc)), Event::None);
        assert!(p.pending().is_none());
        // 再来一次，第二次 Del 才真的删
        p.on_key(key(KeyCode::Delete));
        assert_eq!(p.on_key(key(KeyCode::Delete)), Event::Delete("b".into()));
    }

    #[test]
    fn typing_while_the_delete_question_is_open_counts_as_backing_out() {
        let mut p = panel();
        p.on_key(key(KeyCode::Delete));
        type_str(&mut p, "天");
        assert!(p.pending().is_none(), "按了别的键就不删了");
        assert_eq!(p.matched(), 1, "而且这个字进了搜索框");
    }

    #[test]
    fn forget_drops_the_row_and_keeps_the_selection_nearby() {
        let mut p = panel();
        p.on_key(key(KeyCode::Down));
        p.on_key(key(KeyCode::Down));
        assert_eq!(p.selected(), 2);
        p.forget("c");
        assert_eq!(p.total(), 2);
        assert_eq!(p.matched(), 2);
        assert_eq!(p.selected(), 1, "删掉最后一行，选中项往上收一格");
        assert!(p.notice().unwrap().text.contains("已删除"));
    }

    #[test]
    fn opening_with_a_query_presets_the_search_box() {
        let p = SessionsPanel::with_query(items(), "xss").unwrap();
        assert_eq!(p.query().0.iter().collect::<String>(), "xss");
        assert_eq!(p.matched(), 1);
        assert_eq!(p.visible().next().unwrap().id, "c");
    }

    #[test]
    fn paste_goes_into_the_search_box() {
        let mut p = panel();
        p.paste("天气\n");
        assert_eq!(p.query().0.iter().collect::<String>(), "天气");
        assert_eq!(p.matched(), 1);
    }

    #[test]
    fn ctrl_d_also_asks_to_delete() {
        let mut p = panel();
        assert_eq!(p.on_key(ctrl('d')), Event::None);
        assert_eq!(p.pending().unwrap().id, "a");
    }

    #[test]
    fn an_empty_result_still_closes_cleanly() {
        let mut p = panel();
        type_str(&mut p, "zzz");
        assert_eq!(p.on_key(key(KeyCode::Enter)), Event::None, "没东西可打开");
        assert_eq!(p.on_key(key(KeyCode::Delete)), Event::None, "也没东西可删");
        assert!(p.pending().is_none());
    }
}
