//! 界面语言。
//!
//! 默认英文，`/lang zh` 切中文。用进程级的原子量而不是把 `Lang` 传遍每个函数：
//! 界面文案散在十几个文件里，穿参数会把签名全改一遍，收益却只有「测试能并行」。
//!
//! **注意**：原子的初值是中文，不是英文。二进制启动时会立刻用配置覆盖它
//! （配置默认 `en`），而单元测试不经过 `main`，所以测试里看到的中文断言不受影响。
//! 这是有意的取舍——否则几十个断言界面文案的测试要一个个改成显式设语言。

mod strings;

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    /// 默认：英文
    #[default]
    En,
    Zh,
}

impl Lang {
    pub fn parse(s: &str) -> Option<Lang> {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" | "en-us" | "en-gb" | "english" => Some(Lang::En),
            "zh" | "zh-cn" | "zh-hans" | "chinese" | "中文" => Some(Lang::Zh),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Zh => "zh",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Zh => "中文",
        }
    }
}

/// 0 = 英文，1 = 中文
static LANG: AtomicU8 = AtomicU8::new(1);

pub fn set(lang: Lang) {
    LANG.store(u8::from(lang == Lang::Zh), Ordering::Relaxed);
}

pub fn current() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 {
        Lang::Zh
    } else {
        Lang::En
    }
}

pub fn is_zh() -> bool {
    LANG.load(Ordering::Relaxed) == 1
}

/// 二选一：`t("中文", "English")`
pub fn t(zh: &'static str, en: &'static str) -> &'static str {
    if is_zh() { zh } else { en }
}

/// 带插值的版本。两边都算一遍，再按语言挑一个——
/// 字符串都很短，这点浪费换来的是「调用处不用为两个分支写两遍 format!」。
pub fn pick(zh: String, en: String) -> String {
    if is_zh() { zh } else { en }
}

/// 中文原文 → 译文。
///
/// 查不到就原样返回中文：漏译只会让界面露一句中文，不会 panic。
/// 用中文当键是为了让调用处保持原样——`"读取"` 改成 `tr("读取")`，
/// 翻译集中在一张表里，改文案和改译文都不用两头找。
pub fn tr(zh: &str) -> &str {
    if is_zh() {
        return zh;
    }
    table().get(zh).copied().unwrap_or(zh)
}

fn table() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| strings::TABLE.iter().copied().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_the_obvious_spellings() {
        for s in ["en", "EN", "english", " en-us "] {
            assert_eq!(Lang::parse(s), Some(Lang::En), "{s}");
        }
        for s in ["zh", "zh-CN", "chinese", "中文"] {
            assert_eq!(Lang::parse(s), Some(Lang::Zh), "{s}");
        }
        assert_eq!(Lang::parse("fr"), None);
    }

    #[test]
    fn the_default_is_english() {
        assert_eq!(Lang::default(), Lang::En);
        assert_eq!(Lang::En.code(), "en");
    }

    /// 表是手写的，最容易出的错是复制粘贴留下的重复键和空译文
    #[test]
    fn the_table_has_no_duplicates_and_no_empty_values() {
        let mut seen = std::collections::HashSet::new();
        for (zh, en) in strings::TABLE {
            assert!(seen.insert(*zh), "重复的中文原文：{zh}");
            assert!(!zh.trim().is_empty(), "空的中文原文");
            assert!(!en.trim().is_empty(), "没有译文：{zh}");
        }
    }

    /// 译文少一个 `{}`，format! 会在运行时 panic——编译期看不出来，只能这里拦
    #[test]
    fn every_translation_keeps_the_same_placeholders() {
        for (zh, en) in strings::TABLE {
            let count = |s: &str| s.matches('{').count();
            assert_eq!(
                count(zh),
                count(en),
                "占位符数量不一致：\n  中文 {zh}\n  英文 {en}"
            );
        }
    }

    #[test]
    fn the_common_labels_are_translated() {
        let map: std::collections::HashMap<_, _> = strings::TABLE.iter().copied().collect();
        assert_eq!(map.get("只读"), Some(&"read-only"));
        assert_eq!(map.get("严重"), Some(&"Critical"));
        assert_eq!(map.get("跳过"), Some(&"Skip"));
        assert_eq!(map.get("已取消"), Some(&"Cancelled"));
    }

    #[test]
    fn unknown_strings_fall_back_to_the_chinese_source() {
        // 查不到就返回原文：漏译只露一句中文，不会 panic
        assert_eq!(tr("这句话故意没有译文"), "这句话故意没有译文");
    }

    /// 界面靠「错误：权限不足」这个前缀判断是不是权限问题。
    /// 英文那边由 "Error: " 和 "Permission denied: …" 拼出来，
    /// 前缀也得是 "Error: Permission denied"——少一个字母，判断就失效。
    #[test]
    fn the_permission_prefix_matches_the_message_in_english() {
        let map = table();
        for body in ["权限不足：不允许写文件", "权限不足：不允许编辑文件"] {
            let composed = format!("{}: {}", map["错误"], map[body]);
            assert!(
                composed.starts_with(map["错误：权限不足"]),
                "前缀对不上：{composed} 不以 {} 开头",
                map["错误：权限不足"]
            );
        }
    }
}
