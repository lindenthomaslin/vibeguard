//! 全屏界面的应用状态：处理按键与后台事件，不碰终端，便于测试。

use super::form::{FormEvent, ModelForm, ModelSettings};
use super::view::Choice;
use crate::chat::{Entry, Scroll, Transcript};
use crate::editor::{Candidate, Editor};
use crate::finding::{Finding, Fix};
use crate::input::{Outcome, handle_key};
use crate::review;
use crate::review::{Decision, Reviewer};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// 输入 `/` 时弹出的命令菜单
pub const COMMANDS: &[Candidate] = &[
    Candidate { name: "/model", desc: "配置 API 地址、API Key、模型与思考力度" },
    Candidate { name: "/permissions", desc: "配置权限模式和工作目录" },
    Candidate { name: "/resume", desc: "恢复上一次对话" },
    Candidate { name: "/sessions", desc: "选择历史对话" },
    Candidate { name: "/search", desc: "搜索历史对话：/search 关键词" },
    Candidate { name: "/clear", desc: "开始新对话（旧对话会保留）" },
    Candidate { name: "/thinking", desc: "查看或切换思考模式（仅本次）" },
    Candidate { name: "/config", desc: "查看或修改配置" },
    Candidate { name: "/help", desc: "显示帮助" },
    Candidate { name: "/demo", desc: "预览扫描与修复界面（示例数据）" },
    Candidate { name: "/scan", desc: "扫描漏洞（开发中）" },
    Candidate { name: "/fix", desc: "修复漏洞（开发中）" },
    Candidate { name: "/report", desc: "生成报告（开发中）" },
    Candidate { name: "/exit", desc: "退出" },
];

const REVIEW_HINT: &str = "↑↓ 选择 · Enter 确认 · y 应用 · n 跳过 · a 全部应用 · q 结束";
const SESSIONS_HINT: &str = "↑↓ 选择 · Enter 恢复 · Esc 取消";

/// 按键处理后，主循环需要执行的动作
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    /// 用户发送了普通文本，交给 AI
    Chat(String),
    /// 用户输入了斜杠命令（不含 `/`），拆成词
    Command(Vec<String>),
    /// 用户取消了正在进行的 AI 请求
    Cancel,
    /// 模型表单点了保存：设置已通过格式检查。写入配置文件由调用方完成，
    /// 成功后调用 `close_form`；写入失败时表单保持打开，改动不会丢
    SaveModel(ModelSettings),
    /// 对话选择器确认：要恢复的对话编号
    ResumeSession(String),
}

/// 逐条确认的选项，顺序与 `Decision` 对应
const REVIEW_OPTIONS: [(&str, &str, Decision); 4] = [
    ("应用", "写入这个修复", Decision::Apply),
    ("跳过", "保持原样，看下一条", Decision::Skip),
    ("应用此条及之后全部", "不再逐条询问", Decision::ApplyRest),
    ("结束确认", "已选中的保留，其余不修复", Decision::Quit),
];

pub struct ReviewState {
    reviewer: Reviewer,
    selected: usize,
}

/// 对话选择器里的一项
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionItem {
    pub id: String,
    /// 左列：相对时间
    pub when: String,
    /// 右列：标题与说明
    pub text: String,
}

/// 弹出的对话选择器（`/sessions` 和 `/search` 共用）
struct SessionPicker {
    title: String,
    items: Vec<SessionItem>,
    selected: usize,
}

impl SessionPicker {
    fn choice(&self) -> Choice {
        Choice {
            title: format!("{} ({}/{})", self.title, self.selected + 1, self.items.len()),
            items: self
                .items
                .iter()
                .map(|i| (i.when.clone(), i.text.clone()))
                .collect(),
            selected: self.selected,
        }
    }
}

pub struct AppState {
    pub transcript: Transcript,
    pub scroll: Scroll,
    pub editor: Editor,
    pub review: Option<ReviewState>,
    /// 正在等待 AI 时的提示；`Some` 表示忙碌，此时不接受新的发送
    pub busy: Option<String>,
    pub tick: usize,
    interrupted: bool,
    picker: Option<SessionPicker>,
    /// `/model` 表单；打开时键盘输入都交给它
    form: Option<ModelForm>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            transcript: Transcript::default(),
            scroll: Scroll::default(),
            editor: Editor::new(COMMANDS),
            review: None,
            busy: None,
            tick: 0,
            interrupted: false,
            picker: None,
            form: None,
        }
    }

    // ---- 选择区 ----

    /// 当前应显示的选择区：确认流程优先，其次是对话选择器、模型表单和斜杠命令菜单
    pub fn choice(&self) -> Option<Choice> {
        if let Some(r) = &self.review {
            return Some(Choice {
                title: "应用这个修复？".into(),
                items: REVIEW_OPTIONS
                    .iter()
                    .map(|&(n, d, _)| (n.to_string(), d.to_string()))
                    .collect(),
                selected: r.selected,
            });
        }
        if let Some(p) = &self.picker {
            return Some(p.choice());
        }
        if let Some(f) = &self.form {
            return Some(f.choice());
        }
        let matches = self.editor.matches();
        if matches.is_empty() {
            return None;
        }
        Some(Choice {
            title: "命令".into(),
            items: matches
                .iter()
                .map(|c| (c.name.to_string(), c.desc.to_string()))
                .collect(),
            selected: self.editor.menu_selected(),
        })
    }

    // ---- 底部输入框 ----

    /// 输入框里显示的文字和光标：表单打开时是当前正在改的那一项
    pub fn input(&self) -> (&[char], usize) {
        match &self.form {
            Some(f) => f.input(),
            None => (self.editor.chars(), self.editor.cursor()),
        }
    }

    /// 输入框边框上的标签（表单里正在改哪一项）
    pub fn input_label(&self) -> Option<&'static str> {
        self.form.as_ref().and_then(ModelForm::label)
    }

    /// 输入框当前能不能输入：确认、选择器打开时，或表单停在没有文本的行上时不能
    pub fn input_active(&self) -> bool {
        if self.review.is_some() || self.picker.is_some() {
            return false;
        }
        match &self.form {
            Some(f) => f.label().is_some(),
            None => true,
        }
    }

    // ---- 对话内容 ----

    pub fn push(&mut self, entry: Entry) {
        self.transcript.push(entry);
        self.scroll.to_bottom();
    }

    /// 开始逐条确认：先显示汇总，再显示第一条
    pub fn start_review(&mut self, findings: &[Finding], fixes: &[Fix]) {
        self.push(Entry::Summary(findings.to_vec()));
        let reviewer = Reviewer::new(findings, fixes);
        if reviewer.total() == 0 {
            self.push(Entry::Info("没有可确认的修复".into()));
            return;
        }
        self.review = Some(ReviewState { reviewer, selected: 0 });
        self.show_current_review();
    }

    fn show_current_review(&mut self) {
        let Some(state) = &self.review else { return };
        if let Some((finding, fix, index, total)) = state.reviewer.current() {
            // 卡片在渲染时按当前宽度排版，窗口缩放后也不会错位
            let entry = Entry::Card {
                finding: Box::new(finding.clone()),
                fix: Box::new(fix.clone()),
                index,
                total,
            };
            self.push(entry);
        }
    }

    fn answer_review(&mut self, decision: Decision) {
        let Some(state) = &mut self.review else { return };
        state.reviewer.answer(decision);
        state.selected = 0;
        if state.reviewer.current().is_some() {
            self.show_current_review();
            return;
        }
        let Some(state) = self.review.take() else { return };
        let accepted = state.reviewer.into_accepted();
        if accepted.is_empty() {
            self.push(Entry::Info("没有选中任何修复".into()));
        } else {
            let names: Vec<_> = accepted.iter().map(|f| f.finding_id.as_str()).collect();
            self.push(Entry::Success(format!(
                "已选中 {} 个修复：{}（演示模式，未写入文件）",
                accepted.len(),
                names.join("、")
            )));
        }
    }

    // ---- 按键 ----

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // 滚动：对话区的快捷键，任何时候都可用
        match key.code {
            KeyCode::PageUp => {
                self.scroll.up(self.scroll.page());
                return Action::None;
            }
            KeyCode::PageDown => {
                self.scroll.down(self.scroll.page());
                return Action::None;
            }
            KeyCode::Char('b') if ctrl => {
                self.scroll.to_top();
                return Action::None;
            }
            KeyCode::Char('g') if ctrl => {
                self.scroll.to_bottom();
                return Action::None;
            }
            _ => {}
        }

        if self.review.is_some() {
            return self.on_review_key(key);
        }
        if self.picker.is_some() {
            return self.on_picker_key(key);
        }
        if self.form.is_some() {
            return self.on_form_key(key);
        }
        if self.busy.is_some() {
            // 忙碌时只响应取消；输入框仍可编辑，方便先写下一条
            if key.code == KeyCode::Esc || (ctrl && key.code == KeyCode::Char('c')) {
                return Action::Cancel;
            }
        }

        match handle_key(&mut self.editor, key) {
            Outcome::Continue => {
                self.interrupted = false;
                Action::None
            }
            Outcome::Submit(text) => {
                self.interrupted = false;
                if self.busy.is_some() {
                    // 忙碌时不发送，把内容还给输入框
                    self.editor.insert_str(&text);
                    return Action::None;
                }
                match text.strip_prefix('/') {
                    Some(cmd) => {
                        let parts: Vec<String> =
                            cmd.split_whitespace().map(String::from).collect();
                        if matches!(parts.first().map(String::as_str), Some("exit" | "quit" | "q")) {
                            return Action::Quit;
                        }
                        Action::Command(parts)
                    }
                    None => Action::Chat(text),
                }
            }
            Outcome::Eof => Action::Quit,
            Outcome::Interrupt => {
                if self.interrupted {
                    return Action::Quit;
                }
                self.interrupted = true;
                self.push(Entry::Info("再按一次 Ctrl-C 退出".into()));
                Action::None
            }
        }
    }

    fn on_review_key(&mut self, key: KeyEvent) -> Action {
        let Some(state) = &mut self.review else {
            return Action::None;
        };
        let n = REVIEW_OPTIONS.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => state.selected = (state.selected + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                state.selected = (state.selected + 1) % n
            }
            KeyCode::Char('y') => self.answer_review(Decision::Apply),
            KeyCode::Char('n') => self.answer_review(Decision::Skip),
            KeyCode::Char('a') => self.answer_review(Decision::ApplyRest),
            KeyCode::Char('q') | KeyCode::Esc => self.answer_review(Decision::Quit),
            KeyCode::Enter => {
                let decision = REVIEW_OPTIONS[state.selected].2;
                self.answer_review(decision);
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.answer_review(Decision::Quit)
            }
            _ => {}
        }
        Action::None
    }

    // ---- 对话选择器 ----

    /// 打开对话选择器。列表为空时不打开。
    pub fn open_sessions(&mut self, title: &str, items: Vec<SessionItem>) {
        if items.is_empty() {
            return;
        }
        self.picker = Some(SessionPicker {
            title: title.to_string(),
            items,
            selected: 0,
        });
    }

    fn on_picker_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(p) = self.picker.as_mut() else {
            return Action::None;
        };
        let len = p.items.len();
        match key.code {
            KeyCode::Up => p.selected = (p.selected + len - 1) % len,
            KeyCode::Down | KeyCode::Tab => p.selected = (p.selected + 1) % len,
            KeyCode::Home => p.selected = 0,
            KeyCode::End => p.selected = len - 1,
            KeyCode::Enter => {
                let id = p.items[p.selected].id.clone();
                self.picker = None;
                return Action::ResumeSession(id);
            }
            KeyCode::Esc | KeyCode::Char('q') => self.picker = None,
            KeyCode::Char('c') if ctrl => self.picker = None,
            _ => {}
        }
        Action::None
    }

    // ---- 模型表单 ----

    /// 打开 `/model` 表单，初始值是调用方传入的当前设置
    pub fn open_model_form(&mut self, settings: &ModelSettings) {
        self.editor.clear();
        self.form = Some(ModelForm::new(settings));
    }

    /// 保存成功后关闭表单
    pub fn close_form(&mut self) {
        self.form = None;
    }

    fn on_form_key(&mut self, key: KeyEvent) -> Action {
        let Some(form) = self.form.as_mut() else {
            return Action::None;
        };
        match form.on_key(key) {
            FormEvent::None => Action::None,
            FormEvent::Cancel => {
                self.form = None;
                self.push(Entry::Info("已取消，配置没有改动".into()));
                Action::None
            }
            FormEvent::Save(settings) => Action::SaveModel(settings),
            FormEvent::Invalid(msg) => {
                self.push(Entry::Error(msg));
                Action::None
            }
        }
    }

    /// 粘贴：确认和选择器打开时忽略；表单打开时进当前文本项，否则进输入框
    pub fn on_paste(&mut self, text: &str) {
        if self.review.is_some() || self.picker.is_some() {
            return;
        }
        match self.form.as_mut() {
            Some(form) => form.paste(text),
            None => self.editor.insert_str(text),
        }
    }

    /// 当前模式专属的快捷键提示；没有则用默认提示
    pub fn hint(&self) -> Option<&'static str> {
        if self.review.is_some() {
            return Some(REVIEW_HINT);
        }
        if self.picker.is_some() {
            return Some(SESSIONS_HINT);
        }
        self.form.as_ref().map(ModelForm::hint)
    }

    // ---- AI 流式事件 ----

    pub fn begin_request(&mut self, input: &str) {
        self.push(Entry::User(input.to_string()));
        self.transcript.start_assistant();
        self.busy = Some("等待响应…".into());
    }

    pub fn on_content(&mut self, s: &str) {
        self.transcript.append_content(s);
        self.busy = Some("生成中…".into());
    }

    pub fn on_reasoning(&mut self, s: &str) {
        self.transcript.append_reasoning(s);
        self.busy = Some("思考中…".into());
    }

    pub fn end_request(&mut self) {
        self.busy = None;
    }

    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }
}

/// 演示：用示例数据走一遍扫描与修复界面
pub fn start_demo(app: &mut AppState) {
    let (findings, fixes) = review::demo_data();
    app.push(Entry::Warn("演示模式：以下是示例数据，不会读取或修改任何文件".into()));
    app.start_review(&findings, &fixes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Thinking;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_str(app: &mut AppState, s: &str) {
        for c in s.chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
    }

    fn texts(app: &AppState) -> String {
        app.transcript
            .lines(80)
            .iter()
            .map(crate::chat::line_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn settings(api_key: Option<&str>) -> ModelSettings {
        ModelSettings {
            base_url: "https://api.deepseek.com".into(),
            api_key: api_key.map(String::from),
            model: "deepseek-flash".into(),
            thinking: Thinking::Auto,
        }
    }

    fn session_items() -> Vec<SessionItem> {
        let item = |id: &str| SessionItem {
            id: id.into(),
            when: "刚刚".into(),
            text: format!("对话 {id}"),
        };
        vec![item("s1"), item("s2")]
    }

    #[test]
    fn plain_text_becomes_a_chat_action() {
        let mut app = AppState::new();
        type_str(&mut app, "你好");
        assert_eq!(app.on_key(key(KeyCode::Enter)), Action::Chat("你好".into()));
    }

    #[test]
    fn slash_input_becomes_a_command_with_args() {
        let mut app = AppState::new();
        type_str(&mut app, "/model deepseek-chat");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::Command(vec!["model".into(), "deepseek-chat".into()])
        );
    }

    #[test]
    fn partial_command_is_completed_before_it_runs() {
        let mut app = AppState::new();
        type_str(&mut app, "/he");
        assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
        assert_eq!(app.editor.text(), "/help ");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::Command(vec!["help".into()])
        );
    }

    #[test]
    fn slash_opens_command_choice_and_it_closes_after_a_space() {
        let mut app = AppState::new();
        type_str(&mut app, "/");
        let c = app.choice().unwrap();
        assert_eq!(c.title, "命令");
        assert_eq!(c.items.len(), COMMANDS.len());
        type_str(&mut app, "help ");
        assert!(app.choice().is_none());
    }

    #[test]
    fn exit_commands_quit_instead_of_reaching_the_command_runner() {
        for cmd in ["/exit", "/quit", "/q"] {
            let mut app = AppState::new();
            type_str(&mut app, cmd);
            assert_eq!(app.on_key(key(KeyCode::Enter)), Action::Quit, "{cmd}");
        }
    }

    #[test]
    fn ctrl_d_quits_and_double_ctrl_c_quits() {
        let mut app = AppState::new();
        assert_eq!(app.on_key(ctrl('d')), Action::Quit);

        let mut app = AppState::new();
        assert_eq!(app.on_key(ctrl('c')), Action::None);
        assert!(texts(&app).contains("再按一次 Ctrl-C"));
        assert_eq!(app.on_key(ctrl('c')), Action::Quit);
    }

    #[test]
    fn typing_between_ctrl_c_presses_resets_the_quit_warning() {
        let mut app = AppState::new();
        app.on_key(ctrl('c'));
        type_str(&mut app, "a");
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.on_key(ctrl('c')), Action::None);
    }

    #[test]
    fn busy_blocks_sending_and_returns_text_to_the_input() {
        let mut app = AppState::new();
        app.begin_request("first");
        type_str(&mut app, "second");
        assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
        assert_eq!(app.editor.text(), "second");
    }

    #[test]
    fn busy_esc_and_ctrl_c_cancel() {
        let mut app = AppState::new();
        app.begin_request("q");
        assert_eq!(app.on_key(key(KeyCode::Esc)), Action::Cancel);
        assert_eq!(app.on_key(ctrl('c')), Action::Cancel);
    }

    #[test]
    fn streaming_events_build_one_assistant_entry_and_update_status() {
        let mut app = AppState::new();
        app.begin_request("hi");
        assert_eq!(app.busy.as_deref(), Some("等待响应…"));
        app.on_reasoning("想");
        assert_eq!(app.busy.as_deref(), Some("思考中…"));
        app.on_content("答");
        app.on_content("案");
        assert_eq!(app.busy.as_deref(), Some("生成中…"));
        app.end_request();
        assert!(app.busy.is_none());
        let t = texts(&app);
        assert!(t.contains("› hi") && t.contains("思考过程") && t.contains("● 答案"));
    }

    #[test]
    fn page_keys_scroll_even_while_typing() {
        let mut app = AppState::new();
        for i in 0..50 {
            app.push(Entry::Info(format!("m{i}")));
        }
        app.scroll.resolve(100, 10);
        assert!(app.scroll.is_following());
        app.on_key(key(KeyCode::PageUp));
        assert!(!app.scroll.is_following());
        app.on_key(ctrl('g'));
        assert!(app.scroll.is_following());
        app.on_key(ctrl('b'));
        assert!(!app.scroll.is_following());
        assert!(app.editor.is_empty());
    }

    #[test]
    fn new_content_jumps_back_to_the_bottom() {
        let mut app = AppState::new();
        app.scroll.resolve(100, 10);
        app.scroll.up(10);
        app.push(Entry::Info("x".into()));
        assert!(app.scroll.is_following());
    }

    #[test]
    fn demo_walks_through_review_with_keys() {
        let mut app = AppState::new();
        start_demo(&mut app);
        assert!(app.review.is_some());
        let c = app.choice().unwrap();
        assert_eq!(c.title, "应用这个修复？");
        assert_eq!(c.items.len(), 4);
        assert!(texts(&app).contains("VG-001"));

        app.on_key(key(KeyCode::Enter)); // 应用 VG-001
        assert!(texts(&app).contains("VG-002"));
        app.on_key(key(KeyCode::Char('n'))); // 跳过 VG-002
        assert!(texts(&app).contains("VG-003"));
        app.on_key(key(KeyCode::Char('y'))); // 应用 VG-003
        assert!(app.review.is_none());
        assert!(app.choice().is_none());
        let t = texts(&app);
        assert!(t.contains("已选中 2 个修复") && t.contains("VG-001、VG-003"));
    }

    #[test]
    fn review_menu_navigation_wraps_and_enter_uses_selection() {
        let mut app = AppState::new();
        start_demo(&mut app);
        app.on_key(key(KeyCode::Up)); // 从第一项绕到最后一项「结束确认」
        assert_eq!(app.choice().unwrap().selected, 3);
        app.on_key(key(KeyCode::Enter));
        assert!(app.review.is_none());
        assert!(texts(&app).contains("没有选中任何修复"));
    }

    #[test]
    fn apply_rest_finishes_the_review() {
        let mut app = AppState::new();
        start_demo(&mut app);
        app.on_key(key(KeyCode::Char('a')));
        assert!(app.review.is_none());
        assert!(texts(&app).contains("已选中 3 个修复"));
    }

    #[test]
    fn typing_is_ignored_during_review() {
        let mut app = AppState::new();
        start_demo(&mut app);
        type_str(&mut app, "xyz");
        assert!(app.editor.is_empty());
        assert!(app.review.is_some());
    }

    #[test]
    fn empty_session_list_does_not_open_a_picker() {
        let mut app = AppState::new();
        app.open_sessions("t", Vec::new());
        assert!(app.choice().is_none());
    }

    #[test]
    fn session_picker_navigates_wraps_and_resumes_the_selected_one() {
        let mut app = AppState::new();
        app.open_sessions("选择对话", session_items());
        let c = app.choice().unwrap();
        assert_eq!(c.title, "选择对话 (1/2)");
        assert_eq!(c.items[0], ("刚刚".to_string(), "对话 s1".to_string()));
        app.on_key(key(KeyCode::Right)); // 左右键在这里没有用
        app.on_key(key(KeyCode::Char('k')));
        assert_eq!(app.choice().unwrap().selected, 0);
        app.on_key(key(KeyCode::Up)); // 绕到最后一项
        assert_eq!(app.choice().unwrap().selected, 1);
        assert_eq!(app.hint(), Some(SESSIONS_HINT));
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::ResumeSession("s2".into())
        );
        assert!(app.choice().is_none(), "the picker closes after choosing");
    }

    #[test]
    fn esc_closes_the_session_picker_and_typing_is_ignored_meanwhile() {
        let mut app = AppState::new();
        app.open_sessions("选择对话", session_items());
        type_str(&mut app, "zzz");
        assert!(app.editor.is_empty());
        assert_eq!(app.on_key(key(KeyCode::Esc)), Action::None);
        assert!(app.choice().is_none());
    }

    #[test]
    fn model_form_takes_over_the_keyboard_and_the_bottom_input() {
        let mut app = AppState::new();
        app.open_model_form(&settings(None)); // 没有 Key，焦点在 Key 上
        let c = app.choice().unwrap();
        assert_eq!(c.title, "模型配置");
        assert_eq!(c.selected, 1);
        assert_eq!(app.input_label(), Some("API Key"));
        assert!(app.input_active());

        type_str(&mut app, "sk-abc");
        assert!(app.editor.is_empty(), "the main input box is not touched");
        let (chars, cursor) = app.input();
        assert_eq!(chars.iter().collect::<String>(), "sk-abc");
        assert_eq!(cursor, 6);
        assert_eq!(app.choice().unwrap().items[1].1, "sk-abc", "the key is shown in plain text");
    }

    #[test]
    fn typing_a_slash_inside_the_form_does_not_open_the_command_menu() {
        let mut app = AppState::new();
        app.open_model_form(&settings(Some("k")));
        type_str(&mut app, "/");
        assert_eq!(app.choice().unwrap().title, "模型配置");
        assert!(app.hint().unwrap().contains("Ctrl-S"));
    }

    #[test]
    fn saving_returns_the_settings_and_the_form_stays_open_until_closed() {
        let mut app = AppState::new();
        app.open_model_form(&settings(None));
        type_str(&mut app, "sk-1");
        match app.on_key(ctrl('s')) {
            Action::SaveModel(s) => {
                assert_eq!(s.api_key.as_deref(), Some("sk-1"));
                assert_eq!(s.model, "deepseek-flash");
            }
            other => panic!("{other:?}"),
        }
        // 写入配置文件可能失败：调用方确认成功之后才关闭，改动不会丢
        assert!(app.form.is_some());
        app.close_form();
        assert!(app.form.is_none());
        assert!(app.choice().is_none());
        assert_eq!(app.hint(), None);
    }

    #[test]
    fn cancelling_the_form_closes_it_without_saving() {
        let mut app = AppState::new();
        app.open_model_form(&settings(Some("k")));
        type_str(&mut app, "x");
        assert_eq!(app.on_key(key(KeyCode::Esc)), Action::None);
        assert!(app.form.is_none());
        assert!(texts(&app).contains("已取消，配置没有改动"));
    }

    #[test]
    fn an_invalid_form_reports_the_problem_and_stays_open() {
        let mut app = AppState::new();
        app.open_model_form(&settings(Some("k"))); // 焦点在模型上
        app.on_key(key(KeyCode::Up));
        app.on_key(key(KeyCode::Up)); // 地址
        app.on_key(ctrl('u'));
        type_str(&mut app, "nope");
        assert_eq!(app.on_key(ctrl('s')), Action::None);
        assert!(app.form.is_some());
        assert!(texts(&app).contains("API 地址"));
        assert_eq!(app.input_label(), Some("API 地址"));
    }

    #[test]
    fn the_form_reaches_the_thinking_slider_and_the_save_row() {
        let mut app = AppState::new();
        app.open_model_form(&settings(Some("k")));
        app.on_key(key(KeyCode::Down)); // 思考力度
        assert_eq!(app.input_label(), None);
        assert!(!app.input_active(), "no text to edit on this row");
        app.on_key(key(KeyCode::Right));
        app.on_key(key(KeyCode::Right));
        app.on_key(key(KeyCode::Down)); // 保存
        match app.on_key(key(KeyCode::Enter)) {
            Action::SaveModel(s) => assert_eq!(s.thinking, Thinking::Low),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn paste_goes_to_the_form_field_or_the_main_input_and_not_while_choosing() {
        let mut app = AppState::new();
        app.on_paste("hello");
        assert_eq!(app.editor.text(), "hello");

        let mut app = AppState::new();
        app.open_model_form(&settings(None));
        app.on_paste("sk-pasted\n");
        assert!(app.editor.is_empty());
        assert_eq!(app.choice().unwrap().items[1].1, "sk-pasted");

        let mut app = AppState::new();
        app.open_sessions("t", session_items());
        app.on_paste("ignored");
        assert!(app.editor.is_empty());

        let mut app = AppState::new();
        start_demo(&mut app);
        app.on_paste("ignored");
        assert!(app.editor.is_empty());
    }

    #[test]
    fn input_is_only_active_when_typing_does_something() {
        let mut app = AppState::new();
        assert!(app.input_active());
        start_demo(&mut app);
        assert!(!app.input_active());

        let mut app = AppState::new();
        app.open_sessions("t", session_items());
        assert!(!app.input_active());
    }

    #[test]
    fn review_with_no_pairs_does_not_open_a_menu() {
        let mut app = AppState::new();
        app.start_review(&[], &[]);
        assert!(app.review.is_none());
        assert!(texts(&app).contains("没有可确认的修复"));
    }
}
