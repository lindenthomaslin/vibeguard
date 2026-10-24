//! 配色与字形：`dark`/`light` 两套底色，`default`/`anime` 两种字形。
//!
//! 两个维度是独立的：主题决定明暗和饱和度，字形决定框线、标记符号和 spinner。
//! 终端里换不了字体，能换的是「用什么符号画」——圆角框配星星和爱心，看起来就柔和得多。
//!
//! 颜色全部用 `Rgb` 而不是 ANSI 名字：`Yellow` 在浅色底上几乎看不见，
//! 而浅色主题需要的是琥珀色，ANSI 八色里根本没有。

use crate::mode::Mode;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::BorderType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontStyle {
    #[default]
    Default,
    Anime,
}

impl Theme {
    pub const ALL: [Theme; 2] = [Theme::Dark, Theme::Light];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "dark" | "深色" => Some(Theme::Dark),
            "light" | "浅色" => Some(Theme::Light),
            _ => None,
        }
    }
}

impl FontStyle {
    pub const ALL: [FontStyle; 2] = [FontStyle::Default, FontStyle::Anime];

    pub fn label(self) -> &'static str {
        match self {
            FontStyle::Default => "default",
            FontStyle::Anime => "anime",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "default" | "plain" | "默认" => Some(FontStyle::Default),
            "anime" | "cute" | "动漫" => Some(FontStyle::Anime),
            _ => None,
        }
    }
}

/// 强调色：边框、品牌字、输入前缀、状态栏标签都用它。
///
/// 主题定明暗，字形定框线，强调色定「这套界面是什么颜色」——三个维度独立，
/// 所以 `dark + anime + ocean` 这种组合也能用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Accent {
    #[default]
    Teal,
    Sakura,
    Ocean,
    Violet,
    Amber,
    Lime,
    Mono,
}

impl Accent {
    pub const ALL: [Accent; 7] = [
        Accent::Teal,
        Accent::Sakura,
        Accent::Ocean,
        Accent::Violet,
        Accent::Amber,
        Accent::Lime,
        Accent::Mono,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Accent::Teal => "teal",
            Accent::Sakura => "sakura",
            Accent::Ocean => "ocean",
            Accent::Violet => "violet",
            Accent::Amber => "amber",
            Accent::Lime => "lime",
            Accent::Mono => "mono",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" => None,
            "teal" | "default" | "青" => Some(Accent::Teal),
            "sakura" | "pink" | "樱" => Some(Accent::Sakura),
            "ocean" | "blue" | "蓝" => Some(Accent::Ocean),
            "violet" | "purple" | "紫" => Some(Accent::Violet),
            "amber" | "gold" | "琥珀" => Some(Accent::Amber),
            "lime" | "green" | "绿" => Some(Accent::Lime),
            "mono" | "grey" | "gray" | "灰" => Some(Accent::Mono),
            _ => None,
        }
    }

    /// 深色底和浅色底各一份。同一个名字在浅色底上必须压暗压饱和，
    /// 否则边框细、背景亮，整块糊在一起。
    pub fn color(self, theme: Theme) -> Color {
        match (self, theme) {
            (Accent::Teal, Theme::Dark) => rgb(0x6F_D3_E0),
            (Accent::Teal, Theme::Light) => rgb(0x0F_6E_7C),
            (Accent::Sakura, Theme::Dark) => rgb(0xF7_A8_C8),
            (Accent::Sakura, Theme::Light) => rgb(0xB0_3A_6B),
            (Accent::Ocean, Theme::Dark) => rgb(0x5A_A9_F0),
            (Accent::Ocean, Theme::Light) => rgb(0x1B_5F_A8),
            (Accent::Violet, Theme::Dark) => rgb(0xB6_9C_F0),
            (Accent::Violet, Theme::Light) => rgb(0x6A_4B_B5),
            (Accent::Amber, Theme::Dark) => rgb(0xE8_B4_57),
            (Accent::Amber, Theme::Light) => rgb(0x8A_5A_00),
            (Accent::Lime, Theme::Dark) => rgb(0x9B_D4_6A),
            (Accent::Lime, Theme::Light) => rgb(0x4A_7A_1E),
            (Accent::Mono, Theme::Dark) => rgb(0xC8_C8_C8),
            (Accent::Mono, Theme::Light) => rgb(0x3A_3A_3A),
        }
    }
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// 语义色板。名字按用途取，不按颜色取——浅色主题里 `warn` 不是黄色而是琥珀色。
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// 边框、品牌、用户输入的前缀
    pub accent: Color,
    pub success: Color,
    pub warn: Color,
    pub error: Color,
    /// 次要强调：低危级别、信息标记
    pub info: Color,
}

const DARK: Palette = Palette {
    accent: rgb(0x6F_D3_E0),
    success: rgb(0x6F_D0_8C),
    warn: rgb(0xE0_B3_41),
    error: rgb(0xF2_77_7A),
    info: rgb(0x7A_A2_F7),
};

const LIGHT: Palette = Palette {
    accent: rgb(0x0F_6E_7C),
    success: rgb(0x1B_6E_45),
    warn: rgb(0x8A_5A_00),
    error: rgb(0xA8_2A_34),
    info: rgb(0x27_53_96),
};

/// 动漫字形的主色是樱花粉；浅色底上换成深玫瑰，否则看不清
const SAKURA_DARK: Color = rgb(0xF7_A8_C8);
const SAKURA_LIGHT: Color = rgb(0xB0_3A_6B);

/// 状态栏模式标签的底色（上面压黑字）
const CHIP_DARK: [Color; 3] = [rgb(0x6F_D3_E0), rgb(0xE0_B3_41), rgb(0x6F_D0_8C)];
const CHIP_LIGHT: [Color; 3] = [rgb(0x8F_D3_E0), rgb(0xF2_C9_6B), rgb(0x9A_DD_B4)];
const CHIP_ANIME: [Color; 3] = [rgb(0xA8_D8_EA), rgb(0xF7_D6_A8), rgb(0xB5_E8_C8)];

const PLAIN_SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPARKLE_SPINNER: [&str; 4] = ["✦", "✧", "✩", "⋆"];

/// 字形：框线形状、每种条目的前缀符号、spinner 的帧
#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    pub border: BorderType,
    /// 用户输入
    pub user: &'static str,
    pub info: &'static str,
    pub success: &'static str,
    pub warn: &'static str,
    pub error: &'static str,
    /// 思考过程的小标题
    pub thinking: &'static str,
    pub spinner: &'static [&'static str],
}

const GLYPHS_DEFAULT: Glyphs = Glyphs {
    border: BorderType::Plain,
    user: "›",
    info: "●",
    success: "✓",
    warn: "!",
    error: "✗",
    thinking: "✻",
    spinner: &PLAIN_SPINNER,
};

/// 圆角框 + 爱心/星星。勾和叉保留原样：这两个符号要的是「一眼看懂」，不是可爱
const GLYPHS_ANIME: Glyphs = Glyphs {
    border: BorderType::Rounded,
    user: "♡",
    info: "✦",
    success: "✓",
    warn: "⚠",
    error: "✗",
    thinking: "✧",
    spinner: &SPARKLE_SPINNER,
};

/// 一帧要用的全部配色和字形。`Copy`，每帧传一份没有成本。
#[derive(Debug, Clone, Copy)]
pub struct Look {
    pub theme: Theme,
    pub font: FontStyle,
    pub palette: Palette,
    pub glyphs: Glyphs,
}

impl Default for Look {
    fn default() -> Self {
        Self::new(Theme::default(), FontStyle::default())
    }
}

impl Look {
    pub fn new(theme: Theme, font: FontStyle) -> Self {
        Self::with_accent(theme, font, None)
    }

    /// 显式选了强调色就听它的；没选时动漫字形默认樱花粉，其余用主题自带的青色
    pub fn with_accent(theme: Theme, font: FontStyle, accent: Option<Accent>) -> Self {
        let mut palette = match theme {
            Theme::Dark => DARK,
            Theme::Light => LIGHT,
        };
        if let Some(accent) = accent {
            palette.accent = accent.color(theme);
        } else if font == FontStyle::Anime {
            palette.accent = match theme {
                Theme::Dark => SAKURA_DARK,
                Theme::Light => SAKURA_LIGHT,
            };
        }
        Self {
            theme,
            font,
            palette,
            glyphs: match font {
                FontStyle::Default => GLYPHS_DEFAULT,
                FontStyle::Anime => GLYPHS_ANIME,
            },
        }
    }

    /// 从配置读。配置项已经校验过，认不出来就退回默认值。
    pub fn from_config(cfg: &crate::config::Config) -> Self {
        Self::with_accent(
            Theme::parse(&cfg.theme).unwrap_or_default(),
            FontStyle::parse(&cfg.font_style).unwrap_or_default(),
            Accent::parse(&cfg.accent),
        )
    }

    // ---- 常用样式 ----

    pub fn accent(&self) -> Style {
        Style::new().fg(self.palette.accent)
    }

    pub fn accent_bold(&self) -> Style {
        self.accent().add_modifier(Modifier::BOLD)
    }

    /// 次要文字。用 `DIM` 而不是固定灰色：终端会把它朝背景方向压，两套底色都对
    pub fn muted(&self) -> Style {
        Style::new().add_modifier(Modifier::DIM)
    }

    pub fn success(&self) -> Style {
        Style::new().fg(self.palette.success)
    }

    pub fn warn(&self) -> Style {
        Style::new().fg(self.palette.warn)
    }

    pub fn error(&self) -> Style {
        Style::new().fg(self.palette.error)
    }

    pub fn info(&self) -> Style {
        Style::new().fg(self.palette.info)
    }

    /// 状态栏模式标签的底色
    pub fn chip(&self, mode: Mode) -> Color {
        let table = match (self.font, self.theme) {
            (FontStyle::Anime, _) => CHIP_ANIME,
            (_, Theme::Dark) => CHIP_DARK,
            (_, Theme::Light) => CHIP_LIGHT,
        };
        table[mode as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luma(c: Color) -> u32 {
        match c {
            Color::Rgb(r, g, b) => 299 * r as u32 + 587 * g as u32 + 114 * b as u32,
            other => panic!("expected Rgb, got {other:?}"),
        }
    }

    #[test]
    fn light_and_dark_are_actually_opposite() {
        let dark = Look::new(Theme::Dark, FontStyle::Default);
        let light = Look::new(Theme::Light, FontStyle::Default);
        for (d, l) in [
            (dark.palette.accent, light.palette.accent),
            (dark.palette.success, light.palette.success),
            (dark.palette.warn, light.palette.warn),
            (dark.palette.error, light.palette.error),
            (dark.palette.info, light.palette.info),
        ] {
            assert!(
                luma(d) > luma(l),
                "浅色主题应当比深色主题暗：{d:?} vs {l:?}"
            );
        }
    }

    #[test]
    fn every_colour_stays_readable_against_its_background() {
        // 深色底要亮色（luma > 0.45 白），浅色底要暗色（luma < 0.55 白）
        let max = 255 * 1000;
        for (theme, want_light) in [(Theme::Dark, true), (Theme::Light, false)] {
            let look = Look::new(theme, FontStyle::Default);
            for c in [
                look.palette.accent,
                look.palette.success,
                look.palette.warn,
                look.palette.error,
                look.palette.info,
            ] {
                let ratio = luma(c) as f32 / max as f32;
                if want_light {
                    assert!(ratio > 0.42, "{theme:?} 的 {c:?} 太暗，压在深色底上会糊");
                } else {
                    assert!(ratio < 0.55, "{theme:?} 的 {c:?} 太亮，压在浅色底上看不清");
                }
            }
        }
    }

    #[test]
    fn the_two_font_styles_differ_visibly() {
        let d = Look::new(Theme::Dark, FontStyle::Default);
        let a = Look::new(Theme::Dark, FontStyle::Anime);
        assert_eq!(d.glyphs.border, BorderType::Plain);
        assert_eq!(a.glyphs.border, BorderType::Rounded);
        assert_ne!(d.glyphs.user, a.glyphs.user);
        assert_ne!(d.glyphs.spinner, a.glyphs.spinner);
        // 勾和叉两个都要，语义优先
        assert_eq!(d.glyphs.success, a.glyphs.success);
        assert_eq!(d.glyphs.error, a.glyphs.error);
    }

    #[test]
    fn anime_recolours_the_accent_but_keeps_it_legible() {
        for theme in Theme::ALL {
            let plain = Look::new(theme, FontStyle::Default);
            let anime = Look::new(theme, FontStyle::Anime);
            assert_ne!(plain.palette.accent, anime.palette.accent);
            assert_eq!(plain.palette.error, anime.palette.error, "只换主色");
        }
    }

    #[test]
    fn the_mode_chip_is_always_a_background_with_black_text() {
        for theme in Theme::ALL {
            for font in FontStyle::ALL {
                let look = Look::new(theme, font);
                for mode in Mode::ALL {
                    let bg = look.chip(mode);
                    // 压黑字的底色必须够亮，否则黑字看不见
                    assert!(
                        luma(bg) as f32 / (255.0 * 1000.0) > 0.55,
                        "{theme:?}/{font:?} 的 {mode:?} 底色太暗"
                    );
                }
            }
        }
    }

    #[test]
    fn every_mode_gets_its_own_chip_colour() {
        let look = Look::default();
        let colours: Vec<Color> = Mode::ALL.iter().map(|m| look.chip(*m)).collect();
        for (i, a) in colours.iter().enumerate() {
            for b in &colours[i + 1..] {
                assert_ne!(a, b, "{colours:?} 有重复");
            }
        }
    }

    #[test]
    fn parse_accepts_english_and_chinese_aliases() {
        assert_eq!(Theme::parse("DARK"), Some(Theme::Dark));
        assert_eq!(Theme::parse("浅色"), Some(Theme::Light));
        assert_eq!(Theme::parse("nope"), None);
        assert_eq!(FontStyle::parse("Anime"), Some(FontStyle::Anime));
        assert_eq!(FontStyle::parse("动漫"), Some(FontStyle::Anime));
        assert_eq!(FontStyle::parse("nope"), None);
    }

    #[test]
    fn cycling_wraps() {
        assert_eq!(Theme::Dark.next(), Theme::Light);
        assert_eq!(Theme::Light.next(), Theme::Dark);
    }

    #[test]
    fn defaults_are_the_current_look() {
        let look = Look::default();
        assert_eq!(look.theme, Theme::Dark);
        assert_eq!(look.font, FontStyle::Default);
        assert_eq!(look.glyphs.user, "›");
    }
    #[test]
    fn an_explicit_accent_beats_the_font_default() {
        let anime = Look::new(Theme::Dark, FontStyle::Anime);
        let ocean = Look::with_accent(Theme::Dark, FontStyle::Anime, Some(Accent::Ocean));
        assert_ne!(anime.palette.accent, ocean.palette.accent);
        assert_eq!(ocean.palette.accent, Accent::Ocean.color(Theme::Dark));
    }

    #[test]
    fn every_accent_has_a_distinct_colour_on_both_backgrounds() {
        for theme in Theme::ALL {
            let mut seen = std::collections::BTreeSet::new();
            for accent in Accent::ALL {
                assert!(
                    seen.insert(format!("{:?}", accent.color(theme))),
                    "{} 在 {theme:?} 上和别的强调色撞了",
                    accent.label()
                );
            }
        }
    }

    #[test]
    fn accent_names_round_trip() {
        for accent in Accent::ALL {
            assert_eq!(Accent::parse(accent.label()), Some(accent));
        }
        assert_eq!(Accent::parse(""), None);
    }

    #[test]
    fn the_config_accent_reaches_the_palette() {
        let mut cfg = crate::config::Config {
            accent: "ocean".to_string(),
            ..Default::default()
        };
        let look = Look::from_config(&cfg);
        assert_eq!(look.palette.accent, Accent::Ocean.color(look.theme));
        // 空值 = 跟随主题，不改强调色
        cfg.accent.clear();
        let plain = Look::from_config(&cfg);
        assert_eq!(
            plain.palette.accent,
            Look::new(plain.theme, plain.font).palette.accent
        );
    }
}
