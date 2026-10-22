//! 扫描模式状态管理

use crate::vuln_scanner::{ScanMode, ScanPlan, ScanResult, VulnScanner};
use crate::vuln_checklist::Severity;

#[derive(Debug, Clone)]
pub enum ScanState {
    Idle,
    SelectingMode,                 // 选择扫描模式
    ConfiguringFilters,            // 配置过滤条件
    Scanning(ScanPlan),            // 正在扫描
    ReviewingResults(Vec<ScanResult>), // 审查结果
    PlanReview(String),            // 制定计划，展示 AI 生成的计划文本
}

pub struct ScanSession {
    pub scanner: VulnScanner,
    pub state: ScanState,
    pub selected_mode: Option<ScanMode>,
    pub severity_filter: Option<Severity>,
    pub category_filter: Option<String>,
}

impl ScanSession {
    pub fn new() -> Self {
        Self {
            scanner: VulnScanner::new(),
            state: ScanState::Idle,
            selected_mode: None,
            severity_filter: None,
            category_filter: None,
        }
    }

    pub fn start_scan(&mut self) -> &mut ScanPlan {
        if let Some(mode) = self.selected_mode {
            let plan = self.scanner.create_plan(
                mode,
                self.severity_filter,
                self.category_filter.as_deref(),
            );
            self.state = ScanState::Scanning(plan);

            if let ScanState::Scanning(ref mut p) = self.state {
                return p;
            }
        }
        unreachable!()
    }

    pub fn set_mode(&mut self, mode: ScanMode) {
        self.selected_mode = Some(mode);
    }

    pub fn set_severity_filter(&mut self, severity: Severity) {
        self.severity_filter = Some(severity);
    }

    pub fn clear_severity_filter(&mut self) {
        self.severity_filter = None;
    }

    pub fn set_category_filter(&mut self, category: String) {
        self.category_filter = Some(category);
    }

    pub fn clear_category_filter(&mut self) {
        self.category_filter = None;
    }

    pub fn reset(&mut self) {
        self.state = ScanState::Idle;
        self.selected_mode = None;
        self.severity_filter = None;
        self.category_filter = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_session() {
        let mut session = ScanSession::new();
        session.set_mode(ScanMode::FullAuto);
        assert_eq!(session.selected_mode, Some(ScanMode::FullAuto));
    }
}
