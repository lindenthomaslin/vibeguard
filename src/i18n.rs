//! 界面语言。
//!
//! 默认英文，`/lang zh` 切中文。用进程级的原子量而不是把 `Lang` 传遍每个函数：
//! 界面文案散在十几个文件里，穿参数会把签名全改一遍，收益却只有「测试能并行」。
//!
//! **注意**：原子的初值是中文，不是英文。二进制启动时会立刻用配置覆盖它
//! （配置默认 `en`），而单元测试不经过 `main`，所以测试里看到的中文断言不受影响。
//! 这是有意的取舍——否则几十个断言界面文案的测试要一个个改成显式设语言。

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
}
