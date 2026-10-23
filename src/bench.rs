//! 标定集：拿一个「已知有几处漏洞」的样例仓库跑本地规则，算准召。
//!
//! 没有这个，所有「准确率提高了」都是感觉。改动规则之后跑一次
//! `cargo test bench`，漏报和误报都会当场列出来。
//!
//! 样例仓库在 `bench/fixtures`，期望值在 `bench/expected.tsv`。
//! 两个文件都是手工维护的——故意不从扫描结果生成，否则它永远等于自己。

use crate::finding::Finding;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 期望命中的 (清单编号, 文件, 行号)
fn expected() -> BTreeSet<(String, String, u32)> {
    let text =
        std::fs::read_to_string(root().join("bench/expected.tsv")).expect("bench/expected.tsv");
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut it = l.split('\t');
            let id = it.next().unwrap_or_default().trim().to_string();
            let file = it.next().unwrap_or_default().trim().to_string();
            let line = it
                .next()
                .unwrap_or_default()
                .trim()
                .parse()
                .unwrap_or_default();
            (id, file, line)
        })
        .collect()
}

/// `[CONF-01] 说明` → `CONF-01`
fn rule_id(f: &Finding) -> String {
    f.description
        .trim_start_matches('[')
        .split(']')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn actual() -> BTreeSet<(String, String, u32)> {
    let report = crate::scan::scan_local(
        &root().join("bench/fixtures"),
        &crate::vuln_checklist::VulnChecklist::new(),
    );
    report
        .findings
        .iter()
        .map(|f| (rule_id(f), f.file.clone(), f.line))
        .collect()
}

#[test]
fn local_rules_find_every_seeded_vulnerability_without_noise() {
    let want = expected();
    let got = actual();
    let hit: Vec<_> = want.intersection(&got).collect();
    let missed: Vec<_> = want.difference(&got).collect();
    let extra: Vec<_> = got.difference(&want).collect();

    let recall = hit.len() as f32 / want.len() as f32;
    let precision = if got.is_empty() {
        1.0
    } else {
        hit.len() as f32 / got.len() as f32
    };

    assert!(
        missed.is_empty(),
        "漏报（规则没命中）：{missed:?}；实际命中：{got:?}"
    );
    assert!(extra.is_empty(), "误报（命中了不该命中的）：{extra:?}");
    assert!(
        recall >= 0.95 && precision >= 0.95,
        "recall={recall:.2} precision={precision:.2}"
    );
}

#[test]
fn the_bench_covers_every_local_rule() {
    // 规则表加了新规则却没加样例，这里会红：覆盖率不能靠自觉
    let covered: BTreeSet<String> = expected().into_iter().map(|(id, _, _)| id).collect();
    let missing: Vec<&str> = crate::scan::covered_ids()
        .into_iter()
        .filter(|id| !covered.contains(*id))
        .collect();
    assert!(
        missing.is_empty(),
        "这些规则还没有标定样例：{missing:?}（在 bench/fixtures 里加一个，并更新 expected.tsv）"
    );
}
