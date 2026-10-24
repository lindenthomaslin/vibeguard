//! 全屏界面的应用状态：处理按键与后台事件，不碰终端，便于测试。

use super::form::{FormEvent, ModelForm, ModelSettings};
use super::sessions::{Event as PanelEvent, SessionItem, SessionsPanel};
use super::view::Choice;
use crate::chat::{Entry, Scroll, Transcript};
use crate::editor::{Candidate, Editor};
use crate::finding::{Finding, Fix};
use crate::i18n::{Lang, pick, tr};
use crate::input::{Outcome, handle_key};
use crate::mode::Mode;
use crate::review;
use crate::review::{Decision, Reviewer};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// 输入 `/` 时弹出的命令菜单
impl Candidate {
    /// 命令说明。英文模式下走下面这张表——中文原文留在 `desc` 里，
    /// 中文界面直接用，不用维护两份结构。
    pub fn desc(&self) -> &'static str {
        if crate::i18n::is_zh() {
            return self.desc;
        }
        match self.name {
            "/model" => "Configure endpoint, API key, model and thinking effort",
            "/mode" => "Switch work mode: plan / confirm / auto",
            "/lang" => "Switch interface language: English / Chinese",
            "/accent" => "Switch the accent colour (the interface's main colour)",
            "/trust" => "Trust this workspace: allow commands and edits",
            "/scan" => "Scan a directory with local rules; --ai adds model judgement",
            "/fix" => "Generate fixes for scan findings, apply after you approve",
            "/report" => "Export the last scan as .md + .json",
            "/theme" => "Colour theme: dark / light (Ctrl-T)",
            "/font" => "Glyph style: default / anime",
            "/permissions" => "Permission mode and workspace root",
            "/resume" => "Resume the previous conversation",
            "/sessions" => "History: search, open, delete",
            "/search" => "Open history with a query: /search <keyword>",
            "/clear" => "Start a new conversation (the old one is kept)",
            "/thinking" => "Show or change thinking effort (this session)",
            "/config" => "Show or change configuration",
            "/help" => "Show help",
            "/demo" => "Preview the scan and fix screens with sample data",
            "/exit" => "Quit",
            _ => self.desc,
        }
    }
}

pub const COMMANDS: &[Candidate] = &[
    Candidate {
        name: "/model",
        desc: "配置 API 地址、API Key、模型与思考力度",
    },
    Candidate {
        name: "/mode",
        desc: "切换工作模式：计划 / 逐条确认 / 全自动",
    },
    Candidate {
        name: "/accent",
        desc: "切换强调色（界面主色）",
    },
    Candidate {
        name: "/trust",
        desc: "信任当前工作区，允许执行与写入",
    },
    Candidate {
        name: "/lang",
        desc: "切换界面语言：英文 / 中文",
    },
    Candidate {
        name: "/scan",
        desc: "扫描目录：本地规则；加 --ai 让模型逐条判定（/scan 路径 --ai）",
    },
    Candidate {
        name: "/fix",
        desc: "为扫描结果生成修复，逐条确认后写入（/fix 数量 或 /fix VG-003）",
    },
    Candidate {
        name: "/report",
        desc: "把扫描结果导出成 .md + .json（/report 路径）",
    },
    Candidate {
        name: "/theme",
        desc: "切换配色：dark / light（Ctrl-T）",
    },
    Candidate {
        name: "/font",
        desc: "切换字形：default / anime",
    },
    Candidate {
        name: "/permissions",
        desc: "配置权限模式和工作目录",
    },
    Candidate {
        name: "/resume",
        desc: "恢复上一次对话",
    },
    Candidate {
        name: "/sessions",
        desc: "历史对话：搜索、打开、删除",
    },
    Candidate {
        name: "/search",
        desc: "带关键词打开历史对话：/search 关键词",
    },
    Candidate {
        name: "/clear",
        desc: "开始新对话（旧对话会保留）",
    },
    Candidate {
        name: "/thinking",
        desc: "查看或切换思考模式（仅本次）",
    },
    Candidate {
        name: "/config",
        desc: "查看或修改配置",
    },
    Candidate {
        name: "/help",
        desc: "显示帮助",
    },
    Candidate {
        name: "/demo",
        desc: "预览扫描与修复界面（示例数据）",
    },
    Candidate {
        name: "/exit",
        desc: "退出",
    },
];

fn review_hint() -> &'static str {
    crate::i18n::t(
        "↑↓ 选择 · Enter 确认 · y 应用 · n 跳过 · a 全部应用 · q 结束",
        "↑↓ select · Enter confirm · y apply · n skip · a apply all · q quit",
    )
}

fn sessions_hint() -> &'static str {
    crate::i18n::t(
        "输入过滤 · ↑↓ 选择 · Enter 打开 · Del 删除 · Esc 关闭",
        "type to filter · ↑↓ select · Enter open · Del delete · Esc close",
    )
}

fn confirm_hint() -> &'static str {
    crate::i18n::t(
        "↑↓ 选择 · Enter 确认 · y 应用 · n 跳过 · a 本次全部应用 · Esc 跳过",
        "↑↓ select · Enter confirm · y apply · n skip · a apply all · Esc skip",
    )
}

fn picker_hint() -> &'static str {
    crate::i18n::t(
        "↑↓ 选择 · Enter 确认 · Esc 取消",
        "↑↓ select · Enter confirm · Esc cancel",
    )
}

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
    /// 历史对话面板确认：要恢复的对话编号
    ResumeSession(String),
    /// 历史对话面板里按了两次 Del：要删掉的对话编号
    DeleteSession(String),
    /// 逐条确认模式下，用户对一处改动的答复
    Answer(Answer),
    /// Ctrl-T：换一套配色
    CycleTheme,
    /// 逐条确认结束，这些修复被接受了，去写文件
    ApplyFixes(Vec<Fix>),
    /// 语言选择里按了 Enter：切到这个语言
    SetLang(Lang),
    /// 信任浮层的答复
    Trust(bool),
    /// 强调色选择
    SetAccent(crate::theme::Accent),
    /// 主题 / 字形 / 权限模式的选择
    SetTheme(crate::theme::Theme),
    SetFont(crate::theme::FontStyle),
    SetPermission(crate::tools::PermissionMode),
    /// 工作模式（plan / confirm / auto）
    SetMode(crate::mode::Mode),
}

/// 逐条确认模式下的答复
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// 写入这一处
    Apply,
    /// 不写入，继续下一个
    Skip,
    /// 本次请求里剩下的改动不再逐条询问
    Always,
}

impl Answer {
    /// 是否同意写入
    pub fn approved(self) -> bool {
        !matches!(self, Answer::Skip)
    }
}

/// 逐条确认的选项，顺序与 `Answer` 对应
const CONFIRM_OPTIONS: [(&str, &str, Answer); 3] = [
    ("应用", "写入这个改动", Answer::Apply),
    ("跳过", "不写入，继续下一个", Answer::Skip),
    (
        "本次全部应用",
        "这次请求里剩下的改动不再询问",
        Answer::Always,
    ),
];

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

/// 逐条确认模式下，一处等着答复的文件改动。
/// 改动内容本身已经作为 `Entry::Diff` 写进对话区，这里只记路径和菜单选中项。
struct Confirm {
    path: String,
    selected: usize,
}

/// 一个两三项的选择浮层。语言、信任工作区、强调色共用一套——
/// 三处各写一份键盘处理和绘制，只会让其中两处慢慢长歪。
struct Picker {
    kind: PickerKind,
    selected: usize,
    /// 标题里要带的上下文（信任浮层放路径，其余为空）
    subject: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerKind {
    Lang,
    Trust,
    Accent,
    Theme,
    Font,
    Permission,
    Mode,
}

/// 可选的语言，顺序即菜单顺序
const LANGS: [Lang; 2] = [Lang::En, Lang::Zh];

fn lang_desc(lang: Lang) -> &'static str {
    match lang {
        Lang::En => tr("默认语言"),
        Lang::Zh => tr("中文界面"),
    }
}

fn theme_desc(theme: crate::theme::Theme) -> &'static str {
    match theme {
        crate::theme::Theme::Dark => tr("深色底，亮色字"),
        crate::theme::Theme::Light => tr("浅色底，深色字"),
    }
}

fn font_desc(font: crate::theme::FontStyle) -> &'static str {
    match font {
        crate::theme::FontStyle::Default => tr("朴素框线，圆点标记"),
        crate::theme::FontStyle::Anime => tr("圆角框 + 星星爱心"),
    }
}

fn accent_desc(accent: crate::theme::Accent) -> &'static str {
    match accent {
        crate::theme::Accent::Teal => tr("默认的青"),
        crate::theme::Accent::Sakura => tr("樱花粉"),
        crate::theme::Accent::Ocean => tr("海蓝"),
        crate::theme::Accent::Violet => tr("紫罗兰"),
        crate::theme::Accent::Amber => tr("琥珀"),
        crate::theme::Accent::Lime => tr("青柠"),
        crate::theme::Accent::Mono => tr("无彩"),
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
    /// 历史对话面板；打开时键盘和粘贴都归它
    pub sessions: Option<SessionsPanel>,
    /// 最近一次扫描的结果，`/fix` 之后会用到
    pub findings: Vec<Finding>,
    /// 最近一次模型判定（`/report` 要写进报告，包括「不存在」和「无法判断」）
    pub judgements: Vec<crate::scan_ai::Judgement>,
    /// 最近一次扫描了多少个文件，报告里要写
    pub files_scanned: usize,
    /// 判定失败的批次。报告里必须写出来，否则「覆盖 230/410」会显得很完整
    pub failed_batches: Vec<String>,
    /// 模型给了判定、但证据核实不了的条目（文件不存在 / 行号超范围）
    pub unverified: Vec<String>,
    /// 这一轮故意没交给模型的工程规范条目数（N/O/Q/S）
    pub skipped_items: usize,
    /// 扫描时每个文件的指纹：用来判断「这条发现还准不准」
    pub fingerprints: std::collections::BTreeMap<String, crate::scan_store::Fingerprint>,
    /// 这次判定所依据的仓库快照，增量扫描靠它判断哪些文件变过
    pub digest: Option<crate::scan_ai::ScanDigest>,
    /// 有值表示这一轮只走界面、不写文件，值就是原因（演示 / 计划模式 / 只读权限）
    dry_run: Option<String>,
    /// `/model` 表单；打开时键盘输入都交给它
    form: Option<ModelForm>,
    /// 当前工作模式，决定 AI 能不能改文件、改之前要不要问
    pub mode: Mode,
    /// 逐条确认模式下等待答复的改动
    confirm: Option<Confirm>,
    /// `/lang` 打开的语言选择；打开时键盘归它
    picker: Option<Picker>,
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
            sessions: None,
            findings: Vec::new(),
            judgements: Vec::new(),
            files_scanned: 0,
            failed_batches: Vec::new(),
            unverified: Vec::new(),
            skipped_items: 0,
            fingerprints: std::collections::BTreeMap::new(),
            digest: None,
            dry_run: None,
            form: None,
            mode: Mode::default(),
            confirm: None,
            picker: None,
        }
    }

    // ---- 工作模式 ----

    /// 切换模式并写进对话区。已经是这个模式就什么都不做。
    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.push(Entry::Info(pick(
            format!("已切换到「{}」模式：{}", mode.title(), mode.describe()),
            format!("Switched to \"{}\" mode: {}", mode.title(), mode.describe()),
        )));
    }

    /// 逐条确认：先把改动写进对话区，再弹出三选一。
    /// 后台任务正停在这里等答复，所以不能静默丢弃。
    pub fn ask_confirm(&mut self, path: String, old_code: String, new_code: String) {
        self.push(Entry::Diff {
            path: path.clone(),
            old: old_code,
            new: new_code,
        });
        self.confirm = Some(Confirm { path, selected: 0 });
    }

    /// 有没有改动在等用户点头
    pub fn is_confirming(&self) -> bool {
        self.confirm.is_some()
    }

    /// 关掉确认卡片。写入结果由后台任务回报，这里不猜。
    pub fn close_confirm(&mut self, answer: Answer) {
        let Some(state) = self.confirm.take() else {
            return;
        };
        if answer == Answer::Skip {
            self.push(Entry::Info(pick(
                format!("已跳过 {}", state.path),
                format!("Skipped {}", state.path),
            )));
        }
    }

    // ---- 选择区 ----

    /// 当前应显示的选择区：逐条确认优先，其次是修复确认、对话选择器、模型表单和斜杠命令菜单
    pub fn choice(&self) -> Option<Choice> {
        if let Some(c) = &self.confirm {
            return Some(Choice {
                title: pick(
                    format!("应用对 {} 的改动？", c.path),
                    format!("Apply the change to {}?", c.path),
                ),
                items: CONFIRM_OPTIONS
                    .iter()
                    .map(|&(n, d, _)| (tr(n).to_string(), tr(d).to_string()))
                    .collect(),
                selected: c.selected,
            });
        }
        if let Some(r) = &self.review {
            return Some(Choice {
                title: tr("应用这个修复？").into(),
                items: REVIEW_OPTIONS
                    .iter()
                    .map(|&(n, d, _)| (tr(n).to_string(), tr(d).to_string()))
                    .collect(),
                selected: r.selected,
            });
        }
        if let Some(p) = &self.picker {
            let (title, items) = match p.kind {
                PickerKind::Lang => (
                    tr("界面语言").to_string(),
                    LANGS
                        .iter()
                        .map(|&l| (l.label().to_string(), lang_desc(l).to_string()))
                        .collect(),
                ),
                PickerKind::Trust => (
                    pick(
                        format!("信任这个工作区？{}", p.subject),
                        format!("Trust this workspace? {}", p.subject),
                    ),
                    vec![
                        (
                            tr("是，信任它").to_string(),
                            tr("可以在这里执行命令、修改文件").to_string(),
                        ),
                        (
                            tr("否，只读").to_string(),
                            tr("本次会话不写文件、不跑命令").to_string(),
                        ),
                    ],
                ),
                PickerKind::Accent => (
                    tr("强调色").to_string(),
                    crate::theme::Accent::ALL
                        .iter()
                        .map(|&a| (a.label().to_string(), accent_desc(a).to_string()))
                        .collect(),
                ),
                PickerKind::Theme => (
                    tr("界面主题").to_string(),
                    crate::theme::Theme::ALL
                        .iter()
                        .map(|&t| (t.label().to_string(), theme_desc(t).to_string()))
                        .collect(),
                ),
                PickerKind::Font => (
                    tr("字形").to_string(),
                    crate::theme::FontStyle::ALL
                        .iter()
                        .map(|&f| (f.label().to_string(), font_desc(f).to_string()))
                        .collect(),
                ),
                PickerKind::Permission => (
                    tr("权限模式").to_string(),
                    crate::tools::PermissionMode::ALL
                        .iter()
                        .map(|&m| (m.label().to_string(), m.desc().to_string()))
                        .collect(),
                ),
                PickerKind::Mode => (
                    tr("工作模式").to_string(),
                    crate::mode::Mode::ALL
                        .iter()
                        .map(|&m| (m.label().to_string(), m.describe().to_string()))
                        .collect(),
                ),
            };
            return Some(Choice {
                title,
                items,
                selected: p.selected,
            });
        }
        if let Some(f) = &self.form {
            return Some(f.choice());
        }
        let matches = self.editor.matches();
        if matches.is_empty() {
            return None;
        }
        Some(Choice {
            title: crate::i18n::t("命令", "Commands").into(),
            items: matches
                .iter()
                .map(|c| (c.name.to_string(), c.desc().to_string()))
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
        if self.confirm.is_some()
            || self.review.is_some()
            || self.sessions.is_some()
            || self.picker.is_some()
        {
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
        self.scroll.jump_to_bottom();
    }

    /// 开始逐条确认：先显示汇总，再显示第一条
    /// `dry_run` 有值时只走界面、不写文件，值是给用户看的原因
    pub fn start_review(&mut self, findings: &[Finding], fixes: &[Fix], dry_run: Option<String>) {
        self.dry_run = dry_run;
        self.push(Entry::Summary(findings.to_vec()));
        let reviewer = Reviewer::new(findings, fixes);
        if reviewer.total() == 0 {
            self.push(Entry::Info(tr("没有可确认的修复").into()));
            return;
        }
        self.review = Some(ReviewState {
            reviewer,
            selected: 0,
        });
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

    fn answer_review(&mut self, decision: Decision) -> Action {
        let Some(state) = &mut self.review else {
            return Action::None;
        };
        state.reviewer.answer(decision);
        state.selected = 0;
        if state.reviewer.current().is_some() {
            self.show_current_review();
            return Action::None;
        }
        let Some(state) = self.review.take() else {
            return Action::None;
        };
        let accepted = state.reviewer.into_accepted();
        if accepted.is_empty() {
            self.push(Entry::Info(tr("没有选中任何修复").into()));
            return Action::None;
        }
        if let Some(reason) = self.dry_run.take() {
            let names: Vec<_> = accepted.iter().map(|f| f.finding_id.as_str()).collect();
            self.push(Entry::Success(pick(
                format!(
                    "已选中 {} 个修复：{}（未写入文件 · {reason}）",
                    accepted.len(),
                    names.join("、")
                ),
                format!(
                    "{} fixes selected: {} (nothing written · {reason})",
                    accepted.len(),
                    names.join("、")
                ),
            )));
            return Action::None;
        }
        Action::ApplyFixes(accepted)
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
                self.scroll.jump_to_top();
                return Action::None;
            }
            KeyCode::Char('g') if ctrl => {
                self.scroll.jump_to_bottom();
                return Action::None;
            }
            _ => {}
        }

        // 切换模式：Ctrl-P，或者 Shift-Tab（终端把它报成 BackTab）。
        // 弹出层开着时不响应——正在确认的时候换模式只会让人更糊涂。
        let chord = (ctrl && key.code == KeyCode::Char('p'))
            || matches!(key.code, KeyCode::BackTab)
            || (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT));
        if chord && self.no_overlay() {
            self.set_mode(self.mode.next());
            return Action::None;
        }
        // Ctrl-T 换配色。主题存在配置里，所以交给调用方改，这里只表达意图
        if ctrl && key.code == KeyCode::Char('t') && self.no_overlay() {
            return Action::CycleTheme;
        }

        if self.confirm.is_some() {
            return self.on_confirm_key(key);
        }
        if self.review.is_some() {
            return self.on_review_key(key);
        }
        if self.sessions.is_some() {
            return self.on_sessions_key(key);
        }
        if self.form.is_some() {
            return self.on_form_key(key);
        }
        if self.picker.is_some() {
            return self.on_picker_key(key);
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
                        let parts: Vec<String> = cmd.split_whitespace().map(String::from).collect();
                        if matches!(
                            parts.first().map(String::as_str),
                            Some("exit" | "quit" | "q")
                        ) {
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
                self.push(Entry::Info(tr("再按一次 Ctrl-C 退出").into()));
                Action::None
            }
        }
    }

    /// 有没有弹出层占着键盘
    fn no_overlay(&self) -> bool {
        self.confirm.is_none()
            && self.review.is_none()
            && self.sessions.is_none()
            && self.form.is_none()
            && self.picker.is_none()
    }

    fn on_picker_key(&mut self, key: KeyEvent) -> Action {
        let Some(p) = self.picker.as_ref() else {
            return Action::None;
        };
        let kind = p.kind;
        let selected = p.selected;
        let n = match kind {
            PickerKind::Lang => LANGS.len(),
            PickerKind::Trust => 2,
            PickerKind::Accent => crate::theme::Accent::ALL.len(),
            PickerKind::Theme => crate::theme::Theme::ALL.len(),
            PickerKind::Font => crate::theme::FontStyle::ALL.len(),
            PickerKind::Permission => crate::tools::PermissionMode::ALL.len(),
            PickerKind::Mode => crate::mode::Mode::ALL.len(),
        };
        let mut move_to = |sel: usize| {
            if let Some(p) = self.picker.as_mut() {
                p.selected = sel;
            }
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                move_to((selected + n - 1) % n);
                Action::None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                move_to((selected + 1) % n);
                Action::None
            }
            KeyCode::Enter => {
                self.picker = None;
                match kind {
                    PickerKind::Lang => Action::SetLang(LANGS[selected]),
                    // 第一项是「信任」，第二项是「只读」
                    PickerKind::Trust => Action::Trust(selected == 0),
                    PickerKind::Accent => Action::SetAccent(crate::theme::Accent::ALL[selected]),
                    PickerKind::Theme => Action::SetTheme(crate::theme::Theme::ALL[selected]),
                    PickerKind::Font => Action::SetFont(crate::theme::FontStyle::ALL[selected]),
                    PickerKind::Permission => {
                        Action::SetPermission(crate::tools::PermissionMode::ALL[selected])
                    }
                    PickerKind::Mode => Action::SetMode(crate::mode::Mode::ALL[selected]),
                }
            }
            // 信任浮层按 Esc 当「不信任」：关掉窗口不能等于放行
            KeyCode::Esc | KeyCode::Char('q') => {
                self.picker = None;
                if kind == PickerKind::Trust {
                    Action::Trust(false)
                } else {
                    Action::None
                }
            }
            _ => Action::None,
        }
    }

    fn on_confirm_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(c) = &mut self.confirm else {
            return Action::None;
        };
        let n = CONFIRM_OPTIONS.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => c.selected = (c.selected + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => c.selected = (c.selected + 1) % n,
            KeyCode::Char('y') => return Action::Answer(Answer::Apply),
            KeyCode::Char('n') | KeyCode::Char('q') | KeyCode::Esc => {
                return Action::Answer(Answer::Skip);
            }
            KeyCode::Char('a') => return Action::Answer(Answer::Always),
            KeyCode::Char('c') if ctrl => return Action::Answer(Answer::Skip),
            KeyCode::Enter => return Action::Answer(CONFIRM_OPTIONS[c.selected].2),
            _ => {}
        }
        Action::None
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
            KeyCode::Char('y') => return self.answer_review(Decision::Apply),
            KeyCode::Char('n') => return self.answer_review(Decision::Skip),
            KeyCode::Char('a') => return self.answer_review(Decision::ApplyRest),
            KeyCode::Char('q') | KeyCode::Esc => return self.answer_review(Decision::Quit),
            KeyCode::Enter => {
                let decision = REVIEW_OPTIONS[state.selected].2;
                return self.answer_review(decision);
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return self.answer_review(Decision::Quit);
            }
            _ => {}
        }
        Action::None
    }

    // ---- 历史对话面板 ----

    /// 打开面板。列表为空时不打开，由调用方负责告诉用户「还没有历史对话」。
    pub fn open_sessions(&mut self, items: Vec<SessionItem>, query: &str) {
        self.sessions = SessionsPanel::with_query(items, query);
    }

    /// 文件已经删掉了：把这一行从面板里拿掉，面板留着继续挑
    pub fn forget_session(&mut self, id: &str) {
        if let Some(panel) = &mut self.sessions {
            panel.forget(id);
        }
    }

    /// 删除失败之类的事情写在面板页脚，别把整块对话顶上去
    pub fn sessions_notice(&mut self, text: impl Into<String>, error: bool) {
        if let Some(panel) = &mut self.sessions {
            panel.set_notice(text, error);
        }
    }

    fn on_sessions_key(&mut self, key: KeyEvent) -> Action {
        let Some(panel) = &mut self.sessions else {
            return Action::None;
        };
        match panel.on_key(key) {
            PanelEvent::None => Action::None,
            PanelEvent::Close => {
                self.sessions = None;
                Action::None
            }
            PanelEvent::Resume(id) => {
                self.sessions = None;
                Action::ResumeSession(id)
            }
            // 面板不自己动磁盘：这里只表达意图，删成功之后调用 forget_session
            PanelEvent::Delete(id) => Action::DeleteSession(id),
        }
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

    /// 打开语言选择，光标停在当前语言上
    pub fn open_lang_picker(&mut self) {
        let current = crate::i18n::current();
        let selected = LANGS.iter().position(|&l| l == current).unwrap_or(0);
        self.picker = Some(Picker {
            kind: PickerKind::Lang,
            selected,
            subject: String::new(),
        });
    }

    /// 打开信任浮层。`subject` 是显示用的路径，光标默认停在「否」上——
    /// 需要用户主动往上挪一格才算信任。
    pub fn open_trust_picker(&mut self, subject: String) {
        self.picker = Some(Picker {
            kind: PickerKind::Trust,
            selected: 1,
            subject,
        });
    }

    /// 打开工作模式选择，光标停在当前模式上
    pub fn open_mode_picker(&mut self, current: crate::mode::Mode) {
        let selected = crate::mode::Mode::ALL
            .iter()
            .position(|&m| m == current)
            .unwrap_or(0);
        self.picker = Some(Picker {
            kind: PickerKind::Mode,
            selected,
            subject: String::new(),
        });
    }

    /// 打开主题选择，光标停在当前主题上
    pub fn open_theme_picker(&mut self, current: crate::theme::Theme) {
        let selected = crate::theme::Theme::ALL
            .iter()
            .position(|&t| t == current)
            .unwrap_or(0);
        self.picker = Some(Picker {
            kind: PickerKind::Theme,
            selected,
            subject: String::new(),
        });
    }

    /// 打开字形选择
    pub fn open_font_picker(&mut self, current: crate::theme::FontStyle) {
        let selected = crate::theme::FontStyle::ALL
            .iter()
            .position(|&f| f == current)
            .unwrap_or(0);
        self.picker = Some(Picker {
            kind: PickerKind::Font,
            selected,
            subject: String::new(),
        });
    }

    /// 打开权限模式选择，光标停在当前模式上
    pub fn open_permission_picker(&mut self, current: crate::tools::PermissionMode) {
        let selected = crate::tools::PermissionMode::ALL
            .iter()
            .position(|&m| m == current)
            .unwrap_or(0);
        self.picker = Some(Picker {
            kind: PickerKind::Permission,
            selected,
            subject: String::new(),
        });
    }

    /// 打开强调色选择，光标停在当前色上
    pub fn open_accent_picker(&mut self, current: crate::theme::Accent) {
        let selected = crate::theme::Accent::ALL
            .iter()
            .position(|&a| a == current)
            .unwrap_or(0);
        self.picker = Some(Picker {
            kind: PickerKind::Accent,
            selected,
            subject: String::new(),
        });
    }

    pub fn close_picker(&mut self) {
        self.picker = None;
    }

    fn on_form_key(&mut self, key: KeyEvent) -> Action {
        let Some(form) = self.form.as_mut() else {
            return Action::None;
        };
        match form.on_key(key) {
            FormEvent::None => Action::None,
            FormEvent::Cancel => {
                self.form = None;
                self.push(Entry::Info(tr("已取消，配置没有改动").into()));
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
        if self.confirm.is_some() || self.review.is_some() {
            return;
        }
        // 面板开着时粘贴进搜索框，比直接丢掉更符合直觉
        if let Some(panel) = &mut self.sessions {
            panel.paste(text);
            return;
        }
        match self.form.as_mut() {
            Some(form) => form.paste(text),
            None => self.editor.insert_str(text),
        }
    }

    /// 当前模式专属的快捷键提示；没有则用默认提示
    pub fn hint(&self) -> Option<&'static str> {
        if self.confirm.is_some() {
            return Some(confirm_hint());
        }
        if self.review.is_some() {
            return Some(review_hint());
        }
        if self.sessions.is_some() {
            return Some(sessions_hint());
        }
        if self.picker.is_some() {
            return Some(picker_hint());
        }
        self.form.as_ref().map(ModelForm::hint)
    }

    // ---- AI 流式事件 ----

    pub fn begin_request(&mut self, input: &str) {
        self.push(Entry::User(input.to_string()));
        self.transcript.start_assistant();
        self.busy = Some(tr("等待响应…").into());
    }

    pub fn on_content(&mut self, s: &str) {
        self.transcript.append_content(s);
        self.busy = Some(tr("生成中…").into());
    }

    pub fn on_reasoning(&mut self, s: &str) {
        self.transcript.append_reasoning(s);
        self.busy = Some(tr("思考中…").into());
    }

    pub fn end_request(&mut self) {
        self.busy = None;
        // 请求结束了，没有后台任务还在等这个答复
        self.confirm = None;
    }

    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }
}

/// 演示：用示例数据走一遍扫描与修复界面
pub fn start_demo(app: &mut AppState) {
    let (findings, fixes) = review::demo_data();
    app.push(Entry::Warn(
        tr("演示模式：以下是示例数据，不会读取或修改任何文件").into(),
    ));
    app.start_review(&findings, &fixes, Some(tr("演示数据").into()));
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
        let item = |id: &str, title: &str, body: &str| {
            SessionItem::from_catalog(
                crate::sessions::CatalogEntry {
                    summary: crate::sessions::Summary {
                        id: id.into(),
                        title: title.into(),
                        updated: 0,
                        turns: 1,
                        model: "deepseek-flash".into(),
                    },
                    body: body.into(),
                },
                "刚刚".into(),
            )
        };
        vec![
            item("s1", "如何防止 SQL 注入", "用参数化查询"),
            item("s2", "聊聊天气", "今天不错"),
        ]
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
    fn an_empty_session_list_does_not_open_the_panel() {
        let mut app = AppState::new();
        app.open_sessions(Vec::new(), "");
        assert!(app.sessions.is_none());
        assert!(app.choice().is_none(), "面板不是选择区，不该占用下面的空间");
    }

    #[test]
    fn the_panel_takes_the_keyboard_and_enter_resumes_the_highlighted_session() {
        let mut app = AppState::new();
        app.open_sessions(session_items(), "");
        assert!(app.sessions.is_some());
        assert_eq!(app.hint(), Some(sessions_hint()));
        assert!(!app.input_active(), "底部输入框让位给面板");

        app.on_key(key(KeyCode::Down));
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::ResumeSession("s2".into())
        );
        assert!(app.sessions.is_none(), "选中之后面板自己关掉");
    }

    #[test]
    fn typing_in_the_panel_filters_instead_of_reaching_the_input_box() {
        let mut app = AppState::new();
        app.open_sessions(session_items(), "");
        type_str(&mut app, "天气");
        assert!(app.editor.is_empty(), "不会打到下面的输入框里");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::ResumeSession("s2".into())
        );
    }

    #[test]
    fn esc_closes_the_panel_without_touching_the_input_box() {
        let mut app = AppState::new();
        app.open_sessions(session_items(), "");
        assert_eq!(app.on_key(key(KeyCode::Esc)), Action::None);
        assert!(app.sessions.is_none());
        assert!(app.editor.is_empty());
    }

    #[test]
    fn deleting_a_session_takes_two_presses_and_leaves_the_panel_open() {
        let mut app = AppState::new();
        app.open_sessions(session_items(), "");
        assert_eq!(
            app.on_key(key(KeyCode::Delete)),
            Action::None,
            "第一次只是问一句"
        );
        assert_eq!(
            app.on_key(key(KeyCode::Delete)),
            Action::DeleteSession("s1".into())
        );
        assert!(app.sessions.is_some(), "文件删掉之前面板不能关");
        // 调用方删成功后告诉面板，这一行就没了
        app.forget_session("s1");
        assert!(app.sessions.is_some());
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::ResumeSession("s2".into())
        );
    }

    #[test]
    fn the_mode_chord_is_ignored_while_the_panel_is_open() {
        let mut app = AppState::new();
        app.open_sessions(session_items(), "");
        app.on_key(ctrl('p'));
        assert_eq!(app.mode, Mode::Plan);
        assert!(app.sessions.is_some());
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
        assert_eq!(
            app.choice().unwrap().items[1].1,
            "sk-abc",
            "the key is shown in plain text"
        );
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
        app.open_sessions(session_items(), "");
        app.on_paste("天气");
        assert!(app.editor.is_empty(), "粘贴进的是搜索框");
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::ResumeSession("s2".into())
        );

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
        app.open_sessions(session_items(), "");
        assert!(!app.input_active());
    }

    #[test]
    fn a_real_review_hands_the_accepted_fixes_to_the_caller() {
        let (findings, fixes) = crate::review::demo_data();
        let mut app = AppState::new();
        app.start_review(&findings, &fixes, None);
        // 第一项按 y 应用，第二项起「全部应用」
        assert_eq!(app.on_key(key(KeyCode::Char('y'))), Action::None);
        match app.on_key(key(KeyCode::Char('a'))) {
            // 第一条按 y、后两条按 a，三份都要交给调用方去写
            Action::ApplyFixes(f) => assert_eq!(f.len(), 3),
            other => panic!("{other:?}"),
        }
        assert!(app.review.is_none());
    }

    #[test]
    fn a_demo_review_never_asks_to_write_files() {
        let mut app = AppState::new();
        start_demo(&mut app);
        assert_eq!(
            app.on_key(key(KeyCode::Char('a'))),
            Action::None,
            "演示数据不能落盘"
        );
        assert!(texts(&app).contains("演示模式"));
    }

    #[test]
    fn review_with_no_pairs_does_not_open_a_menu() {
        let mut app = AppState::new();
        app.start_review(&[], &[], None);
        assert!(app.review.is_none());
        assert!(texts(&app).contains("没有可确认的修复"));
    }

    // ---- 工作模式 ----

    #[test]
    fn the_mode_chord_cycles_plan_confirm_auto() {
        let mut app = AppState::new();
        assert_eq!(app.mode, Mode::Plan, "默认先看不动");

        app.on_key(key(KeyCode::BackTab)); // Shift-Tab
        assert_eq!(app.mode, Mode::Confirm);
        app.on_key(ctrl('p'));
        assert_eq!(app.mode, Mode::Auto);
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT));
        assert_eq!(app.mode, Mode::Plan, "绕回第一档");
        assert!(texts(&app).contains("已切换到「计划」模式"));
    }

    #[test]
    fn plain_tab_still_completes_a_command_instead_of_switching_modes() {
        let mut app = AppState::new();
        type_str(&mut app, "/he");
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.editor.text(), "/help ");
        assert_eq!(app.mode, Mode::Plan);
    }

    #[test]
    fn the_mode_chord_is_ignored_while_an_overlay_is_open() {
        let mut app = AppState::new();
        app.ask_confirm("a.rs".into(), String::new(), "x".into());
        app.on_key(ctrl('p'));
        assert_eq!(app.mode, Mode::Plan, "确认的时候换模式只会更糊涂");
        assert!(app.choice().is_some(), "确认卡片不该被抢走");
    }

    #[test]
    fn setting_the_same_mode_twice_says_nothing() {
        let mut app = AppState::new();
        app.set_mode(Mode::Plan);
        assert!(!texts(&app).contains("已切换"));
        app.set_mode(Mode::Auto);
        assert!(texts(&app).contains("已切换到「全自动」模式"));
    }

    #[test]
    fn only_the_commands_that_actually_work_are_listed() {
        let names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
        // 扫描、修复、导出现在都能真的干活
        for done in ["/scan", "/fix", "/report"] {
            assert!(names.contains(&done), "{done} 已经能用，命令该在列表里");
        }
        assert!(names.contains(&"/mode"));
    }

    // ---- 逐条确认 ----

    #[test]
    fn confirm_card_asks_about_the_file_and_takes_shortcuts() {
        let mut app = AppState::new();
        app.ask_confirm("src/db.py".into(), "a\n".into(), "b\n".into());

        let c = app.choice().unwrap();
        assert_eq!(c.title, "应用对 src/db.py 的改动？");
        assert_eq!(c.items.len(), 3);
        assert_eq!(c.items[0].0, "应用");
        assert!(!app.input_active());
        assert_eq!(app.hint(), Some(confirm_hint()));
        // 改动本身已经摊在对话区里，不用再点开才知道改了什么
        assert!(texts(&app).contains("src/db.py"));

        assert_eq!(
            app.on_key(key(KeyCode::Char('y'))),
            Action::Answer(Answer::Apply)
        );
        assert_eq!(
            app.on_key(key(KeyCode::Char('n'))),
            Action::Answer(Answer::Skip)
        );
        assert_eq!(
            app.on_key(key(KeyCode::Char('a'))),
            Action::Answer(Answer::Always)
        );
        assert_eq!(app.on_key(key(KeyCode::Esc)), Action::Answer(Answer::Skip));
    }

    #[test]
    fn confirm_menu_navigation_wraps_and_enter_uses_the_selection() {
        let mut app = AppState::new();
        app.ask_confirm("a.rs".into(), String::new(), "x".into());
        app.on_key(key(KeyCode::Up)); // 从第一项绕到最后一项
        assert_eq!(app.choice().unwrap().selected, 2);
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::Answer(Answer::Always)
        );
    }

    #[test]
    fn skipping_a_change_leaves_a_note_and_closes_the_card() {
        let mut app = AppState::new();
        app.ask_confirm("a.rs".into(), "x\n".into(), "y\n".into());
        app.close_confirm(Answer::Skip);
        assert!(app.choice().is_none());
        assert!(app.input_active());
        assert!(texts(&app).contains("已跳过 a.rs"));
    }

    #[test]
    fn typing_is_ignored_while_confirming() {
        let mut app = AppState::new();
        app.ask_confirm("a.rs".into(), String::new(), "x".into());
        type_str(&mut app, "xyz");
        assert!(app.editor.is_empty());
        app.on_paste("pasted");
        assert!(app.editor.is_empty());
    }

    #[test]
    fn finishing_the_request_closes_a_stale_confirm_card() {
        let mut app = AppState::new();
        app.begin_request("hi");
        app.ask_confirm("a.rs".into(), String::new(), "x".into());
        app.end_request();
        assert!(app.choice().is_none(), "任务结束了，没人再等这个答复");
    }

    #[test]
    fn the_language_picker_offers_both_languages_and_returns_the_selection() {
        let mut app = AppState::new();
        app.open_lang_picker();
        let choice = app.choice().expect("打开后应当出现选择区");
        assert_eq!(choice.items.len(), LANGS.len());
        // 光标停在当前语言上，而不是永远停在第一项
        let current = crate::i18n::current();
        assert_eq!(
            choice.selected,
            LANGS.iter().position(|&l| l == current).unwrap()
        );
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::SetLang(LANGS[choice.selected])
        );
    }

    #[test]
    fn the_language_picker_wraps_and_esc_cancels() {
        let mut app = AppState::new();
        app.open_lang_picker();
        let start = app.choice().unwrap().selected;
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.choice().unwrap().selected, (start + 1) % LANGS.len());
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.choice().unwrap().selected, start);
        assert_eq!(app.on_key(key(KeyCode::Esc)), Action::None);
        assert!(app.choice().is_none(), "Esc 之后选择区应当关掉");
        assert!(app.input_active(), "关掉之后输入框要恢复");
    }
    #[test]
    fn the_trust_picker_defaults_to_no() {
        let mut app = AppState::new();
        app.open_trust_picker("~/proj".into());
        let choice = app.choice().expect("信任浮层应当出现");
        assert_eq!(choice.selected, 1, "光标要停在「否」上，信任得自己往上挪");
        assert_eq!(choice.items.len(), 2);
        assert!(choice.title.contains("~/proj"));
        // 直接回车 = 不信任
        assert_eq!(app.on_key(key(KeyCode::Enter)), Action::Trust(false));
    }

    #[test]
    fn trusting_needs_deliberate_input() {
        let mut app = AppState::new();
        app.open_trust_picker("/tmp".into());
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.on_key(key(KeyCode::Enter)), Action::Trust(true));
    }

    #[test]
    fn esc_on_the_trust_picker_does_not_grant_trust() {
        let mut app = AppState::new();
        app.open_trust_picker("/tmp".into());
        assert_eq!(app.on_key(key(KeyCode::Esc)), Action::Trust(false));
        assert!(app.choice().is_none(), "浮层要关掉");
    }

    #[test]
    fn the_accent_picker_returns_the_highlighted_colour() {
        use crate::theme::Accent;
        let mut app = AppState::new();
        app.open_accent_picker(Accent::Ocean);
        let choice = app.choice().expect("强调色浮层应当出现");
        assert_eq!(
            choice.selected,
            Accent::ALL
                .iter()
                .position(|&a| a == Accent::Ocean)
                .unwrap()
        );
        assert_eq!(choice.items.len(), Accent::ALL.len());
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::SetAccent(Accent::Ocean)
        );
    }
    #[test]
    fn the_theme_picker_returns_the_highlighted_theme() {
        use crate::theme::Theme;
        let mut app = AppState::new();
        app.open_theme_picker(Theme::Dark);
        let choice = app.choice().expect("主题浮层应当出现");
        assert_eq!(choice.items.len(), Theme::ALL.len());
        assert_eq!(choice.selected, 0);
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::SetTheme(Theme::Dark)
        );
        // 下移一格再回车，应当给出另一套主题
        app.open_theme_picker(Theme::Dark);
        app.on_key(key(KeyCode::Down));
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::SetTheme(Theme::Light)
        );
    }

    #[test]
    fn the_permission_picker_lists_all_three_modes() {
        use crate::tools::PermissionMode;
        let mut app = AppState::new();
        app.open_permission_picker(PermissionMode::Full);
        let choice = app.choice().expect("权限浮层应当出现");
        assert_eq!(choice.items.len(), PermissionMode::ALL.len());
        // 光标落在当前模式上，不是永远第一项
        assert_eq!(choice.selected, 2);
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::SetPermission(PermissionMode::Full)
        );
        app.open_permission_picker(PermissionMode::ReadOnly);
        app.on_key(key(KeyCode::Down));
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::SetPermission(PermissionMode::Editable)
        );
    }

    #[test]
    fn the_font_picker_returns_the_highlighted_font() {
        use crate::theme::FontStyle;
        let mut app = AppState::new();
        app.open_font_picker(FontStyle::Anime);
        assert_eq!(app.choice().unwrap().selected, 1);
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::SetFont(FontStyle::Anime)
        );
    }
    #[test]
    fn the_mode_picker_returns_the_highlighted_mode() {
        use crate::mode::Mode;
        let mut app = AppState::new();
        app.open_mode_picker(Mode::Confirm);
        let choice = app.choice().expect("模式浮层应当出现");
        assert_eq!(choice.items.len(), Mode::ALL.len());
        assert_eq!(choice.selected, 1);
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::SetMode(Mode::Confirm)
        );
        app.open_mode_picker(Mode::Plan);
        app.on_key(key(KeyCode::Down));
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::SetMode(Mode::Confirm)
        );
    }
}
