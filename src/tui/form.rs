//! `/model` 表单：API 地址、API Key、模型和思考力度放在一起填，保存时一次写入配置文件。
//! 只做状态和按键处理，不碰终端；选择区的行由 `choice()` 生成，
//! 正在编辑的文本项显示在底部输入框里（带真实光标）。

use super::view::Choice;
use crate::config::{Config, DEFAULT_BASE_URL, KeySource, Thinking};
use crate::editor::Editor;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const F_URL: usize = 0;
const F_KEY: usize = 1;
const F_MODEL: usize = 2;
const F_THINKING: usize = 3;
const F_SAVE: usize = 4;
const FIELD_COUNT: usize = 5;

const NO_CHARS: &[char] = &[];

/// 表单里的设置，也是保存时写入配置文件的内容
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSettings {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub thinking: Thinking,
}

impl ModelSettings {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            base_url: cfg.base_url.clone(),
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
            thinking: cfg.thinking,
        }
    }

    /// 把设置应用到配置上，返回校验过的新配置；传入的配置不会被修改。
    /// 直接赋值而不是逐项 `set`：即使配置文件里原有的某一项不合法，也能用新值把它修好。
    pub fn applied_to(&self, cfg: &Config) -> Result<Config> {
        let mut next = cfg.clone();
        next.base_url = self.base_url.clone();
        next.model = self.model.clone();
        next.api_key = self.api_key.clone();
        next.key_source = if next.api_key.is_some() {
            KeySource::File
        } else {
            KeySource::Unset
        };
        next.thinking = self.thinking;
        next.validate()?;
        Ok(next)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FormEvent {
    None,
    Cancel,
    Save(ModelSettings),
    /// 校验失败：错误信息。焦点已经移到出问题的那一项，表单保持打开
    Invalid(String),
}

pub struct ModelForm {
    base_url: Editor,
    api_key: Editor,
    model: Editor,
    /// `Thinking::ALL` 的下标
    level: usize,
    focus: usize,
}

fn field(text: &str) -> Editor {
    let mut ed = Editor::new(&[]);
    ed.insert_str(text);
    ed
}

/// 单行文本项的编辑键。Enter、上下键由表单处理；Ctrl-J 之类会插入换行的键在这里被忽略。
fn edit(ed: &mut Editor, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Backspace => ed.backspace(),
        KeyCode::Delete => ed.delete(),
        KeyCode::Left if ctrl || alt => ed.word_left(),
        KeyCode::Right if ctrl || alt => ed.word_right(),
        KeyCode::Left => ed.left(),
        KeyCode::Right => ed.right(),
        KeyCode::Home => ed.home(),
        KeyCode::End => ed.end(),
        KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
            'a' => ed.home(),
            'e' => ed.end(),
            'b' => ed.left(),
            'f' => ed.right(),
            'w' => ed.delete_word_back(),
            'u' => ed.kill_to_line_start(),
            'k' => ed.kill_to_line_end(),
            'd' => ed.delete(),
            _ => {}
        },
        KeyCode::Char(c) if !alt => ed.insert(c),
        _ => {}
    }
}

/// 一行滑块：所有档位排开，当前档位加方括号
fn slider(level: usize) -> String {
    Thinking::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == level {
                format!("[{}]", t.as_str())
            } else {
                t.as_str().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
}

impl ModelForm {
    /// 还没有 Key 时先停在 Key 上，否则停在模型上（最常改的一项）
    pub fn new(s: &ModelSettings) -> Self {
        Self {
            base_url: field(&s.base_url),
            api_key: field(s.api_key.as_deref().unwrap_or("")),
            model: field(&s.model),
            level: Thinking::ALL
                .iter()
                .position(|t| *t == s.thinking)
                .unwrap_or(0),
            focus: if s.api_key.is_none() { F_KEY } else { F_MODEL },
        }
    }

    fn editor(&self) -> Option<&Editor> {
        match self.focus {
            F_URL => Some(&self.base_url),
            F_KEY => Some(&self.api_key),
            F_MODEL => Some(&self.model),
            _ => None,
        }
    }

    fn editor_mut(&mut self) -> Option<&mut Editor> {
        match self.focus {
            F_URL => Some(&mut self.base_url),
            F_KEY => Some(&mut self.api_key),
            F_MODEL => Some(&mut self.model),
            _ => None,
        }
    }

    /// 底部输入框显示的内容和光标：当前文本项；思考力度和保存行没有文本
    pub fn input(&self) -> (&[char], usize) {
        match self.editor() {
            Some(ed) => (ed.chars(), ed.cursor()),
            None => (NO_CHARS, 0),
        }
    }

    /// 输入框边框上的标签，告诉用户正在改哪一项
    pub fn label(&self) -> Option<&'static str> {
        match self.focus {
            F_URL => Some("API 地址"),
            F_KEY => Some("API Key"),
            F_MODEL => Some("模型"),
            _ => None,
        }
    }

    pub fn hint(&self) -> &'static str {
        match self.focus {
            F_THINKING => "←→ 调整思考力度 · ↑↓ 切换项 · Enter 下一项 · Ctrl-S 保存 · Esc 取消",
            F_SAVE => "Enter 保存 · ↑↓ 切换项 · Esc 取消",
            _ => "直接输入修改 · ↑↓ 切换项 · Enter 下一项 · Ctrl-S 保存 · Esc 取消",
        }
    }

    /// 选择区里的表单行。Key 明文显示；空的文本项显示「未设置」
    pub fn choice(&self) -> Choice {
        let text = |ed: &Editor| {
            let t = ed.text();
            if t.is_empty() { "（未设置）".to_string() } else { t }
        };
        Choice {
            title: "模型配置".into(),
            items: vec![
                ("API 地址".into(), text(&self.base_url)),
                ("API Key".into(), text(&self.api_key)),
                ("模型".into(), text(&self.model)),
                ("思考力度".into(), slider(self.level)),
                ("保存".into(), "写入配置文件，以后每次启动都会使用".into()),
            ],
            selected: self.focus,
        }
    }

    /// 粘贴：只进当前文本项，换行和制表符直接丢掉（常见于复制 Key 时带上的结尾换行）
    pub fn paste(&mut self, text: &str) {
        let single: String = text
            .chars()
            .filter(|c| !matches!(c, '\r' | '\n' | '\t'))
            .collect();
        if let Some(ed) = self.editor_mut() {
            ed.insert_str(&single);
        }
    }

    fn move_focus(&mut self, delta: isize) {
        self.focus = (self.focus as isize + delta).rem_euclid(FIELD_COUNT as isize) as usize;
    }

    fn adjust_level(&mut self, delta: isize) {
        let max = Thinking::ALL.len() as isize - 1;
        self.level = (self.level as isize + delta).clamp(0, max) as usize;
    }

    pub fn on_key(&mut self, key: KeyEvent) -> FormEvent {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return FormEvent::Cancel,
            KeyCode::Char('c') if ctrl => return FormEvent::Cancel,
            KeyCode::Char('s') if ctrl => return self.save(),
            KeyCode::Up | KeyCode::BackTab => self.move_focus(-1),
            KeyCode::Down | KeyCode::Tab => self.move_focus(1),
            KeyCode::Enter if self.focus == F_SAVE => return self.save(),
            KeyCode::Enter => self.move_focus(1),
            KeyCode::Left if self.focus == F_THINKING => self.adjust_level(-1),
            KeyCode::Right if self.focus == F_THINKING => self.adjust_level(1),
            _ => {
                if let Some(ed) = self.editor_mut() {
                    edit(ed, key);
                }
            }
        }
        FormEvent::None
    }

    fn save(&mut self) -> FormEvent {
        match self.validate() {
            Ok(settings) => FormEvent::Save(settings),
            Err((focus, msg)) => {
                self.focus = focus;
                FormEvent::Invalid(msg)
            }
        }
    }

    /// 按表单顺序检查，返回第一个问题和它所在的项
    fn validate(&self) -> Result<ModelSettings, (usize, String)> {
        let url = self.base_url.text().trim().to_string();
        if url.is_empty() {
            return Err((F_URL, format!("API 地址不能为空，默认是 {DEFAULT_BASE_URL}")));
        }
        if url.chars().any(char::is_whitespace) {
            return Err((F_URL, "API 地址不能包含空白字符".into()));
        }
        if let Err(e) = Config::default().set("base_url", &url) {
            return Err((F_URL, format!("{e:#}").replace("base_url", "API 地址")));
        }
        let key = self.api_key.text().trim().to_string();
        if key.chars().any(char::is_whitespace) {
            return Err((F_KEY, "API Key 不能包含空白字符，请检查是否多复制了内容".into()));
        }
        let model = self.model.text().trim().to_string();
        if model.is_empty() {
            return Err((F_MODEL, "模型不能为空，例如 deepseek-flash".into()));
        }
        if model.chars().any(char::is_whitespace) {
            return Err((F_MODEL, "模型名不能包含空白字符".into()));
        }
        Ok(ModelSettings {
            base_url: url,
            api_key: (!key.is_empty()).then_some(key),
            model,
            thinking: Thinking::ALL[self.level],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn settings(api_key: Option<&str>) -> ModelSettings {
        ModelSettings {
            base_url: "https://api.deepseek.com".into(),
            api_key: api_key.map(String::from),
            model: "deepseek-flash".into(),
            thinking: Thinking::Auto,
        }
    }

    fn type_str(f: &mut ModelForm, s: &str) {
        for c in s.chars() {
            f.on_key(key(KeyCode::Char(c)));
        }
    }

    fn text(f: &ModelForm) -> String {
        f.input().0.iter().collect()
    }

    fn row(f: &ModelForm, i: usize) -> (String, String) {
        f.choice().items[i].clone()
    }

    #[test]
    fn starts_on_key_when_missing_otherwise_on_model() {
        assert_eq!(ModelForm::new(&settings(None)).focus, F_KEY);
        assert_eq!(ModelForm::new(&settings(Some("sk-1"))).focus, F_MODEL);
    }

    #[test]
    fn typing_edits_only_the_focused_field_and_the_key_is_plain_text() {
        let mut f = ModelForm::new(&settings(None));
        assert_eq!(f.label(), Some("API Key"));
        type_str(&mut f, "sk-abc123");
        assert_eq!(text(&f), "sk-abc123");
        assert_eq!(row(&f, F_KEY).1, "sk-abc123", "the key is shown in full");
        assert_eq!(row(&f, F_URL).1, "https://api.deepseek.com");
        assert_eq!(row(&f, F_MODEL).1, "deepseek-flash");
    }

    #[test]
    fn rows_have_labels_placeholders_and_a_slider() {
        let f = ModelForm::new(&settings(None));
        let c = f.choice();
        let names: Vec<_> = c.items.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["API 地址", "API Key", "模型", "思考力度", "保存"]);
        assert_eq!(c.title, "模型配置");
        assert_eq!(c.selected, F_KEY);
        assert_eq!(row(&f, F_KEY).1, "（未设置）");
        assert_eq!(row(&f, F_THINKING).1, "[auto]  off  low  high  max");
    }

    #[test]
    fn up_down_tab_and_backtab_wrap_around() {
        let mut f = ModelForm::new(&settings(Some("k")));
        assert_eq!(f.focus, F_MODEL);
        f.on_key(key(KeyCode::Down));
        f.on_key(key(KeyCode::Tab));
        assert_eq!(f.focus, F_SAVE);
        f.on_key(key(KeyCode::Down));
        assert_eq!(f.focus, F_URL, "wraps from the last row to the first");
        f.on_key(key(KeyCode::Up));
        assert_eq!(f.focus, F_SAVE);
        f.on_key(key(KeyCode::BackTab));
        assert_eq!(f.focus, F_THINKING);
    }

    #[test]
    fn enter_moves_on_and_only_saves_on_the_save_row() {
        let mut f = ModelForm::new(&settings(None));
        type_str(&mut f, "sk-1");
        for expected in [F_MODEL, F_THINKING, F_SAVE] {
            assert_eq!(f.on_key(key(KeyCode::Enter)), FormEvent::None);
            assert_eq!(f.focus, expected);
        }
        assert_eq!(
            f.on_key(key(KeyCode::Enter)),
            FormEvent::Save(ModelSettings {
                api_key: Some("sk-1".into()),
                ..settings(None)
            })
        );
    }

    #[test]
    fn ctrl_s_saves_from_any_row() {
        let mut f = ModelForm::new(&settings(Some("sk-1")));
        assert!(matches!(f.on_key(ctrl('s')), FormEvent::Save(_)));
        f.on_key(key(KeyCode::Up));
        assert_eq!(f.focus, F_KEY);
        assert!(matches!(f.on_key(ctrl('s')), FormEvent::Save(_)));
        f.on_key(key(KeyCode::Down));
        f.on_key(key(KeyCode::Down));
        assert_eq!(f.focus, F_THINKING);
        assert!(matches!(f.on_key(ctrl('s')), FormEvent::Save(_)));
    }

    #[test]
    fn left_right_change_thinking_only_on_that_row_and_clamp() {
        let mut f = ModelForm::new(&settings(Some("k")));
        // 文本项上左右键只移动光标
        f.on_key(key(KeyCode::Left));
        f.on_key(key(KeyCode::Right));
        assert_eq!(f.level, 0);
        f.on_key(key(KeyCode::Down));
        assert_eq!(f.focus, F_THINKING);
        f.on_key(key(KeyCode::Left));
        assert_eq!(f.level, 0, "clamped at the first level");
        for _ in 0..3 {
            f.on_key(key(KeyCode::Right));
        }
        assert_eq!(row(&f, F_THINKING).1, "auto  off  low  [high]  max");
        for _ in 0..10 {
            f.on_key(key(KeyCode::Right));
        }
        assert_eq!(f.level, Thinking::ALL.len() - 1);
        match f.on_key(ctrl('s')) {
            FormEvent::Save(s) => assert_eq!(s.thinking, Thinking::Max),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rows_without_text_show_an_empty_input() {
        let mut f = ModelForm::new(&settings(Some("k")));
        f.on_key(key(KeyCode::Down));
        assert_eq!(f.label(), None);
        assert!(f.input().0.is_empty());
        type_str(&mut f, "zzz"); // 打字不会落到别的项上
        assert_eq!(row(&f, F_MODEL).1, "deepseek-flash");
    }

    #[test]
    fn editing_keys_work_inside_a_field() {
        let mut f = ModelForm::new(&settings(None));
        type_str(&mut f, "abc def");
        f.on_key(ctrl('w'));
        assert_eq!(text(&f), "abc ");
        f.on_key(key(KeyCode::Backspace));
        assert_eq!(text(&f), "abc");
        f.on_key(key(KeyCode::Home));
        type_str(&mut f, "X");
        assert_eq!(text(&f), "Xabc");
        f.on_key(ctrl('u'));
        assert_eq!(text(&f), "abc");
        f.on_key(ctrl('k'));
        assert_eq!(text(&f), "");
        f.on_key(ctrl('j'));
        assert_eq!(text(&f), "", "no newlines in single-line fields");
    }

    #[test]
    fn invalid_address_is_reported_and_focus_jumps_to_it() {
        let mut f = ModelForm::new(&settings(Some("k")));
        f.on_key(key(KeyCode::Up));
        f.on_key(key(KeyCode::Up));
        assert_eq!(f.focus, F_URL);
        f.on_key(ctrl('u'));
        type_str(&mut f, "ftp://nope.example");
        f.on_key(key(KeyCode::Down)); // 移开焦点，错误后应跳回地址
        match f.on_key(ctrl('s')) {
            FormEvent::Invalid(msg) => {
                assert!(msg.contains("API 地址") && msg.contains("http"), "{msg}");
                assert!(!msg.contains("base_url"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(f.focus, F_URL);
        assert_eq!(text(&f), "ftp://nope.example", "the text is kept for fixing");
    }

    #[test]
    fn empty_address_model_and_inner_whitespace_are_rejected() {
        let mut f = ModelForm::new(&settings(Some("k")));
        f.base_url = field("");
        match f.on_key(ctrl('s')) {
            FormEvent::Invalid(msg) => assert!(msg.contains("不能为空") && msg.contains("deepseek.com")),
            other => panic!("{other:?}"),
        }
        assert_eq!(f.focus, F_URL);

        let mut f = ModelForm::new(&settings(Some("k")));
        f.api_key = field("sk a b");
        assert!(matches!(f.on_key(ctrl('s')), FormEvent::Invalid(m) if m.contains("API Key")));
        assert_eq!(f.focus, F_KEY);

        let mut f = ModelForm::new(&settings(Some("k")));
        f.model = field("   ");
        assert!(matches!(f.on_key(ctrl('s')), FormEvent::Invalid(m) if m.contains("模型不能为空")));
        assert_eq!(f.focus, F_MODEL);
        f.model = field("my model");
        assert!(matches!(f.on_key(ctrl('s')), FormEvent::Invalid(m) if m.contains("模型名")));
    }

    #[test]
    fn the_first_problem_in_form_order_wins() {
        let mut f = ModelForm::new(&settings(Some("k")));
        f.base_url = field("nope");
        f.model = field("");
        assert!(matches!(f.on_key(ctrl('s')), FormEvent::Invalid(_)));
        assert_eq!(f.focus, F_URL);
    }

    #[test]
    fn empty_key_is_allowed_and_values_are_trimmed() {
        let mut f = ModelForm::new(&settings(None));
        f.base_url = field("  https://example.com/v1  ");
        f.model = field(" my-model ");
        match f.on_key(ctrl('s')) {
            FormEvent::Save(s) => {
                assert_eq!(s.api_key, None);
                assert_eq!(s.base_url, "https://example.com/v1");
                assert_eq!(s.model, "my-model");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn esc_and_ctrl_c_cancel() {
        let mut f = ModelForm::new(&settings(None));
        assert_eq!(f.on_key(key(KeyCode::Esc)), FormEvent::Cancel);
        assert_eq!(f.on_key(ctrl('c')), FormEvent::Cancel);
    }

    #[test]
    fn paste_drops_line_breaks_and_only_touches_text_rows() {
        let mut f = ModelForm::new(&settings(None));
        f.paste("sk-pasted\r\n");
        assert_eq!(text(&f), "sk-pasted");
        f.paste("\tmore\n");
        assert_eq!(text(&f), "sk-pastedmore");
        f.on_key(key(KeyCode::Down));
        f.on_key(key(KeyCode::Down));
        assert_eq!(f.focus, F_THINKING);
        f.paste("ignored");
        assert_eq!(row(&f, F_KEY).1, "sk-pastedmore");
        assert_eq!(row(&f, F_MODEL).1, "deepseek-flash");
    }

    #[test]
    fn hint_depends_on_the_row() {
        let mut f = ModelForm::new(&settings(None));
        assert!(f.hint().starts_with("直接输入修改"));
        f.on_key(key(KeyCode::Down));
        f.on_key(key(KeyCode::Down));
        assert!(f.hint().starts_with("←→ 调整思考力度"));
        f.on_key(key(KeyCode::Down));
        assert!(f.hint().starts_with("Enter 保存"));
    }

    #[test]
    fn settings_roundtrip_through_a_config() {
        let s = ModelSettings {
            base_url: "https://x.example/v1".into(),
            api_key: Some("sk-1".into()),
            model: "m".into(),
            thinking: Thinking::Max,
        };
        let base = Config::default();
        let next = s.applied_to(&base).unwrap();
        assert_eq!(ModelSettings::from_config(&next), s);
        assert_eq!(next.key_source, KeySource::File);
        assert_eq!(base, Config::default(), "the original config is untouched");

        let none = ModelSettings { api_key: None, ..s.clone() }
            .applied_to(&next)
            .unwrap();
        assert_eq!(none.api_key, None);
        assert_eq!(none.key_source, KeySource::Unset);
    }

    #[test]
    fn applied_to_rejects_invalid_settings_and_repairs_a_broken_config() {
        let bad = ModelSettings {
            base_url: "nope".into(),
            ..settings(None)
        };
        assert!(bad.applied_to(&Config::default()).is_err());

        // 配置文件里原来的模型是空的：用表单里的新值可以直接修好
        let broken = Config {
            model: String::new(),
            ..Config::default()
        };
        assert!(broken.validate().is_err());
        assert!(settings(None).applied_to(&broken).is_ok());
    }
}
