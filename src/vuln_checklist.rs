//! 漏洞检查清单：410 条规则，从 `assets/checklist.tsv` 编译进二进制。
//!
//! 这份清单是扫描与评审的核对表，不是「一定能自动判定」的规则集：
//! 其中一部分能靠代码特征（正则）直接命中，一部分要靠模型读代码做语义判断，
//! 还有一部分（约 39%）本质上是运行时行为——比如「连打 100 次密码看有没有锁定」——
//! 静态分析只能给出「无法判断」。`detect_point` 就是给模型看的判断依据。

use crate::finding::Severity;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 编译进二进制的清单。改 `assets/checklist.tsv` 后重新 `cargo build` 即可生效。
const CHECKLIST_TSV: &str = include_str!("../assets/checklist.tsv");

/// 英文界面用的标题与修复建议，按 id 挂在上面那份清单上。
///
/// 单独一个文件而不是给 checklist.tsv 加列：那份是生成的，重跑生成器会把加进去的列冲掉。
/// 缺 id 就退回中文——少一句译文不该让扫描跑不起来。
const CHECKLIST_EN_TSV: &str = include_str!("../assets/checklist.en.tsv");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VulnItem {
    /// 清单编号，如 `AUTH-01`
    pub id: String,
    pub title: String,
    pub severity: Severity,
    /// 所属大类，如 `A. 认证与账号`
    pub category: String,
    /// 怎么扫——同时也是给模型判断的提示
    pub detect_point: String,
    /// 修复要点
    pub fix_point: String,
    /// 英文标题；空表示这份清单没有英文版，显示时退回中文
    #[serde(default)]
    pub title_en: String,
    /// 英文修复建议
    #[serde(default)]
    pub fix_en: String,
    /// 对应的 CWE 编号，没有就是 `None`
    pub cwe: Option<String>,
}

pub struct VulnChecklist {
    items: Vec<VulnItem>,
    by_id: HashMap<String, usize>,
    by_category: HashMap<String, Vec<usize>>,
}

impl Default for VulnChecklist {
    fn default() -> Self {
        Self::new()
    }
}

impl VulnItem {
    /// 标题按界面语言取。英文缺失时退回中文——宁可露出一句中文，
    /// 也不要显示空字符串让整行看不出是什么问题。
    pub fn display_title(&self) -> &str {
        self.title_in(crate::i18n::current())
    }

    /// 修复建议，同上
    pub fn display_fix(&self) -> &str {
        self.fix_in(crate::i18n::current())
    }

    /// 指定语言取标题。分开一个方法是为了能测——界面语言是进程级的全局状态，
    /// 测试里切来切去会互相干扰。
    pub fn title_in(&self, lang: crate::i18n::Lang) -> &str {
        pick_text(lang, &self.title, &self.title_en)
    }

    pub fn fix_in(&self, lang: crate::i18n::Lang) -> &str {
        pick_text(lang, &self.fix_point, &self.fix_en)
    }
}

fn pick_text<'a>(lang: crate::i18n::Lang, zh: &'a str, en: &'a str) -> &'a str {
    if lang == crate::i18n::Lang::Zh || en.is_empty() {
        zh
    } else {
        en
    }
}

// `by_category` / `by_severity` / `categories` 是给第二阶段（按分类分批喂给模型）准备的，
// 现在只有测试在用；先留着，比删掉再写回来便宜。
#[allow(dead_code)]
impl VulnChecklist {
    pub fn new() -> Self {
        Self::from_tsv_with_en(CHECKLIST_TSV, CHECKLIST_EN_TSV)
    }

    /// 主清单 + 英文覆盖层
    pub fn from_tsv_with_en(text: &str, en_text: &str) -> Self {
        let mut list = Self::from_tsv(text);
        let overlay: HashMap<&str, (&str, &str)> = en_text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .filter_map(|l| {
                let cells: Vec<&str> = l.split('\t').collect();
                (cells.len() >= 3).then(|| (cells[0].trim(), (cells[1].trim(), cells[2].trim())))
            })
            .collect();
        for item in &mut list.items {
            if let Some((title, fix)) = overlay.get(item.id.as_str()) {
                item.title_en = (*title).to_string();
                item.fix_en = (*fix).to_string();
            }
        }
        list
    }

    /// 解析 TSV。格式：`id \t 等级 \t 分类 \t 标题 \t 检测点 \t 修复要点 \t CWE`。
    /// `#` 开头是注释，空行忽略，列数不对的行跳过（不让一条脏数据毁掉整份清单）。
    pub fn from_tsv(text: &str) -> Self {
        let items: Vec<VulnItem> = text.lines().filter_map(parse_row).collect();
        let mut by_id = HashMap::with_capacity(items.len());
        let mut by_category: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, item) in items.iter().enumerate() {
            by_id.insert(item.id.clone(), idx);
            by_category
                .entry(item.category.clone())
                .or_default()
                .push(idx);
        }
        Self {
            items,
            by_id,
            by_category,
        }
    }

    pub fn get(&self, id: &str) -> Option<&VulnItem> {
        self.by_id.get(id).and_then(|&idx| self.items.get(idx))
    }

    /// 按分类取，顺序与清单一致
    pub fn by_category(&self, category: &str) -> Vec<&VulnItem> {
        self.by_category
            .get(category)
            .map(|ids| ids.iter().filter_map(|&i| self.items.get(i)).collect())
            .unwrap_or_default()
    }

    pub fn all(&self) -> &[VulnItem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn by_severity(&self, severity: Severity) -> Vec<&VulnItem> {
        self.items
            .iter()
            .filter(|item| item.severity == severity)
            .collect()
    }

    /// 所有分类名，按清单里的字母序（A、B、C…）
    pub fn categories(&self) -> Vec<&str> {
        let mut cats: Vec<_> = self.by_category.keys().map(String::as_str).collect();
        cats.sort();
        cats
    }
}

fn parse_row(line: &str) -> Option<VulnItem> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.trim().is_empty() || line.starts_with('#') {
        return None;
    }
    let cells: Vec<&str> = line.split('\t').collect();
    if cells.len() != 7 {
        return None;
    }
    let severity = match cells[1] {
        "critical" => Severity::Critical,
        "high" => Severity::High,
        "medium" => Severity::Medium,
        "low" => Severity::Low,
        _ => return None,
    };
    let cwe = cells[6].trim();
    Some(VulnItem {
        id: cells[0].to_string(),
        severity,
        category: cells[2].to_string(),
        title: cells[3].to_string(),
        detect_point: cells[4].to_string(),
        fix_point: cells[5].to_string(),
        title_en: String::new(),
        fix_en: String::new(),
        cwe: (!cwe.is_empty() && cwe != "—").then(|| cwe.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_checklist_is_complete() {
        let list = VulnChecklist::new();
        assert_eq!(list.len(), 410, "清单被截断了");
        assert_eq!(list.categories().len(), 19);
        // 分级数量与清单总览表一致
        assert_eq!(list.by_severity(Severity::Critical).len(), 53);
        assert_eq!(list.by_severity(Severity::High).len(), 145);
        assert_eq!(list.by_severity(Severity::Medium).len(), 176);
        assert_eq!(list.by_severity(Severity::Low).len(), 36);
    }

    #[test]
    fn every_row_has_an_id_a_title_and_a_detect_point() {
        let list = VulnChecklist::new();
        for item in list.all() {
            assert!(!item.id.is_empty());
            assert!(!item.title.is_empty(), "{} 没有标题", item.id);
            assert!(!item.detect_point.is_empty(), "{} 没有检测点", item.id);
            assert!(!item.category.is_empty(), "{} 没有分类", item.id);
        }
        let mut ids: Vec<&str> = list.all().iter().map(|i| i.id.as_str()).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "编号有重复");
    }

    #[test]
    fn lookup_and_category_index_work() {
        let list = VulnChecklist::new();
        let item = list.get("AUTH-01").expect("AUTH-01");
        assert_eq!(item.severity, Severity::Critical);
        assert_eq!(item.cwe.as_deref(), Some("CWE-306"));
        assert!(list.get("NOPE-99").is_none());

        let auth = list.by_category("A. 认证与账号");
        assert_eq!(auth.len(), 24);
        assert!(auth.iter().all(|i| i.category == "A. 认证与账号"));
    }

    #[test]
    fn parser_skips_junk_without_losing_good_rows() {
        let text = "\
# 注释
BAD\tline

GOOD-01\thigh\tA. 分类\t标题\t检测点\t修复要点\tCWE-1
BAD2\tnot-a-severity\tA. 分类\t标题\t检测点\t修复\tCWE-2
GOOD-02\tlow\tA. 分类\t标题\t检测点\t修复要点\t
";
        let list = VulnChecklist::from_tsv(text);
        let ids: Vec<_> = list.all().iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["GOOD-01", "GOOD-02"]);
        assert_eq!(list.get("GOOD-01").unwrap().severity, Severity::High);
        assert_eq!(list.get("GOOD-02").unwrap().cwe, None, "空 CWE 应当是 None");
    }
    #[test]
    fn every_item_has_an_english_title_and_fix() {
        let list = VulnChecklist::new();
        let missing: Vec<&str> = list
            .all()
            .iter()
            .filter(|i| i.title_en.is_empty() || i.fix_en.is_empty())
            .map(|i| i.id.as_str())
            .collect();
        assert!(
            missing.is_empty(),
            "这些条目没有英文版，英文界面会露出中文：{missing:?}"
        );
    }

    #[test]
    fn the_english_side_has_no_chinese_left_in_it() {
        let han = |s: &str| s.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
        let list = VulnChecklist::new();
        for item in list.all() {
            assert!(!han(&item.title_en), "{} 的英文标题还是中文", item.id);
            assert!(!han(&item.fix_en), "{} 的英文建议还是中文", item.id);
        }
    }

    #[test]
    fn the_language_switch_picks_the_right_column() {
        use crate::i18n::Lang;
        let list = VulnChecklist::new();
        let item = list.get("AUTHZ-01").expect("AUTHZ-01");
        assert_eq!(item.title_in(Lang::Zh), item.title);
        assert_eq!(item.title_in(Lang::En), item.title_en);
        assert_ne!(item.title_in(Lang::En), item.title);
        assert_eq!(item.fix_in(Lang::En), item.fix_en);
    }

    #[test]
    fn a_missing_english_row_falls_back_to_chinese() {
        use crate::i18n::Lang;
        let zh = "AUTH-01\tcritical\tA. 认证\t中文标题\t检测点\t中文建议\tCWE-306\n";
        let list = VulnChecklist::from_tsv_with_en(zh, "");
        let item = list.get("AUTH-01").unwrap();
        assert_eq!(
            item.title_in(Lang::En),
            "中文标题",
            "缺英文时不能变成空字符串"
        );
        assert_eq!(item.fix_in(Lang::En), "中文建议");
    }
}
