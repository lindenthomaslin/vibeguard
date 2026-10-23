//! 扫描报告导出：把对话区里的结果落成文件。
//!
//! 同时出两份，因为两种读者要的东西不一样：
//! - `vibeguard-report.md` 给人看：结论、按级别排好的清单、每条的位置和修复建议；
//! - `vibeguard-report.json` 给 CI 看：同样的数据，机器读。
//!
//! 报告里必须写清楚**覆盖面**。一份「0 个问题」的报告，如果没说它只看了
//! 410 条里的 20 条，就是在骗人——这是这个工具最容易犯的错。

use crate::finding::{Finding, Severity, counts, sorted};
use crate::scan_ai::{Judgement, Verdict};
use serde::Serialize;

/// 报告头部的元信息
pub struct Meta {
    /// 扫描的目录
    pub root: String,
    /// 生成时间（已格式化好）
    pub when: String,
    /// 工具版本
    pub version: String,
    /// 本地规则覆盖了多少条清单
    pub local_rules: usize,
    /// 清单总条数
    pub checklist_total: usize,
    /// 扫了多少文件
    pub files_scanned: usize,
    /// 判定失败的批次：有值就说明这次覆盖不完整
    pub failed: Vec<String>,
    /// 证据核实不了、已剔除的条目：报告里必须写，否则数字对不上
    pub unverified: Vec<String>,
}

impl Meta {
    /// 这一次总共看了清单里的多少条
    fn covered(&self, judgements: &[Judgement]) -> usize {
        let judged = judgements.len();
        (self.local_rules + judged).min(self.checklist_total)
    }
}

/// 生成 Markdown 报告
pub fn markdown(findings: &[Finding], judgements: &[Judgement], meta: &Meta) -> String {
    let mut out = String::new();
    out.push_str("# VibeGuard 扫描报告\n\n");
    out.push_str("| | |\n|---|---|\n");
    out.push_str(&format!("| 扫描目标 | `{}` |\n", meta.root));
    out.push_str(&format!("| 生成时间 | {} |\n", meta.when));
    out.push_str(&format!("| 工具版本 | vibeguard {} |\n", meta.version));
    out.push_str(&format!("| 扫描文件 | {} 个 |\n", meta.files_scanned));
    out.push_str(&format!(
        "| 清单覆盖 | {}/{} 条（本地规则 {} 条 + 模型判定 {} 条） |\n\n",
        meta.covered(judgements),
        meta.checklist_total,
        meta.local_rules,
        judgements.len()
    ));

    if !meta.failed.is_empty() {
        out.push_str(&format!(
            "\n> ⚠ 有 {} 批判定没有跑成功，上面的覆盖数字**不完整**。\
             \n> 修好网络/额度之后重跑 `/scan --ai`，否则这些条目等于没看。\n\n",
            meta.failed.len()
        ));
        for f in &meta.failed {
            out.push_str(&format!("> - {f}\n"));
        }
        out.push('\n');
    }

    if !meta.unverified.is_empty() {
        out.push_str(&format!(
            "\n> ⚠ 有 {} 条模型判定找不到对应代码（文件不存在或行号超范围），**已从下面的清单里剔除**。\n> 剔除不等于没问题：这些条目可能确实存在，只是模型指错了地方，值得手工看一眼。\n\n",
            meta.unverified.len()
        ));
        for u in &meta.unverified {
            out.push_str(&format!("> - {u}\n"));
        }
        out.push('\n');
    }

    // ---- 结论 ----
    out.push_str("## 结论\n\n");
    if findings.is_empty() {
        out.push_str("本次未发现已确认的问题。\n\n");
    } else {
        out.push_str(&format!("共发现 **{}** 个问题：\n\n", findings.len()));
        out.push_str("| 级别 | 数量 |\n|---|---:|\n");
        for (sev, n) in counts(findings).into_iter().filter(|&(_, n)| n > 0) {
            out.push_str(&format!("| {} | {} |\n", sev.label(), n));
        }
        out.push('\n');
    }

    if !judgements.is_empty() {
        let count = |v: Verdict| judgements.iter().filter(|j| j.verdict == v).count();
        out.push_str("| 模型判定 | 条数 |\n|---|---:|\n");
        for v in [
            Verdict::Exists,
            Verdict::Suspected,
            Verdict::NotFound,
            Verdict::Undetermined,
        ] {
            out.push_str(&format!("| {} | {} |\n", v.label(), count(v)));
        }
        out.push('\n');
        if count(Verdict::Undetermined) > 0 {
            out.push_str(&format!(
                "> {} 条被判为「无法判断」：它们需要把服务跑起来发请求才能确认\
                 （限流、爆破、越权、响应头这类）。**不要把它们当成「没问题」。**\n\n",
                count(Verdict::Undetermined)
            ));
        }
    }

    // ---- 明细 ----
    if !findings.is_empty() {
        out.push_str("## 发现明细\n\n");
        for (i, f) in sorted(findings).into_iter().enumerate() {
            out.push_str(&format!(
                "### {}. [{}] {}\n\n",
                i + 1,
                f.severity.label(),
                f.title
            ));
            out.push_str(&format!("- **位置**：`{}:{}`\n", f.file, f.line));
            if let Some(cwe) = &f.cwe {
                out.push_str(&format!("- **编号**：{} · {}\n", checklist_id(f), cwe));
            } else {
                out.push_str(&format!("- **编号**：{}\n", checklist_id(f)));
            }
            if !f.description.is_empty() {
                out.push_str(&format!("- **说明**：{}\n", f.description));
            }
            if !f.suggestion.is_empty() {
                out.push_str(&format!("- **修复建议**：{}\n", f.suggestion));
            }
            out.push('\n');
        }
    }

    // ---- 附注 ----
    out.push_str("## 说明\n\n");
    out.push_str(
        "- 本地规则命中是**确定**的：它看到了具体的代码特征（字符串里的密钥、`shell=True`、`eval` 调用）。\n\
         - 「疑似」是模型读到相关代码但证据不足，需要人工确认。\n\
         - 本报告由 VibeGuard 生成，是审计的**起点**不是结论；每条都要人看过再改。\n",
    );
    out
}

/// `[CONF-01] 密钥写死` → `CONF-01`
fn checklist_id(f: &Finding) -> &str {
    f.description
        .split(']')
        .next()
        .unwrap_or("")
        .trim_start_matches('[')
}

#[derive(Serialize)]
struct JsonReport<'a> {
    tool: &'static str,
    version: &'a str,
    generated_at: &'a str,
    root: &'a str,
    files_scanned: usize,
    coverage: Coverage,
    summary: Vec<SummaryRow>,
    findings: &'a [Finding],
    verdicts: Vec<VerdictRow>,
}

#[derive(Serialize)]
struct Coverage {
    local_rules: usize,
    model_judged: usize,
    checklist_total: usize,
    covered: usize,
    failed_batches: Vec<String>,
    /// 证据核实不了、已剔除的条目
    unverified: Vec<String>,
}

#[derive(Serialize)]
struct SummaryRow {
    severity: String,
    count: usize,
}

#[derive(Serialize)]
struct VerdictRow {
    id: String,
    verdict: String,
    evidence: String,
    reason: String,
}

/// 生成 JSON 报告。CI 里 `jq '.summary'` 就能拿到分级数量。
pub fn json(findings: &[Finding], judgements: &[Judgement], meta: &Meta) -> String {
    let report = JsonReport {
        tool: "vibeguard",
        version: &meta.version,
        generated_at: &meta.when,
        root: &meta.root,
        files_scanned: meta.files_scanned,
        coverage: Coverage {
            local_rules: meta.local_rules,
            model_judged: judgements.len(),
            checklist_total: meta.checklist_total,
            covered: meta.covered(judgements),
            failed_batches: meta.failed.clone(),
            unverified: meta.unverified.clone(),
        },
        summary: counts(findings)
            .into_iter()
            .map(|(sev, n)| SummaryRow {
                severity: match sev {
                    Severity::Critical => "critical",
                    Severity::High => "high",
                    Severity::Medium => "medium",
                    Severity::Low => "low",
                    Severity::Info => "info",
                }
                .to_string(),
                count: n,
            })
            .collect(),
        findings,
        verdicts: judgements
            .iter()
            .map(|j| VerdictRow {
                id: j.id.clone(),
                verdict: j.verdict.label().to_string(),
                evidence: j.evidence.clone(),
                reason: j.reason.clone(),
            })
            .collect(),
    };
    serde_json::to_string_pretty(&report).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Meta {
        Meta {
            root: "/tmp/demo".into(),
            when: "2026-10-23 09:00".into(),
            version: "0.1.0".into(),
            local_rules: 20,
            checklist_total: 410,
            files_scanned: 6,
            failed: Vec::new(),
            unverified: Vec::new(),
        }
    }

    fn finding(id: &str, sev: Severity, file: &str, line: u32) -> Finding {
        Finding {
            id: id.into(),
            severity: sev,
            title: "硬编码密钥".into(),
            cwe: Some("CWE-798".into()),
            file: file.into(),
            line,
            description: "[CONF-01] 密钥写死".into(),
            suggestion: "改从环境变量读".into(),
        }
    }

    #[test]
    fn markdown_states_the_coverage_so_a_clean_report_is_not_misleading() {
        let md = markdown(&[], &[], &meta());
        assert!(md.contains("清单覆盖 | 20/410"));
        assert!(md.contains("本次未发现已确认的问题"));
        assert!(md.contains("不要把它们当成「没问题」") || !md.contains("无法判断"));
    }

    #[test]
    fn markdown_lists_findings_worst_first_with_location_and_fix() {
        let findings = vec![
            finding("VG-001", Severity::Low, "b.py", 9),
            finding("VG-002", Severity::Critical, "a.py", 3),
        ];
        let md = markdown(&findings, &[], &meta());
        let critical = md.find("a.py:3").unwrap();
        let low = md.find("b.py:9").unwrap();
        assert!(critical < low, "严重的要排在前面");
        assert!(md.contains("CWE-798"));
        assert!(md.contains("改从环境变量读"));
        assert!(md.contains("共发现 **2** 个问题"));
    }

    #[test]
    fn undetermined_verdicts_are_called_out_explicitly() {
        let js = vec![Judgement {
            id: "AUTH-07".into(),
            verdict: Verdict::Undetermined,
            evidence: "需要跑起来".into(),
            reason: "限流只能运行时看".into(),
        }];
        let md = markdown(&[], &js, &meta());
        assert!(md.contains("1 条被判为「无法判断」"));
        assert!(md.contains("不要把它们当成「没问题」"));
        assert!(md.contains("清单覆盖 | 21/410"), "模型判过的也要算进覆盖");
    }

    #[test]
    fn unverified_evidence_is_called_out_in_both_formats() {
        let mut m = meta();
        m.unverified = vec!["AUTHZ-01 api/orders.py:99（文件只有 40 行）".into()];
        let md = markdown(&[], &[], &m);
        assert!(md.contains("找不到对应代码"), "{md}");
        assert!(md.contains("剔除不等于没问题"), "别让人以为剔除就是没事了");
        assert!(md.contains("AUTHZ-01 api/orders.py:99"));
        let v: serde_json::Value = serde_json::from_str(&json(&[], &[], &m)).unwrap();
        assert_eq!(
            v["coverage"]["unverified"][0],
            "AUTHZ-01 api/orders.py:99（文件只有 40 行）"
        );
    }

    #[test]
    fn failed_batches_are_called_out_in_both_formats() {
        let mut m = meta();
        m.failed = vec!["D. 注入类 等 4 类（超时）".into()];
        let md = markdown(&[], &[], &m);
        assert!(md.contains("有 1 批判定没有跑成功"));
        assert!(md.contains("不完整"));
        assert!(md.contains("D. 注入类"));
        let v: serde_json::Value = serde_json::from_str(&json(&[], &[], &m)).unwrap();
        assert_eq!(
            v["coverage"]["failed_batches"][0],
            "D. 注入类 等 4 类（超时）"
        );
    }

    #[test]
    fn json_is_machine_readable_and_carries_the_same_numbers() {
        let findings = vec![finding("VG-001", Severity::Critical, "a.py", 3)];
        let js = json(&findings, &[], &meta());
        let v: serde_json::Value = serde_json::from_str(&js).unwrap();
        assert_eq!(v["tool"], "vibeguard");
        assert_eq!(v["coverage"]["covered"], 20);
        assert_eq!(v["summary"][0]["severity"], "critical");
        assert_eq!(v["summary"][0]["count"], 1);
        assert_eq!(v["findings"][0]["file"], "a.py");
    }
}
