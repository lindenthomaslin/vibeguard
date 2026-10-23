//! 全屏界面的绘制：把状态画到 ratatui 的 `Frame` 上。

use super::layout::areas;
use super::sessions::{SessionItem, SessionsPanel};
use crate::chat::{Scroll, Transcript};
use crate::i18n::{pick, tr};
use crate::mode::Mode;
use crate::theme::Look;
use crate::{icon, input};
use ratatui::Frame;
use ratatui::layout::Alignment;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph};
use ratatui_image::StatefulImage;
use ratatui_image::protocol::StatefulProtocol;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
    /// 历史对话面板：盖在对话区上的一层浮层
    pub sessions: Option<&'a SessionsPanel>,
    /// 逐条确认中：对话区停在最后一个条目的开头，小终端上图标栏收成一行
    pub focus_last: bool,
    /// 正在等待 AI 时的提示文字
    pub busy: Option<&'a str>,
    /// 动画帧计数
    pub tick: usize,
    pub status: &'a str,
    /// 当前工作模式，画在状态栏最左边
    pub mode: Mode,
    /// 配色和字形
    pub look: Look,
    /// 已输入文本的 token 数估算
    pub input_tokens: usize,
    pub scroll: &'a mut Scroll,
    /// 标题栏头像的原生图片；`None` 时退回字符画
    pub avatar: Option<&'a mut StatefulProtocol>,
    /// 启动画面的原生图片；`None` 时退回徽标 + 字标
    pub banner: Option<&'a mut StatefulProtocol>,
    /// 是否显示启动画面（发出第一条消息后就不显示了）
    pub show_banner: bool,
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

/// 按显示宽度截断，末尾补省略号。行尾硬切会切在半个字上，很难看。
fn clip_to(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width.saturating_sub(1) {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
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
    // 确认卡片和历史对话面板都需要纵向空间：矮终端上把图标栏收成一行让位
    let wants_room = p.focus_last || p.sessions.is_some();
    let compact = wants_room && area.height < REVIEW_FULL_HEADER_MIN_HEIGHT;
    let a = areas(area, clamp_u16(lay.rows.len()), choice_rows, compact);

    // 启动画面按终端高度选一档，太矮就没有
    let native = p.banner.is_some();
    let splash = p.show_banner.then(|| splash(area.height, native)).flatten();
    draw_header(frame, a.header, &mut p);
    draw_body(frame, a.body, &mut p, splash);
    if let Some(c) = p.choice {
        draw_choice(frame, a.choice, c, &p.look);
    }
    draw_input(
        frame,
        a.input,
        &lay,
        p.busy.is_some(),
        p.input_label,
        p.input_active,
        &p.look,
    );
    if a.status.height > 0 {
        let status_text = if p.input_tokens > 0 {
            format!("{} · Tokens: {}", p.status, p.input_tokens)
        } else {
            p.status.to_string()
        };
        // 左下角常驻模式标签：反色小块，一眼能看出 AI 现在能不能动文件
        // plan 是「强制只读」：光写 plan 不够，得让用户一眼看出这个模式不管
        // permission_mode 设成什么都改不了文件
        let chip_text = if p.mode == Mode::Plan {
            format!(
                " {} {} · {} ",
                p.mode.glyph(),
                p.mode.label(),
                crate::i18n::t("只读", "read-only")
            )
        } else {
            format!(" {} {} ", p.mode.glyph(), p.mode.label())
        };
        let chip = Span::styled(
            chip_text,
            Style::new()
                .fg(Color::Black)
                .bg(p.look.chip(p.mode))
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                chip,
                Span::raw(" "),
                Span::styled(status_text, p.look.muted()),
            ])),
            a.status,
        );
    }
    // 浮层最后画，盖住底下的对话；光标也由它接管。
    // 只占对话区：图标栏和输入框留在原地，看起来才像「盖在对话上」而不是换了屏
    if let Some(panel) = p.sessions {
        draw_sessions(frame, a.body, panel, &p.look);
    }
}

fn draw_header(frame: &mut Frame, rect: Rect, p: &mut Props<'_>) {
    if rect.height == 0 || rect.width == 0 {
        return;
    }
    let title = format!("VibeGuard v{}", env!("CARGO_PKG_VERSION"));
    if rect.height < icon::AVATAR_ROWS + 2 {
        let line = Line::from(vec![
            Span::styled(format!("{} ", p.look.glyphs.thinking), p.look.accent_bold()),
            Span::styled(title, Style::new().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(" · {} · thinking {}", p.model, p.thinking),
                p.look.muted(),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), Rect { height: 1, ..rect });
        return;
    }

    let block = Block::bordered()
        .border_type(p.look.glyphs.border)
        .border_style(p.look.accent().add_modifier(Modifier::DIM));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let icon_rect =
        Rect::new(inner.x + 1, inner.y, icon::AVATAR_COLS, icon::AVATAR_ROWS).intersection(inner);
    // 终端支持原生图片就直接贴原图，否则用四分之一方块字符拼
    match p.avatar.as_deref_mut() {
        Some(proto) => frame.render_stateful_widget(StatefulImage::new(), icon_rect, proto),
        None => frame.render_widget(Paragraph::new(icon::avatar_lines()), icon_rect),
    }

    let text_x = inner.x + 1 + icon::AVATAR_COLS + 2;
    let text = Rect::new(
        text_x,
        inner.y,
        inner.right().saturating_sub(text_x),
        inner.height,
    );
    let muted = p.look.muted();
    let lines = vec![
        Line::styled(title, Style::new().add_modifier(Modifier::BOLD)),
        Line::styled(
            crate::i18n::t(
                "AI 生成代码的安全扫描与修复",
                "Security review for AI-written code",
            ),
            muted,
        ),
        Line::from(vec![
            Span::styled("model ", muted),
            Span::raw(p.model.to_string()),
            Span::styled(" · thinking ", muted),
            Span::raw(p.thinking.to_string()),
        ]),
        Line::from(vec![
            Span::styled("cwd   ", muted),
            Span::raw(p.cwd.to_string()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), text);
}

fn draw_body(frame: &mut Frame, rect: Rect, p: &mut Props<'_>, splash: Option<Splash>) {
    if rect.height == 0 || rect.width == 0 {
        return;
    }
    let (mut lines, last_start) = p.transcript.lines_marked(rect.width as usize, &p.look);
    if let Some(msg) = p.busy {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        let frames = p.look.glyphs.spinner;
        lines.push(Line::from(vec![
            Span::styled(frames[p.tick % frames.len()], p.look.accent()),
            Span::styled(format!(" {msg}"), p.look.muted()),
        ]));
    }

    // 启动画面和对话一起垂直居中：一开始屏幕上只有几行字，顶在角落会显得空。
    // 内容多到放不下时 pad 归零，对话照常滚动。
    let mut rect = rect;
    if let Some(splash) = splash {
        let group = splash.rows().saturating_add(clamp_u16(lines.len()));
        let pad = rect.height.saturating_sub(group) / 2;
        let area = Rect::new(
            rect.x,
            rect.y + pad,
            rect.width,
            splash.rows().min(rect.height),
        );
        draw_splash(frame, area, p, &splash);
        let top = area.y + area.height;
        rect = Rect::new(rect.x, top, rect.width, rect.bottom().saturating_sub(top));
    }

    let height = rect.height as usize;
    if height == 0 {
        return;
    }
    let mut top = p.scroll.resolve(lines.len(), height);
    // 逐条确认时让当前这一条的开头可见，而不是滚到它的末尾；之后用户仍可自行滚动
    if p.focus_last && p.scroll.is_following() {
        p.scroll.pin(last_start);
        top = p.scroll.resolve(lines.len(), height);
    }
    let end = (top + height).min(lines.len());
    frame.render_widget(Paragraph::new(lines[top..end].to_vec()), rect);
}

/// 终端低于这个高度就不放启动画面了：再矮就挤掉欢迎语，得不偿失
const SPLASH_MIN_HEIGHT: u16 = 30;

/// 启动画面：有图形协议就贴原图，没有就用四分之一方块拼，再不行就只画字标
enum Splash {
    /// 终端原生图片（列, 行）
    Image(u16, u16),
    /// 字符画（列, 行）
    Art(u16, u16),
    /// 只画徽标 + 字标
    Wordmark,
}

/// 启动画面用哪一种。原生图片最清楚；字符格要多给几十列才认得出，
/// 终端不够高就退到字标——总比糊一团强。
fn splash(term_height: u16, native: bool) -> Option<Splash> {
    if native {
        return icon::logo_size(term_height, true).map(|(cols, rows)| Splash::Image(cols, rows));
    }
    if let Some((cols, rows)) = icon::logo_size(term_height, false) {
        return Some(Splash::Art(cols, rows));
    }
    (term_height >= SPLASH_MIN_HEIGHT).then_some(Splash::Wordmark)
}

impl Splash {
    /// 整块占的高度：图 + 一行说明，或者徽标 + 字标 + 说明
    fn rows(&self) -> u16 {
        match self {
            Splash::Image(_, rows) | Splash::Art(_, rows) => rows + 1,
            Splash::Wordmark => icon::AVATAR_ROWS + 2,
        }
    }
}

/// 品牌块：水平居中，图（或徽标）下面写一句说明。
fn draw_splash(frame: &mut Frame, area: Rect, p: &mut Props<'_>, splash: &Splash) {
    let text_y = match *splash {
        Splash::Image(cols, rows) => {
            let img = Rect::new(
                area.x + area.width.saturating_sub(cols) / 2,
                area.y,
                cols.min(area.width),
                rows.min(area.height),
            );
            if let Some(proto) = p.banner.as_deref_mut() {
                frame.render_stateful_widget(StatefulImage::new(), img, proto);
            }
            img.y + img.height
        }
        Splash::Art(cols, rows) => {
            let art = Rect::new(
                area.x + area.width.saturating_sub(cols) / 2,
                area.y,
                cols.min(area.width),
                rows.min(area.height),
            );
            frame.render_widget(Paragraph::new(icon::logo_lines((cols, rows))), art);
            art.y + art.height
        }
        Splash::Wordmark => {
            let mark = Rect::new(
                area.x + area.width.saturating_sub(icon::AVATAR_COLS) / 2,
                area.y,
                icon::AVATAR_COLS.min(area.width),
                icon::AVATAR_ROWS.min(area.height),
            );
            frame.render_widget(Paragraph::new(icon::avatar_lines()), mark);
            mark.y + mark.height
        }
    };
    if text_y >= area.bottom() {
        return;
    }
    let text = Rect::new(area.x, text_y, area.width, area.bottom() - text_y);
    frame.render_widget(
        Paragraph::new(caption_lines(splash, &p.look)).alignment(Alignment::Center),
        text,
    );
}

/// 说明文字。图上（或字符画里）已经有手写的 vibeguard，就不用再写一遍；
/// 只有字标那一档需要把名字打出来——终端里没有大字号，靠拉开字距撑门面。
fn caption_lines(splash: &Splash, look: &Look) -> Vec<Line<'static>> {
    let tagline = Line::styled(
        crate::i18n::t(
            "AI 生成代码的安全扫描与修复",
            "Security review for AI-written code",
        ),
        look.muted(),
    );
    match splash {
        Splash::Image(..) | Splash::Art(..) => vec![tagline],
        Splash::Wordmark => vec![
            Line::styled(
                "v i b e g u a r d",
                Style::new().add_modifier(Modifier::BOLD),
            ),
            tagline,
        ],
    }
}

fn draw_choice(frame: &mut Frame, rect: Rect, c: &Choice, look: &Look) {
    if rect.height < 3 {
        return;
    }
    let block = Block::bordered()
        .border_type(look.glyphs.border)
        .border_style(look.accent())
        .title(Span::styled(format!(" {} ", c.title), look.accent()));
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
                Line::styled(format!("{} {text}", look.glyphs.user), look.accent_bold())
            } else {
                Line::styled(format!("  {text}"), look.muted())
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// 浮层的位置和大小：宽度固定档位，高度跟着条数走（过滤时不会忽大忽小，
/// 因为用的是总数而不是命中数），四周一定留边距，终端再小也不会溢出
fn panel_rect(area: Rect, items: usize) -> Rect {
    let want_h = clamp_u16(items).saturating_add(5).clamp(8, 22);
    let w = area.width.saturating_sub(4).min(88).max(area.width.min(24));
    let h = want_h.min(area.height).max(area.height.min(8));
    Rect::new(
        area.x + area.width.saturating_sub(w) / 2,
        area.y + area.height.saturating_sub(h) / 2,
        w,
        h,
    )
}

/// 搜索框前面的标签，宽度固定 5 列（中文各占 2 列 + 1 个空格）
fn search_label() -> &'static str {
    tr("搜索 ")
}
const SEARCH_LABEL_W: u16 = 5;

fn draw_sessions(frame: &mut Frame, area: Rect, panel: &SessionsPanel, look: &Look) {
    let rect = panel_rect(area, panel.total());
    // 先擦掉底下的对话，否则浮层边缘会露出半行字
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(look.glyphs.border)
        .border_style(look.accent())
        .title(Span::styled(format!(" {} ", panel.title()), look.accent()));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height < 3 || inner.width < 10 {
        return;
    }

    // 搜索行
    let (chars, cursor) = panel.query();
    let lay = input::layout(
        chars,
        cursor,
        (inner.width.saturating_sub(SEARCH_LABEL_W) as usize).max(2),
    );
    let row = lay.rows.get(lay.cursor.0).cloned().unwrap_or_default();
    let empty = chars.is_empty();
    let mut search = vec![Span::styled(search_label(), look.muted())];
    if empty {
        search.push(Span::styled(tr("输入关键词过滤"), look.muted()));
    } else {
        search.push(Span::raw(row));
    }
    frame.render_widget(
        Paragraph::new(Line::from(search)),
        Rect { height: 1, ..inner },
    );

    let rule = inner.height >= 6;
    if rule {
        frame.render_widget(
            Paragraph::new(Line::styled("─".repeat(inner.width as usize), look.muted())),
            Rect {
                x: inner.x,
                y: inner.y + 1,
                width: inner.width,
                height: 1,
            },
        );
    }

    let foot_y = inner.y + inner.height - 1;
    let list_top = inner.y + 1 + u16::from(rule);
    let list_h = foot_y.saturating_sub(list_top) as usize;

    // 列表
    let needle = panel.needle();
    let items: Vec<&SessionItem> = panel.visible().collect();
    let when_w = items.iter().map(|i| i.when.width()).max().unwrap_or(0);
    let mut lines: Vec<Line> = if items.is_empty() {
        vec![Line::styled(tr("没有匹配的对话"), look.muted())]
    } else {
        let start = window_start(panel.selected(), list_h.max(1));
        items
            .iter()
            .enumerate()
            .skip(start)
            .take(list_h)
            .map(|(i, item)| {
                let on = i == panel.selected();
                let lead = if on { look.accent_bold() } else { look.muted() };
                let mut spans = vec![
                    Span::styled(
                        format!("{} ", if on { look.glyphs.user } else { " " }),
                        lead,
                    ),
                    Span::styled(pad_to(&item.when, when_w + 2), lead),
                    Span::styled(
                        item.title.clone(),
                        if on { look.accent_bold() } else { Style::new() },
                    ),
                    Span::styled(
                        pick(
                            format!(" · {} 轮", item.turns),
                            format!(" · {} rounds", item.turns),
                        ),
                        look.muted(),
                    ),
                ];
                if let Some(snip) = item.snippet(&needle) {
                    // 剩下的列数：前缀、时间、标题、轮数都占掉了，摘要只能用余下的
                    let used = 2 + when_w + 2 + item.title.width() + 6;
                    let avail = (inner.width as usize).saturating_sub(used + 3);
                    if avail >= 10 {
                        spans.push(Span::styled(
                            format!(" · {}", clip_to(&snip, avail)),
                            look.muted(),
                        ));
                    }
                }
                Line::from(spans)
            })
            .collect()
    };
    if list_h == 0 {
        lines.clear();
    }
    frame.render_widget(
        Paragraph::new(lines),
        Rect {
            x: inner.x,
            y: list_top,
            width: inner.width,
            height: clamp_u16(list_h),
        },
    );

    // 页脚：删除确认 > 刚才的结果 > 默认快捷键
    let width = inner.width as usize;
    let (foot_text, foot_style) = if let Some(item) = panel.pending() {
        (
            pick(
                format!("删除「{}」？再按 Del 确认 · Esc 取消", item.title),
                format!("Delete \"{}\"? Press Del again · Esc cancel", item.title),
            ),
            look.error(),
        )
    } else if let Some(note) = panel.notice() {
        let style = if note.error {
            look.error()
        } else {
            look.muted()
        };
        (note.text.clone(), style)
    } else {
        (
            pick(
                format!(
                    "共 {} 个 · 输入过滤 · Enter 打开 · Del 删除 · Esc 关闭",
                    panel.total()
                ),
                format!(
                    "{} total · type to filter · Enter open · Del delete · Esc close",
                    panel.total()
                ),
            ),
            look.muted(),
        )
    };
    let foot = Line::styled(clip_to(&foot_text, width), foot_style);
    frame.render_widget(
        Paragraph::new(foot),
        Rect {
            x: inner.x,
            y: foot_y,
            width: inner.width,
            height: 1,
        },
    );

    // 光标交给搜索框：它在最前面画，最后设位置才不会被别处覆盖
    let x =
        (inner.x + SEARCH_LABEL_W + clamp_u16(lay.cursor.1)).min(inner.right().saturating_sub(1));
    frame.set_cursor_position((x, inner.y));
}

fn draw_input(
    frame: &mut Frame,
    rect: Rect,
    lay: &input::Layout,
    busy: bool,
    label: Option<&str>,
    active: bool,
    look: &Look,
) {
    if rect.height == 0 {
        return;
    }
    let border = if busy || !active {
        look.muted()
    } else {
        look.accent()
    };
    let mut block = Block::bordered()
        .border_type(look.glyphs.border)
        .border_style(border)
        .padding(Padding::horizontal(1));
    if let Some(label) = label {
        block = block.title(Span::styled(format!(" {label} "), look.accent()));
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
                Span::styled(format!("{} ", look.glyphs.user), look.accent_bold())
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
    use crate::theme::{FontStyle, Theme};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    struct Fixture {
        transcript: Transcript,
        scroll: Scroll,
        input: Vec<char>,
        choice: Option<Choice>,
        sessions: Option<SessionsPanel>,
        focus_last: bool,
        input_label: Option<&'static str>,
        input_active: bool,
        busy: Option<String>,
        show_banner: bool,
        mode: Mode,
        look: Look,
        avatar_proto: Option<StatefulProtocol>,
        banner_proto: Option<StatefulProtocol>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                transcript: Transcript::default(),
                scroll: Scroll::default(),
                input: Vec::new(),
                choice: None,
                sessions: None,
                focus_last: false,
                input_label: None,
                input_active: true,
                busy: None,
                show_banner: false,
                mode: Mode::default(),
                look: Look::default(),
                avatar_proto: None,
                banner_proto: None,
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
                        sessions: self.sessions.as_ref(),
                        focus_last: self.focus_last,
                        input_label: self.input_label,
                        input_active: self.input_active,
                        busy: self.busy.as_deref(),
                        tick: 0,
                        status: "Enter 发送",
                        mode: self.mode,
                        look: self.look,
                        input_tokens: 0,
                        scroll: &mut self.scroll,
                        avatar: self.avatar_proto.as_mut(),
                        banner: self.banner_proto.as_mut(),
                        show_banner: self.show_banner,
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
        assert!(
            rows.iter()
                .take(6)
                .any(|r| r.contains('█') || r.contains('▀'))
        );
        // 状态栏在最后一行，输入框（上边框、文字、下边框）占倒数第 2 到第 4 行
        assert!(rows[26].starts_with('┌'));
        assert!(rows[27].contains('›'));
        assert!(rows[28].starts_with('└'));
        assert!(rows[29].contains("Enter"));
    }

    /// 左下角常驻模式标签：位置、文字和颜色都跟着模式走
    #[test]
    fn the_mode_chip_sits_at_the_bottom_left() {
        let squeeze = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
        let mut fx = Fixture::new();
        fx.mode = Mode::Plan;
        let term = fx.render(80, 30);
        let rows = screen(&term);
        // plan 会强制只读，标签上直接写出来：否则用户设了 full 还是被拒，会一头雾水
        assert!(
            squeeze(&rows[29]).starts_with("○plan·只读"),
            "{:?}",
            rows[29]
        );
        // 宽字符后面的补位格会插进空格，比较前先压掉空白
        assert!(squeeze(&rows[29]).contains("Enter发送"), "{:?}", rows[29]);
        assert_eq!(
            term.backend().buffer()[(1, 29)].bg,
            Look::default().chip(Mode::Plan)
        );

        fx.mode = Mode::Auto;
        let rows = screen(&fx.render(80, 30));
        assert!(rows[29].starts_with(" ◆ auto "), "{:?}", rows[29]);
    }

    /// anime 字形：圆角框 + 星星标记，一眼能和默认版分开
    #[test]
    fn the_anime_look_rounds_the_borders_and_swaps_the_markers() {
        let mut fx = Fixture::new();
        fx.look = Look::new(Theme::Dark, FontStyle::Anime);
        fx.transcript.push(Entry::Info("hello".into()));
        let rows = screen(&fx.render(80, 30));
        assert!(rows[0].starts_with('╭'), "{:?}", rows[0]);
        assert!(rows[28].starts_with('╰'), "{:?}", rows[28]);
        assert!(
            rows.iter().any(|r| r.contains("✦ hello")),
            "信息条目前缀应当换成星星"
        );

        // 默认版还是方角 + 圆点
        let mut fx = Fixture::new();
        fx.transcript.push(Entry::Info("hello".into()));
        let rows = screen(&fx.render(80, 30));
        assert!(rows[0].starts_with('┌'), "{:?}", rows[0]);
        assert!(rows.iter().any(|r| r.contains("● hello")));
    }

    #[test]
    fn the_light_theme_paints_a_different_mode_chip() {
        let mut fx = Fixture::new();
        fx.look = Look::new(Theme::Light, FontStyle::Default);
        let light = fx.render(80, 30);
        let light_bg = light.backend().buffer()[(1, 29)].bg;
        let mut fx = Fixture::new();
        let dark = fx.render(80, 30);
        let dark_bg = dark.backend().buffer()[(1, 29)].bg;
        assert_ne!(light_bg, dark_bg, "两套主题的模式标签不该同色");
        assert_eq!(
            light_bg,
            Look::new(Theme::Light, FontStyle::Default).chip(Mode::Plan)
        );
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
        fx.scroll.jump_to_top();
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
            items: vec![
                ("/help".into(), "help".into()),
                ("/exit".into(), "quit".into()),
            ],
            selected: 1,
        });
        let rows = screen(&fx.render(80, 30));
        assert!(rows.iter().any(|r| r.contains("commands")));
        assert!(
            rows.iter()
                .any(|r| r.contains("  /help") && r.contains("help"))
        );
        assert!(rows.iter().any(|r| r.contains("› /exit")));
        // 输入框仍然在最底部
        assert!(rows[27].contains('›'));
    }

    #[test]
    fn long_choice_list_scrolls_to_keep_selection_visible() {
        let mut fx = Fixture::new();
        fx.choice = Some(Choice {
            title: "t".into(),
            items: (0..30)
                .map(|i| (format!("item{i}"), String::new()))
                .collect(),
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
        assert!(
            rows[27].starts_with("│ ›"),
            "prompt should not touch the border"
        );
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
        assert!(
            rows.iter()
                .any(|r| r.contains("VG-001") && r.contains("(1/3)"))
        );
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
        assert_eq!(
            (pos.x, pos.y),
            (0, 0),
            "no cursor is placed while the input is inactive"
        );
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
        // 启动画面在各种尺寸下也不能画崩
        fx.show_banner = true;
        for w in [1, 20, 80, 120] {
            for h in [1, 12, 24, 30, 40, 60] {
                fx.render(w, h);
            }
        }
    }

    /// 四分之一方块字形：字符画版本里只会有这些字，用来判断某一行是不是图
    fn art_cells(row: &str) -> usize {
        row.chars()
            .filter(|c| {
                matches!(
                    c,
                    '▘' | '▝'
                        | '▀'
                        | '▖'
                        | '▌'
                        | '▞'
                        | '▛'
                        | '▗'
                        | '▚'
                        | '▐'
                        | '▜'
                        | '▄'
                        | '▙'
                        | '▟'
                        | '█'
                )
            })
            .count()
    }

    /// 终端能画字符但画不了位图、又不够高时：一枚小徽标 + 字标，整块居中
    #[test]
    fn wordmark_is_centred_when_the_art_does_not_fit() {
        let mut fx = Fixture::new();
        fx.show_banner = true;
        let rows = screen(&fx.render(80, 32));
        let body = areas(Rect::new(0, 0, 80, 32), 1, 0, false).body;
        let body_rows = &rows[body.y as usize..body.bottom() as usize];

        // 徽标就是标题栏那一枚，不是整幅插画
        let art: Vec<usize> = body_rows
            .iter()
            .enumerate()
            .filter(|(_, r)| art_cells(r) > 0)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            art.len(),
            icon::AVATAR_ROWS as usize,
            "徽标只占 {} 行",
            icon::AVATAR_ROWS
        );

        let squeeze = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
        let name = body_rows
            .iter()
            .position(|r| squeeze(r) == "vibeguard")
            .expect("字标应当出现在对话区");
        assert!(squeeze(&body_rows[name + 1]).contains("AI生成代码"));

        // 整块水平居中
        let want = (80 - "v i b e g u a r d".width()) / 2;
        let x = rows[body.y as usize + name]
            .chars()
            .position(|c| c == 'v')
            .unwrap();
        assert!(
            x.abs_diff(want) <= 1,
            "字标应当居中（期望第 {want} 列），实际在第 {x} 列"
        );

        // 整块垂直居中：上下留白相当
        let top = art[0];
        let bottom = name + 1;
        assert!(
            top.abs_diff(body_rows.len() - 1 - bottom) <= 1,
            "徽标 + 字标应当垂直居中：上 {top} 下 {}",
            body_rows.len() - 1 - bottom
        );
    }

    /// 终端画不了位图、但高度够：用四分之一方块把整幅插画拼出来，居中
    #[test]
    fn cell_art_is_drawn_when_the_terminal_has_no_image_protocol() {
        let mut fx = Fixture::new();
        fx.show_banner = true;
        let rows = screen(&fx.render(80, 40));
        let body = areas(Rect::new(0, 0, 80, 40), 1, 0, false).body;
        let (cols, art_rows) = icon::logo_size(40, false).unwrap();

        let painted: Vec<usize> = (body.y as usize..body.bottom() as usize)
            .filter(|y| art_cells(&rows[*y]) > 0)
            .collect();
        assert_eq!(
            painted.len(),
            art_rows as usize,
            "字符画应当占 {art_rows} 行"
        );
        assert!(
            painted.windows(2).all(|w| w[1] == w[0] + 1),
            "字符画中间不能断行"
        );

        // 整块水平居中，左右留白相等
        let row = &rows[painted[0]];
        let left = row.chars().take_while(|c| *c == ' ').count();
        let right = row.chars().rev().take_while(|c| *c == ' ').count();
        assert_eq!(art_cells(row), cols as usize, "字符画应当铺满 {cols} 列");
        assert!(
            left.abs_diff(right) <= 1,
            "字符画应当水平居中：左 {left} 右 {right}"
        );

        // 图下面跟一行说明，不再重复品牌名
        let squeeze = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
        let cap = painted[painted.len() - 1] + 1;
        assert!(
            squeeze(&rows[cap]).contains("AI生成代码"),
            "{:?}",
            rows[cap]
        );
        assert!(
            !rows.iter().any(|r| squeeze(r) == "vibeguard"),
            "字符画里已经有手写的品牌名了"
        );
    }

    /// 有对话之后对话排在徽标和字标下面
    #[test]
    fn transcript_sits_below_the_wordmark() {
        let mut fx = Fixture::new();
        fx.show_banner = true;
        fx.transcript.push(Entry::Info("hello".into()));
        let rows = screen(&fx.render(80, 40));
        let body = areas(Rect::new(0, 0, 80, 40), 1, 0, false).body;
        let at = rows.iter().position(|r| r.contains("hello")).unwrap() as u16;
        let art = (body.y..body.bottom())
            .rfind(|y| art_cells(&rows[*y as usize]) > 0)
            .unwrap();
        assert!(
            at >= art + 2,
            "对话应当排在徽标和字标下面：图到第 {art} 行，对话在第 {at} 行"
        );
    }

    #[test]
    fn hidden_banner_gives_the_whole_body_to_the_transcript() {
        let mut fx = Fixture::new();
        fx.transcript.push(Entry::Info("hello".into()));
        let rows = screen(&fx.render(80, 40));
        let body = areas(Rect::new(0, 0, 80, 40), 1, 0, false).body;
        assert!(rows[body.y as usize].contains("hello"));
    }

    /// 矮终端不放启动画面：放不下就会把欢迎语挤没
    #[test]
    fn short_terminal_skips_the_splash_but_keeps_the_transcript() {
        let mut fx = Fixture::new();
        fx.show_banner = true;
        fx.transcript.push(Entry::Info("hello".into()));
        let rows = screen(&fx.render(80, 24));
        let body = areas(Rect::new(0, 0, 80, 24), 1, 0, false).body;
        assert!(
            rows[body.y as usize].contains("hello"),
            "欢迎语不该被启动画面挤掉"
        );
        assert!(
            (body.y..body.bottom()).all(|y| art_cells(&rows[y as usize]) == 0),
            "24 行不该画启动画面"
        );
    }

    /// 横条纹大图配上「半块字符」协议，用来在无头环境里跑一遍原生图片的绘制分支。
    /// 图要比目标区域大，`Resize::Fit` 才会把它缩到整块区域；
    /// 上下半格颜色不同，半块字符才会真的写出 `▀`（同色会被编码成空格）。
    fn striped_protocol() -> StatefulProtocol {
        let mut img = image::RgbaImage::new(600, 600);
        for (_x, y, p) in img.enumerate_pixels_mut() {
            *p = if y % 2 == 0 {
                image::Rgba([247, 168, 200, 255])
            } else {
                image::Rgba([40, 20, 60, 255])
            };
        }
        ratatui_image::picker::Picker::halfblocks()
            .new_resize_protocol(image::DynamicImage::ImageRgba8(img))
    }

    #[test]
    fn a_native_protocol_is_drawn_instead_of_the_wordmark() {
        let mut fx = Fixture::new();
        fx.show_banner = true;
        fx.avatar_proto = Some(striped_protocol());
        fx.banner_proto = Some(striped_protocol());
        let rows = screen(&fx.render(80, 40));
        let body = areas(Rect::new(0, 0, 80, 40), 1, 0, false).body;
        let body_rows = &rows[body.y as usize..body.bottom() as usize];

        // 图片协议用半块字符铺满整块区域（32 列宽），字标只有两行文字
        let blocks = body_rows
            .iter()
            .map(|r| r.chars().filter(|c| *c == '▀' || *c == '▄').count())
            .max()
            .unwrap_or(0);
        assert!(
            blocks > 10,
            "启动画面应当是整行图片，实际最多 {blocks} 个半块字符"
        );

        // 图下面只跟一句说明（图上已经有手写的品牌名）
        let squeeze = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
        assert!(body_rows.iter().any(|r| squeeze(r).contains("AI生成代码")));
        assert!(
            body_rows.iter().all(|r| squeeze(r) != "vibeguard"),
            "有原图时不该再写一遍品牌名"
        );
    }

    fn session_panel() -> SessionsPanel {
        use crate::sessions::{CatalogEntry, Summary};
        let item = |id: &str, title: &str, body: &str, turns: usize| {
            SessionItem::from_catalog(
                CatalogEntry {
                    summary: Summary {
                        id: id.into(),
                        title: title.into(),
                        updated: 0,
                        turns,
                        model: "deepseek-chat".into(),
                    },
                    body: body.into(),
                },
                "2 小时前".into(),
            )
        };
        SessionsPanel::new(vec![
            item("a", "如何防止 SQL 注入", "用参数化查询", 3),
            item("b", "聊聊天气", "今天不错", 1),
        ])
        .unwrap()
    }

    #[test]
    fn the_sessions_panel_floats_over_the_transcript_with_a_search_box() {
        let squeeze = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
        let mut fx = Fixture::new();
        fx.transcript.push(Entry::User("底下的对话".into()));
        fx.sessions = Some(session_panel());
        let rows = screen(&fx.render(100, 34));
        let all: Vec<String> = rows.iter().map(|r| squeeze(r)).collect();

        assert!(all.iter().any(|r| r.contains("历史对话2")), "{all:?}");
        assert!(all.iter().any(|r| r.contains("搜索输入关键词过滤")));
        assert!(all.iter().any(|r| r.contains("如何防止SQL注入")), "{all:?}");
        assert!(all.iter().any(|r| r.contains("聊聊天气")));
        assert!(
            all.iter().any(|r| r.contains("Enter打开")),
            "页脚要有快捷键"
        );

        // 浮层只占对话区：图标栏和输入框还留在原位
        let body = areas(Rect::new(0, 0, 100, 34), 1, 0, false).body;
        let title_at = rows
            .iter()
            .position(|r| squeeze(r).contains("历史对话2"))
            .expect("panel title row");
        assert!(
            (body.y as usize..body.bottom() as usize).contains(&title_at),
            "浮层应当落在对话区里：y={title_at} body={body:?}"
        );
        assert!(all.iter().any(|r| r.contains("VibeGuardv0")), "图标栏还在");
        assert!(all.iter().any(|r| r.contains("Enter发送")), "状态栏还在");

        // 居中的：左上角既不在 0 列，也不贴着左半边
        let x = rows[title_at].find('┌').expect("panel border");
        assert!((1..20).contains(&x), "浮层应当居中：{:?}", rows[title_at]);
    }

    #[test]
    fn typing_in_the_panel_filters_the_list_and_shows_the_query() {
        let squeeze = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
        let mut fx = Fixture::new();
        let mut panel = session_panel();
        for c in "天气".chars() {
            panel.on_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        fx.sessions = Some(panel);
        let all: Vec<String> = screen(&fx.render(100, 34))
            .iter()
            .map(|r| squeeze(r))
            .collect();

        assert!(all.iter().any(|r| r.contains("历史对话1/2")), "{all:?}");
        assert!(all.iter().any(|r| r.contains("搜索天气")));
        assert!(all.iter().any(|r| r.contains("聊聊天气")));
        assert!(
            !all.iter().any(|r| r.contains("如何防止SQL注入")),
            "被过滤掉了"
        );
    }
}
