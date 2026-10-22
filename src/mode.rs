//! 三种工作模式：先看不动、改一处问一次、一路改到底。
//!
//! 模式不只是界面上的一个标签，它同时决定三件事：
//! 系统提示词里的行为约定、工具调用的走法、以及写操作要不要停下来问。
//! 全屏界面在左下角显示当前模式，用 `Tab+P` 循环切换；非交互模式用 `/mode`。

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// 只读：扫描并产出一份漏洞报告，不改任何文件
    #[default]
    Plan,
    /// 每改一处都先停下来问一次
    Confirm,
    /// 不打断，一路做完
    Auto,
}

impl Mode {
    /// 循环顺序，与 `next` 一致
    pub const ALL: [Mode; 3] = [Mode::Plan, Mode::Confirm, Mode::Auto];

    /// 左下角的短标签：英文单词比中文更短，不容易把状态栏挤掉
    pub fn label(self) -> &'static str {
        match self {
            Mode::Plan => "plan",
            Mode::Confirm => "confirm",
            Mode::Auto => "auto",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Mode::Plan => "计划",
            Mode::Confirm => "逐条确认",
            Mode::Auto => "全自动",
        }
    }

    /// 标签前的符号：不看颜色也能分辨是哪一档。
    /// 只用几何图形区里最常见的那几个，冷门字体里也不会变成豆腐块。
    pub fn glyph(self) -> &'static str {
        match self {
            Mode::Plan => "○",
            Mode::Confirm => "◇",
            Mode::Auto => "◆",
        }
    }

    /// 一句话说明，切换模式时写进对话区
    pub fn describe(self) -> &'static str {
        match self {
            Mode::Plan => "只读分析，产出漏洞报告，不改文件",
            Mode::Confirm => "每改一处都先确认",
            Mode::Auto => "不打断，一路做完",
        }
    }

    /// 追加到系统提示词末尾的行为约定
    pub fn instruction(self) -> &'static str {
        match self {
            Mode::Plan => {
                "\n\n【当前模式：计划】只读分析，不要调用 edit_file / write_file 修改任何文件。\
先用 read_file / list_dir 通读相关代码，确认问题真实存在再下结论，不要凭文件名猜。\
最后输出一份 Markdown 漏洞报告：每条包含严重级别、位置（文件:行）、问题说明、风险、\
修复建议（给出改完之后的代码片段），末尾再给一个按严重程度排序的修复顺序。"
            }
            Mode::Confirm => {
                "\n\n【当前模式：逐条确认】每修改一处都会先征求用户同意。\
一次只处理一个文件的一处改动，改完立刻停下等确认，不要在同一条回复里连续提交多个修改。\
用户跳过就记下这一点继续下一个问题，不要换个说法重复提交同一个改动。"
            }
            Mode::Auto => {
                "\n\n【当前模式：全自动】直接完成修改，不需要征求同意。\
按严重程度从高到低依次处理，每处改动尽量小、可验证；全部改完用几句话说明改了什么。"
            }
        }
    }

    /// 下一档，循环
    pub fn next(self) -> Self {
        match self {
            Mode::Plan => Mode::Confirm,
            Mode::Confirm => Mode::Auto,
            Mode::Auto => Mode::Plan,
        }
    }

    /// 解析命令行 / `/mode` 的参数，大小写和中英文都认
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "plan" | "p" | "计划" => Some(Mode::Plan),
            "confirm" | "c" | "确认" | "逐条确认" => Some(Mode::Confirm),
            "auto" | "a" | "自动" | "全自动" => Some(Mode::Auto),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_visits_every_mode_and_wraps() {
        let mut seen = vec![Mode::default()];
        for _ in 0..Mode::ALL.len() - 1 {
            seen.push(seen.last().unwrap().next());
        }
        assert_eq!(seen, Mode::ALL);
        assert_eq!(Mode::Auto.next(), Mode::Plan);
    }

    #[test]
    fn every_mode_looks_different() {
        fn distinct<T: PartialEq + std::fmt::Debug>(what: &str, f: impl Fn(Mode) -> T) {
            let list: Vec<T> = Mode::ALL.iter().map(|m| f(*m)).collect();
            for (i, a) in list.iter().enumerate() {
                for b in &list[i + 1..] {
                    assert_ne!(a, b, "{what} 有重复：{list:?}");
                }
            }
        }
        distinct("标签", Mode::label);
        distinct("符号", Mode::glyph);
        distinct("说明", Mode::describe);
    }

    #[test]
    fn parse_accepts_english_and_chinese_aliases() {
        assert_eq!(Mode::parse("PLAN"), Some(Mode::Plan));
        assert_eq!(Mode::parse(" p "), Some(Mode::Plan));
        assert_eq!(Mode::parse("逐条确认"), Some(Mode::Confirm));
        assert_eq!(Mode::parse("auto"), Some(Mode::Auto));
        assert_eq!(Mode::parse("全自动"), Some(Mode::Auto));
        assert_eq!(Mode::parse("nope"), None);
    }

    #[test]
    fn each_mode_tells_the_model_what_it_may_do() {
        for mode in Mode::ALL {
            let text = mode.instruction();
            assert!(text.contains(mode.title()), "{mode:?} 的提示词应当自报家门");
        }
        assert!(Mode::Plan.instruction().contains("不要调用 edit_file"));
        assert!(Mode::Confirm.instruction().contains("征求用户同意"));
        assert!(Mode::Auto.instruction().contains("不需要征求同意"));
    }

    #[test]
    fn default_mode_is_the_cautious_one() {
        assert_eq!(Mode::default(), Mode::Plan);
    }
}
