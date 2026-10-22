//! 漏洞扫描引擎：执行清单中的检查，结合 AI 和工具

use crate::vuln_checklist::{Severity, VulnChecklist, VulnItem};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub vuln_id: String,
    pub title: String,
    pub verdict: ScanVerdict,     // 存在 / 疑似 / 不存在 / 无法判断
    pub evidence: String,          // 文件:行号 或其他证据
    pub repro_steps: String,       // 复现步骤
    pub fix_suggestion: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanVerdict {
    Exists,      // 存在
    Suspected,   // 疑似
    NotFound,    // 不存在
    Undetermined, // 无法判断
}

impl std::fmt::Display for ScanVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exists => write!(f, "存在"),
            Self::Suspected => write!(f, "疑似"),
            Self::NotFound => write!(f, "不存在"),
            Self::Undetermined => write!(f, "无法判断"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanPlan {
    pub mode: ScanMode,
    pub items: Vec<VulnItem>,
    pub total: usize,
    pub scanned: usize,
    pub results: Vec<ScanResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanMode {
    FullAuto,      // 完全自动：直接扫描并执行修复建议
    Interactive,   // 逐条确认：每条结果前停下让用户确认
    PlanThenExec,  // 先制定计划后执行：AI 先生成完整计划，再逐步执行
}

impl std::fmt::Display for ScanMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FullAuto => write!(f, "完全自动"),
            Self::Interactive => write!(f, "逐条确认"),
            Self::PlanThenExec => write!(f, "制定计划后执行"),
        }
    }
}

pub struct VulnScanner {
    checklist: VulnChecklist,
}

impl VulnScanner {
    pub fn new() -> Self {
        Self {
            checklist: VulnChecklist::new(),
        }
    }

    pub fn checklist(&self) -> &VulnChecklist {
        &self.checklist
    }

    /// 创建扫描计划：选择要扫描的漏洞和模式
    pub fn create_plan(
        &self,
        mode: ScanMode,
        severity_filter: Option<Severity>,
        category_filter: Option<&str>,
    ) -> ScanPlan {
        let mut items = Vec::new();

        if let Some(cat) = category_filter {
            items = self.checklist.by_category(cat).into_iter().cloned().collect();
        } else {
            items = self.checklist.all().to_vec();
        }

        if let Some(sev) = severity_filter {
            items.retain(|item| item.severity >= sev);
        }

        let total = items.len();

        ScanPlan {
            mode,
            items,
            total,
            scanned: 0,
            results: Vec::new(),
        }
    }

    /// 按严重程度排序漏洞
    pub fn prioritize_by_severity(items: &[VulnItem]) -> Vec<&VulnItem> {
        let mut sorted: Vec<_> = items.iter().collect();
        sorted.sort_by(|a, b| b.severity.cmp(&a.severity));
        sorted
    }

    /// 快速扫描（2小时优先级）
    pub fn quick_scan_items(&self) -> Vec<VulnItem> {
        // 根据清单中"两小时快速扫描"的优先级返回
        let priority_ids = vec![
            "AUTH-01", "AUTHZ-01", "CONF-03", "CONF-04", "API-02", "API-03",
            "SUP-01", "FILE-01", "FILE-03", "QUAL-03", "CONF-02",
        ];

        priority_ids
            .iter()
            .filter_map(|id| self.checklist.get(id).cloned())
            .collect()
    }

    /// 按分类统计结果
    pub fn summarize_by_category<'a>(
        &self,
        results: &'a [ScanResult],
    ) -> HashMap<String, Vec<&'a ScanResult>> {
        let mut by_cat: HashMap<String, Vec<&'a ScanResult>> = HashMap::new();

        for result in results {
            if let Some(item) = self.checklist.get(&result.vuln_id) {
                by_cat
                    .entry(item.category.clone())
                    .or_insert_with(Vec::new)
                    .push(result);
            }
        }

        by_cat
    }

    /// 按严重程度统计结果
    pub fn summarize_by_severity<'a>(
        &self,
        results: &'a [ScanResult],
    ) -> HashMap<Severity, Vec<&'a ScanResult>> {
        let mut by_sev: HashMap<Severity, Vec<&'a ScanResult>> = HashMap::new();

        for result in results {
            if let Some(item) = self.checklist.get(&result.vuln_id) {
                by_sev
                    .entry(item.severity)
                    .or_insert_with(Vec::new)
                    .push(result);
            }
        }

        by_sev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_plan() {
        let scanner = VulnScanner::new();
        let plan = scanner.create_plan(ScanMode::FullAuto, None, None);
        assert!(!plan.items.is_empty());
    }

    #[test]
    fn test_quick_scan() {
        let scanner = VulnScanner::new();
        let items = scanner.quick_scan_items();
        assert!(items.len() >= 5);
    }

    #[test]
    fn test_prioritize() {
        let items = VulnScanner::new().checklist().all().to_vec();
        let prioritized = VulnScanner::prioritize_by_severity(&items);
        assert!(!prioritized.is_empty());
        // 第一个应该是 Critical
        assert_eq!(prioritized[0].severity, Severity::Critical);
    }
}
