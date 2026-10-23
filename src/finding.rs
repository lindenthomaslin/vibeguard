//! 漏洞数据模型：扫描器产出，界面与修复流程消费。

use crate::i18n::tr;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;

/// 严重级别，按危害从低到高排列（`Ord` 依赖声明顺序）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    /// 从高到低，用于汇总展示
    pub const ALL: [Severity; 5] = [
        Severity::Critical,
        Severity::High,
        Severity::Medium,
        Severity::Low,
        Severity::Info,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Severity::Critical => tr("严重"),
            Severity::High => tr("高危"),
            Severity::Medium => tr("中危"),
            Severity::Low => tr("低危"),
            Severity::Info => tr("提示"),
        }
    }
}

/// 一条扫描发现
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// 稳定编号，如 `VG-001`，修复方案通过它关联
    pub id: String,
    pub severity: Severity,
    pub title: String,
    #[serde(default)]
    pub cwe: Option<String>,
    pub file: String,
    pub line: u32,
    pub description: String,
    pub suggestion: String,
}

/// 针对某条发现的修复方案：同一文件的修复前后全文
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub finding_id: String,
    pub file: String,
    pub old: String,
    pub new: String,
}

/// 按严重级别从高到低、再按文件和行号排序（稳定）
pub fn sorted(findings: &[Finding]) -> Vec<&Finding> {
    let mut v: Vec<&Finding> = findings.iter().collect();
    v.sort_by(|a, b| {
        (Reverse(a.severity), &a.file, a.line).cmp(&(Reverse(b.severity), &b.file, b.line))
    });
    v
}

/// 各级别的数量，顺序与 `Severity::ALL` 一致
pub fn counts(findings: &[Finding]) -> Vec<(Severity, usize)> {
    Severity::ALL
        .iter()
        .map(|&s| (s, findings.iter().filter(|f| f.severity == s).count()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn finding(id: &str, severity: Severity, file: &str, line: u32) -> Finding {
        Finding {
            id: id.into(),
            severity,
            title: format!("问题 {id}"),
            cwe: None,
            file: file.into(),
            line,
            description: String::new(),
            suggestion: String::new(),
        }
    }

    #[test]
    fn severity_orders_by_danger() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::Low > Severity::Info);
        let mut all = Severity::ALL;
        all.sort();
        assert_eq!(all[0], Severity::Info);
    }

    #[test]
    fn sorted_puts_worst_first_then_file_and_line() {
        let list = vec![
            finding("a", Severity::Low, "z.py", 1),
            finding("b", Severity::Critical, "m.py", 9),
            finding("c", Severity::Critical, "m.py", 2),
            finding("d", Severity::Critical, "a.py", 50),
        ];
        let ids: Vec<_> = sorted(&list).iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["d", "c", "b", "a"]);
    }

    #[test]
    fn counts_cover_every_level_in_order() {
        let list = vec![
            finding("a", Severity::High, "a", 1),
            finding("b", Severity::High, "a", 2),
            finding("c", Severity::Info, "a", 3),
        ];
        let c = counts(&list);
        assert_eq!(c.len(), 5);
        assert_eq!(c[0], (Severity::Critical, 0));
        assert_eq!(c[1], (Severity::High, 2));
        assert_eq!(c[4], (Severity::Info, 1));
    }

    #[test]
    fn finding_parses_from_llm_style_json() {
        let json = r#"{"id":"VG-001","severity":"high","title":"t","file":"a.py",
            "line":3,"description":"d","suggestion":"s"}"#;
        let f: Finding = serde_json::from_str(json).unwrap();
        assert_eq!(f.severity, Severity::High);
        assert_eq!(f.cwe, None);
    }
}
