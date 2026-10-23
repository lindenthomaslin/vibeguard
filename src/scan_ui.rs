//! 扫描与修复界面的渲染：生成 ratatui 的 `Line`，由全屏界面放进对话区。

use crate::chat::wrap;
use crate::finding::{Finding, Fix, Severity, counts};
use crate::theme::Look;
use console::truncate_str;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use similar::{ChangeTag, TextDiff};

const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);

/// 严重级别配色：严重/高危用错误色，中危用警告色，低危用信息色
pub fn severity_style(s: Severity, look: &Look) -> Style {
    match s {
        Severity::Critical => look.error().add_modifier(Modifier::BOLD),
        Severity::High => look.error(),
        Severity::Medium => look.warn(),
        Severity::Low => look.info(),
        Severity::Info => look.muted(),
    }
}

pub fn badge(s: Severity, look: &Look) -> Span<'static> {
    Span::styled(format!("[{}]", s.label()), severity_style(s, look))
}

fn clip(s: &str, width: usize) -> String {
    truncate_str(s, width, "…").to_string()
}

/// 一条发现的卡片：级别、编号、标题、位置、说明与建议
pub fn finding_card(
    f: &Finding,
    index: usize,
    total: usize,
    width: usize,
    look: &Look,
) -> Vec<Line<'static>> {
    let body_w = width.max(30) - 2;
    let mut out = vec![Line::from(vec![
        Span::styled(format!("{} ", look.glyphs.info), look.accent()),
        badge(f.severity, look),
        Span::raw(" "),
        Span::styled(f.id.clone(), BOLD),
        Span::styled(format!("  ({index}/{total})"), look.muted()),
    ])];
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(clip(&f.title, body_w), BOLD),
    ]));
    let loc = match &f.cwe {
        Some(cwe) => format!("{}:{} · {}", f.file, f.line, cwe),
        None => format!("{}:{}", f.file, f.line),
    };
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(clip(&loc, body_w), look.muted()),
    ]));
    for row in wrap(&f.description, body_w) {
        if !row.is_empty() {
            out.push(Line::raw(format!("  {row}")));
        }
    }
    if !f.suggestion.is_empty() {
        for row in wrap(
            &crate::i18n::pick(
                format!("建议：{}", f.suggestion),
                format!("Fix: {}", f.suggestion),
            ),
            body_w,
        ) {
            out.push(Line::styled(format!("  {row}"), look.success()));
        }
    }
    out
}

/// 汇总：总数和各级别的横条
pub fn summary_lines(findings: &[Finding], look: &Look) -> Vec<Line<'static>> {
    if findings.is_empty() {
        return vec![Line::from(vec![
            Span::styled(
                format!("{} ", look.glyphs.success),
                look.success().add_modifier(Modifier::BOLD),
            ),
            Span::raw(crate::i18n::t("未发现安全问题", "No security issues found")),
        ])];
    }
    let c = counts(findings);
    let max = c.iter().map(|&(_, n)| n).max().unwrap_or(1).max(1);
    let mut out = vec![Line::from(vec![
        Span::styled(format!("{} ", look.glyphs.info), look.accent()),
        Span::raw(crate::i18n::t("共发现 ", "Found ")),
        Span::styled(findings.len().to_string(), BOLD),
        Span::raw(crate::i18n::t(" 个问题", " issues")),
    ])];
    for (sev, n) in c.into_iter().filter(|&(_, n)| n > 0) {
        let st = severity_style(sev, look);
        out.push(Line::from(vec![
            Span::styled(format!("  {:<2}  ", sev.label()), st),
            Span::styled("█".repeat((n * 20).div_ceil(max)), st),
            Span::raw(format!(" {n}")),
        ]));
    }
    out
}

/// 一次扫描的完整结果：汇总条 + 逐条列表（按级别从高到低）
pub fn report_lines(report: &crate::scan::LocalReport, look: &Look) -> Vec<Line<'static>> {
    let crate::scan::LocalReport {
        findings,
        files_scanned,
        files_skipped,
        files_with_hits,
        truncated,
        rules,
        checklist_total,
    } = report;
    /// 内联最多列这么多条，再多就只给计数——不然一次扫描能把对话刷走
    const MAX_LISTED: usize = 60;

    let mut out = vec![Line::from(vec![
        Span::styled(format!("{} ", look.glyphs.info), look.accent()),
        Span::raw(crate::i18n::t("扫描了 ", "Scanned ")),
        Span::styled(files_scanned.to_string(), BOLD),
        Span::raw(crate::i18n::t(" 个文件，", " files, ")),
        Span::styled(files_with_hits.to_string(), BOLD),
        Span::raw(crate::i18n::t(" 个有问题", " with findings")),
        Span::styled(
            crate::i18n::pick(
                format!("（跳过 {files_skipped} 个：非文本、超大或已标注忽略）"),
                format!(" (skipped {files_skipped}: binary, oversized or marked ignored)"),
            ),
            look.muted(),
        ),
    ])];
    out.push(Line::styled(
        crate::i18n::pick(
            format!(
                "  本地规则覆盖清单里的 {rules}/{checklist_total} 条；其余需要模型做语义判断，这一轮还没跑"
            ),
            format!(
                "  Local rules cover {rules}/{checklist_total} checklist items; the rest need the model"
            ),
        ),
        look.muted(),
    ));
    out.push(Line::raw(""));
    out.extend(summary_lines(findings, look));

    let sorted = crate::finding::sorted(findings);
    if !sorted.is_empty() {
        out.push(Line::raw(""));
        for f in sorted.iter().take(MAX_LISTED) {
            out.push(Line::from(vec![
                badge(f.severity, look),
                Span::raw(" "),
                Span::styled(
                    format!("{}:{}", f.file, f.line),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::raw(f.title.clone()),
                Span::styled(
                    format!(
                        "  [{}{}]",
                        f.description
                            .split(']')
                            .next()
                            .unwrap_or("")
                            .trim_start_matches('['),
                        f.cwe
                            .as_deref()
                            .map(|c| format!(" · {c}"))
                            .unwrap_or_default()
                    ),
                    look.muted(),
                ),
            ]));
        }
        if sorted.len() > MAX_LISTED {
            out.push(Line::styled(
                crate::i18n::pick(
                    format!("  … 还有 {} 条未列出", sorted.len() - MAX_LISTED),
                    format!("  … {} more not listed", sorted.len() - MAX_LISTED),
                ),
                look.muted(),
            ));
        }
    }
    if *truncated {
        out.push(Line::styled(
            crate::i18n::t(
                "  结果超过上限，已截断（先修掉手头这些，或调大 MAX_FINDINGS）",
                "  Results truncated at the cap (fix these first, or raise MAX_FINDINGS)",
            ),
            look.warn(),
        ));
    }
    out
}

/// 第二段的判定结果：按判定类型汇总，只列出「存在 / 疑似」的明细
pub fn ai_lines(
    judgements: &[crate::scan_ai::Judgement],
    failed: &[String],
    look: &Look,
) -> Vec<Line<'static>> {
    use crate::scan_ai::Verdict;
    /// 明细最多列这么多条
    const MAX_LISTED: usize = 40;

    let count = |v: Verdict| judgements.iter().filter(|j| j.verdict == v).count();
    let (exists, suspected, not_found, undetermined) = (
        count(Verdict::Exists),
        count(Verdict::Suspected),
        count(Verdict::NotFound),
        count(Verdict::Undetermined),
    );

    let mut out = vec![Line::from(vec![
        Span::styled(format!("{} ", look.glyphs.thinking), look.accent()),
        Span::raw(crate::i18n::t("模型判定了 ", "The model judged ")),
        Span::styled(judgements.len().to_string(), BOLD),
        Span::raw(crate::i18n::t(" 条清单：", " items: ")),
        Span::styled(
            crate::i18n::pick(format!("存在 {exists}"), format!("{exists} exists")),
            look.error(),
        ),
        Span::raw(" · "),
        Span::styled(
            crate::i18n::pick(
                format!("疑似 {suspected}"),
                format!("{suspected} suspected"),
            ),
            look.warn(),
        ),
        Span::styled(
            crate::i18n::pick(
                format!(" · 不存在 {not_found} · 无法判断 {undetermined}"),
                format!(" · {not_found} not found · {undetermined} undetermined"),
            ),
            look.muted(),
        ),
    ])];

    let hits: Vec<_> = judgements
        .iter()
        .filter(|j| matches!(j.verdict, Verdict::Exists | Verdict::Suspected))
        .take(MAX_LISTED)
        .collect();
    if !hits.is_empty() {
        out.push(Line::raw(""));
    }
    for j in hits {
        let style = if j.verdict == Verdict::Exists {
            look.error()
        } else {
            look.warn()
        };
        out.push(Line::from(vec![
            Span::styled(format!("  {} ", j.verdict.label()), style),
            Span::styled(j.id.clone(), BOLD),
            Span::raw("  "),
            Span::styled(
                j.evidence.clone(),
                Style::new().add_modifier(Modifier::UNDERLINED),
            ),
            Span::raw("  "),
            Span::styled(j.reason.clone(), look.muted()),
        ]));
    }
    if undetermined > 0 {
        out.push(Line::styled(
            crate::i18n::pick(
                format!(
                    "  {undetermined} 条静态看不出来——它们要跑起来发请求才能验，别当成「没问题」"
                ),
                format!("  {undetermined} cannot be decided statically — they need a running service, so do not read them as clean"),
            ),
            look.muted(),
        ));
    }
    for f in failed {
        out.push(Line::styled(
            crate::i18n::pick(
                format!("  有一批没跑成功：{f}"),
                format!("  A batch failed: {f}"),
            ),
            look.error(),
        ));
    }
    out
}

/// 新增 / 删除的行数
pub fn diff_stats(old: &str, new: &str) -> (usize, usize) {
    TextDiff::from_lines(old, new)
        .iter_all_changes()
        .fold((0, 0), |(a, r), c| match c.tag() {
            ChangeTag::Insert => (a + 1, r),
            ChangeTag::Delete => (a, r + 1),
            ChangeTag::Equal => (a, r),
        })
}

/// 带行号的 diff，只显示改动及其前后 `context` 行
pub fn diff_lines(fix: &Fix, context: usize, width: usize, look: &Look) -> Vec<Line<'static>> {
    let (added, removed) = diff_stats(&fix.old, &fix.new);
    let mut out = vec![Line::from(vec![
        Span::styled("── ", look.muted()),
        Span::styled(fix.file.clone(), BOLD),
        Span::raw("  "),
        Span::styled(format!("+{added}"), look.success()),
        Span::raw(" "),
        Span::styled(format!("-{removed}"), look.error()),
    ])];
    if added == 0 && removed == 0 {
        out.push(Line::styled(
            crate::i18n::t("  （无改动）", "  (no changes)"),
            look.muted(),
        ));
        return out;
    }

    // 两栏行号各 4 列，加分隔共 12 列，再留出符号与空格 2 列
    let text_w = width.max(30).saturating_sub(14);
    let num = |i: Option<usize>| i.map_or("    ".to_string(), |n| format!("{:>4}", n + 1));
    let diff = TextDiff::from_lines(&fix.old, &fix.new);
    for (gi, group) in diff.grouped_ops(context).iter().enumerate() {
        if gi > 0 {
            out.push(Line::styled("     ⋯", look.muted()));
        }
        for op in group {
            for change in diff.iter_changes(op) {
                let (sign, style) = match change.tag() {
                    ChangeTag::Delete => ('-', look.error()),
                    ChangeTag::Insert => ('+', look.success()),
                    ChangeTag::Equal => (' ', look.muted()),
                };
                let value = change
                    .value()
                    .trim_end_matches(['\n', '\r'])
                    .replace('\t', "    ");
                out.push(Line::from(vec![
                    Span::styled(num(change.old_index()), look.muted()),
                    Span::raw(" "),
                    Span::styled(num(change.new_index()), look.muted()),
                    Span::raw(" "),
                    Span::styled("│", look.muted()),
                    Span::raw(" "),
                    Span::styled(format!("{sign} {}", clip(&value, text_w)), style),
                ]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::line_text;
    use console::measure_text_width;

    fn finding(id: &str, severity: Severity) -> Finding {
        Finding {
            id: id.into(),
            severity,
            title: "SQL 注入：用户输入被直接拼接进查询语句".into(),
            cwe: Some("CWE-89".into()),
            file: "src/db.py".into(),
            line: 42,
            description: "查询语句通过字符串格式化构造，攻击者可以注入任意 SQL。".into(),
            suggestion: "改用参数化查询。".into(),
        }
    }

    fn plain(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(line_text).collect()
    }

    #[test]
    fn card_shows_id_location_and_fits_width() {
        let lines = plain(&finding_card(
            &finding("VG-001", Severity::High),
            1,
            3,
            40,
            &Look::default(),
        ));
        assert!(lines[0].contains("[高危]") && lines[0].contains("VG-001"));
        assert!(lines[0].contains("(1/3)"));
        assert!(lines.iter().any(|l| l.contains("src/db.py:42 · CWE-89")));
        assert!(lines.iter().any(|l| l.contains("建议：改用参数化查询")));
        for l in &lines {
            assert!(measure_text_width(l) <= 40, "too wide: {l:?}");
        }
    }

    #[test]
    fn card_without_cwe_or_suggestion_is_still_valid() {
        let mut f = finding("VG-002", Severity::Low);
        f.cwe = None;
        f.suggestion.clear();
        let lines = plain(&finding_card(&f, 2, 2, 60, &Look::default()));
        assert!(lines.iter().any(|l| l.trim() == "src/db.py:42"));
        assert!(!lines.iter().any(|l| l.contains("建议")));
    }

    #[test]
    fn summary_lists_only_nonzero_levels() {
        let list = vec![
            finding("a", Severity::Critical),
            finding("b", Severity::Critical),
            finding("c", Severity::Low),
        ];
        let lines = plain(&summary_lines(&list, &Look::default()));
        assert!(lines[0].contains("共发现 3 个问题"));
        assert_eq!(lines.len(), 3);
        assert!(lines[1].contains("严重") && lines[1].ends_with(" 2"));
        assert!(lines[2].contains("低危") && lines[2].ends_with(" 1"));
        // 最多的一档占满 20 格，其余按比例（向上取整）
        assert_eq!(lines[1].matches('█').count(), 20);
        assert_eq!(lines[2].matches('█').count(), 10);
    }

    #[test]
    fn empty_summary_says_clean() {
        let lines = plain(&summary_lines(&[], &Look::default()));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("未发现安全问题"));
    }

    fn fix(old: &str, new: &str) -> Fix {
        Fix {
            finding_id: "VG-001".into(),
            file: "src/db.py".into(),
            old: old.into(),
            new: new.into(),
        }
    }

    #[test]
    fn diff_marks_changes_and_numbers_lines() {
        let lines = plain(&diff_lines(
            &fix("a\nb\nc\n", "a\nB\nc\n"),
            3,
            60,
            &Look::default(),
        ));
        assert!(
            lines[0].contains("src/db.py") && lines[0].contains("+1") && lines[0].contains("-1")
        );
        assert!(lines.iter().any(|l| l.contains("   2      │ - b")));
        assert!(lines.iter().any(|l| l.contains("        2 │ + B")));
        assert!(lines.iter().any(|l| l.contains("   1    1 │   a")));
    }

    #[test]
    fn diff_colors_removed_red_and_added_green() {
        let look = Look::default();
        let lines = diff_lines(&fix("a\n", "b\n"), 3, 60, &look);
        let color_of = |needle: &str| {
            lines
                .iter()
                .flat_map(|l| &l.spans)
                .find(|s| s.content.contains(needle))
                .and_then(|s| s.style.fg)
        };
        assert_eq!(color_of("- a"), Some(look.palette.error));
        assert_eq!(color_of("+ b"), Some(look.palette.success));
    }

    #[test]
    fn diff_collapses_distant_unchanged_regions() {
        let old: String = (1..=30).map(|i| format!("line{i}\n")).collect();
        let new = old
            .replace("line1\n", "LINE1\n")
            .replace("line30\n", "LINE30\n");
        let lines = plain(&diff_lines(&fix(&old, &new), 1, 60, &Look::default()));
        assert!(lines.iter().any(|l| l.contains('⋯')));
        assert!(!lines.iter().any(|l| l.contains("line15")));
    }

    #[test]
    fn identical_files_report_no_change() {
        let lines = plain(&diff_lines(&fix("x\n", "x\n"), 3, 60, &Look::default()));
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("无改动"));
    }

    #[test]
    fn diff_lines_fit_width_with_cjk_and_tabs() {
        let new = "\t这是一行非常非常非常非常非常非常非常非常非常长的中文注释内容\n";
        for l in plain(&diff_lines(&fix("x = 1\n", new), 3, 40, &Look::default())) {
            assert!(measure_text_width(&l) <= 40, "too wide: {l:?}");
        }
    }

    #[test]
    fn diff_stats_counts_lines() {
        assert_eq!(diff_stats("a\nb\n", "a\nc\nd\n"), (2, 1));
    }
}
