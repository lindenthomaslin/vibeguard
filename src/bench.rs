//! 标定集：拿一个「已知有几处漏洞」的样例仓库跑本地规则，算准召。
//!
//! 没有这个，所有「准确率提高了」都是感觉。改动规则之后跑一次
//! `cargo test bench`，漏报和误报都会当场列出来。
//!
//! 样例仓库在 `bench/fixtures`，期望值在 `bench/expected.tsv`。
//! 两个文件都是手工维护的——故意不从扫描结果生成，否则它永远等于自己。

use crate::finding::Finding;
#[cfg(test)]
use std::collections::BTreeSet;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 期望命中的 (清单编号, 文件, 行号)
#[cfg(test)]
fn expected() -> BTreeSet<(String, String, u32)> {
    let text =
        std::fs::read_to_string(root().join("bench/expected.tsv")).expect("bench/expected.tsv");
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
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
#[cfg(test)]
fn rule_id(f: &Finding) -> String {
    f.description
        .trim_start_matches('[')
        .split(']')
        .next()
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
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

// ---------- 第二段：模型判定 ----------
//
// 本地规则那套标定证明不了模型层——20 条正则全中，也不代表那 300 多条
// 「需要读上下文」的判断是对的。这里另起一套：`bench/ai/fixtures` 里既埋了
// 真问题，也埋了「看着可疑但读上下文就该放过」的陷阱，两边都算分。
//
// 要花 token，所以不在 `cargo test` 里跑：`vibeguard bench`。

use crate::i18n::pick;
use anyhow::{Context, Result, bail};
use std::path::Path;

/// 一条标注：应当报出来的问题，或者一个不该报的陷阱
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bug,
    Trap,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub file: String,
    pub line: u32,
    pub kind: Kind,
    pub label: String,
    /// 对应的清单编号。检索标定要用它找到「这条会被塞进哪一批」
    pub id: Option<String>,
}

/// 读标注表，把 marker 注释换成真实行号。
///
/// 用 marker 而不是写死行号：改了样例代码不用回来改这张表，
/// 也就不会出现「期望值跟着扫描结果漂移」的自欺欺人。
pub fn load(dir: &Path) -> Result<Vec<Row>> {
    let path = dir.join("expected.tsv");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("读不到标注表：{}", path.display()))?;
    // 编辑器可能给文件加上 BOM，不去掉的话第一行会当成数据
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let fixtures = dir.join("fixtures");
    let mut rows = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() || raw.trim_start().starts_with('#') {
            continue;
        }
        let mut it = raw.split('\t');
        let file = it.next().unwrap_or_default().trim().to_string();
        let marker = it.next().unwrap_or_default().trim().to_string();
        let kind = match it.next().unwrap_or_default().trim() {
            "bug" => Kind::Bug,
            "trap" => Kind::Trap,
            other => bail!("第 {} 行：kind 只能是 bug / trap，收到 `{other}`", n + 1),
        };
        let label = it.next().unwrap_or_default().trim().to_string();
        let id = it
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "-")
            .map(str::to_string);
        let line =
            locate(&fixtures.join(&file), &marker).with_context(|| format!("第 {} 行", n + 1))?;
        rows.push(Row {
            file,
            line,
            kind,
            label,
            id,
        });
    }
    if rows.is_empty() {
        bail!("标注表是空的：{}", path.display());
    }
    Ok(rows)
}

/// marker 在文件里必须**恰好**出现一次：出现两次说明标注指代不明，
/// 一次都没有说明代码改了但表没跟着改
fn locate(path: &Path, marker: &str) -> Result<u32> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("读不到样例：{}", path.display()))?;
    let hits: Vec<u32> = text
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains(marker))
        .map(|(i, _)| i as u32 + 1)
        .collect();
    match hits.len() {
        1 => Ok(hits[0]),
        0 => bail!("{} 里找不到标记 `{marker}`", path.display()),
        n => bail!(
            "{} 里 `{marker}` 出现了 {n} 次，标注指代不明",
            path.display()
        ),
    }
}

#[derive(Debug, Default)]
pub struct Score {
    /// 报对了的（每条标注只算一次）
    pub tp: usize,
    /// 没报出来的
    pub missed: Vec<String>,
    /// 报在「真问题所在的那一行」上，但换了另一个清单编号。
    /// 单独计数：它不是瞎报，是同一处多报了几条——噪音，但不是假话
    pub duplicates: Vec<String>,
    /// 完全对不上任何标注的
    pub fp: usize,
    /// 其中踩在「看着可疑但其实安全」的陷阱上的
    pub trap_hits: usize,
    pub extra: Vec<String>,
}

impl Score {
    pub fn precision(&self) -> f32 {
        if self.tp + self.fp == 0 {
            0.0
        } else {
            self.tp as f32 / (self.tp + self.fp) as f32
        }
    }

    pub fn recall(&self) -> f32 {
        let total = self.tp + self.missed.len();
        if total == 0 {
            0.0
        } else {
            self.tp as f32 / total as f32
        }
    }

    /// 每找到一个真问题，平均多报几条（含重复和误报）
    pub fn noise(&self) -> f32 {
        if self.tp == 0 {
            0.0
        } else {
            (self.duplicates.len() + self.fp) as f32 / self.tp as f32
        }
    }
}

fn same_file(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim_start_matches("./").replace('\\', "/");
    let (a, b) = (norm(a), norm(b));
    a == b || a.ends_with(&format!("/{b}")) || b.ends_with(&format!("/{a}"))
}

fn label_of(f: &Finding) -> String {
    let id = f
        .description
        .trim_start_matches('[')
        .split(']')
        .next()
        .unwrap_or_default();
    format!("{}:{} [{id}] {}", f.file, f.line, f.title)
}

/// 行号允许一点偏移：模型引用的是它读到的那一行，差两三行很常见，
/// 差得更多基本就是报错地方了
pub fn score(findings: &[Finding], rows: &[Row], tol: u32) -> Score {
    let bugs: Vec<&Row> = rows.iter().filter(|r| r.kind == Kind::Bug).collect();
    let traps: Vec<&Row> = rows.iter().filter(|r| r.kind == Kind::Trap).collect();
    let mut used = vec![false; bugs.len()];
    let mut duplicates = Vec::new();
    let mut extra = Vec::new();
    let mut trap_hits = 0;

    for f in findings {
        let near = |b: &&Row| same_file(&b.file, &f.file) && b.line.abs_diff(f.line) <= tol;
        // 优先认领还没被认领的标注
        let best = bugs
            .iter()
            .enumerate()
            .filter(|(i, b)| !used[*i] && near(b))
            .map(|(i, b)| (i, b.line.abs_diff(f.line)))
            .min_by_key(|(_, d)| *d);
        if let Some((i, _)) = best {
            used[i] = true;
            continue;
        }
        // 落在某处真问题旁边、但那处已经被认领了：同一处多报了一条
        if bugs.iter().any(near) {
            duplicates.push(label_of(f));
            continue;
        }
        if traps.iter().any(near) {
            trap_hits += 1;
        }
        extra.push(label_of(f));
    }

    let missed = bugs
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(b, _)| format!("{}:{} {}", b.file, b.line, b.label))
        .collect();

    Score {
        tp: used.iter().filter(|u| **u).count(),
        missed,
        duplicates,
        fp: extra.len(),
        trap_hits,
        extra,
    }
}

pub struct Run {
    pub score: Score,
    pub bugs: usize,
    pub traps: usize,
    pub tokens: u64,
    pub failed: Vec<String>,
}

/// 一条命令干完：跑（或读存档）、打分、打印
pub async fn run(
    dir: &Path,
    config: &crate::config::Config,
    save: Option<&Path>,
    score_file: Option<&Path>,
) -> Result<()> {
    let dir = if dir.is_dir() {
        dir.to_path_buf()
    } else {
        root().join(dir)
    };
    let rows = load(&dir)?;
    let (findings, tokens, failed) = match score_file {
        // 离线重打分：改了口径想重算，不该再花一遍 token
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("读不到结果文件：{}", path.display()))?;
            let findings: Vec<Finding> = serde_json::from_str(&text)
                .with_context(|| format!("不是扫描结果的 JSON：{}", path.display()))?;
            (findings, 0, Vec::new())
        }
        None => run_ai(&dir, config).await?,
    };
    if let Some(path) = save {
        let json = serde_json::to_string_pretty(&findings)?;
        std::fs::write(path, json)
            .with_context(|| format!("写不进结果文件：{}", path.display()))?;
        println!(
            "{}",
            pick(
                format!("结果已存到 {}（下次可以 --score 它重打分）", path.display()),
                format!(
                    "Saved to {} (use --score to re-score it later)",
                    path.display()
                ),
            )
        );
    }
    let run = Run {
        score: score(&findings, &rows, 3),
        bugs: rows.iter().filter(|r| r.kind == Kind::Bug).count(),
        traps: rows.iter().filter(|r| r.kind == Kind::Trap).count(),
        tokens,
        failed,
    };
    report(&run);
    Ok(())
}

/// 真跑一遍：本地规则 + 模型判定，然后和标注表对分
pub async fn run_ai(
    dir: &Path,
    config: &crate::config::Config,
) -> Result<(Vec<Finding>, u64, Vec<String>)> {
    let dir = if dir.is_dir() {
        dir.to_path_buf()
    } else {
        root().join(dir)
    };
    let fixtures = dir.join("fixtures");
    if !fixtures.is_dir() {
        bail!("样例仓库不存在：{}", fixtures.display());
    }
    let checklist = crate::vuln_checklist::VulnChecklist::new();
    let index = crate::scan_ai::Index::build(&fixtures);
    let batches = crate::scan_ai::batches_with(
        &checklist,
        &crate::scan::covered_ids(),
        crate::scan_ai::BATCH,
        crate::scan_ai::SECURITY_CATEGORIES,
    );
    let client = std::sync::Arc::new(crate::llm::LlmClient::new(config)?);
    let judged = crate::scan_ai::judge(client, std::sync::Arc::new(index), batches, |_| {}).await;
    let merged = crate::scan_ai::to_findings(&fixtures, &judged.judgements, &checklist, 1);
    Ok((merged.findings, judged.tokens, judged.failed))
}

/// 打印结果。给人看的，所以走和界面一样的语言开关
pub fn report(run: &Run) {
    let s = &run.score;
    // 先说结果完不完整：跑挂了几批的时候，下面的百分比没有意义
    if !run.failed.is_empty() {
        println!(
            "{}",
            pick(
                format!(
                    "⚠ 有 {} 批没跑成功，下面的数字不完整（修好额度/网络再跑）：",
                    run.failed.len()
                ),
                format!(
                    "⚠ {} batches failed, the numbers below are incomplete (fix quota/network and rerun):",
                    run.failed.len()
                ),
            )
        );
        for f in &run.failed {
            println!("  - {f}");
        }
        println!();
    }
    println!(
        "{}",
        pick(
            format!("标注集：{} 条应当报出 · {} 条陷阱", run.bugs, run.traps),
            format!("Labels: {} to find · {} traps", run.bugs, run.traps),
        )
    );
    println!(
        "{}",
        pick(
            format!(
                "命中 {} · 漏报 {} · 同址重复 {} · 误报 {}（其中 {} 条踩在陷阱上）",
                s.tp,
                s.missed.len(),
                s.duplicates.len(),
                s.fp,
                s.trap_hits
            ),
            format!(
                "hit {} · missed {} · duplicate reports {} · false positives {} ({} on traps)",
                s.tp,
                s.missed.len(),
                s.duplicates.len(),
                s.fp,
                s.trap_hits
            ),
        )
    );
    println!(
        "{}",
        pick(
            format!(
                "精确率 {:.1}% · 召回率 {:.1}% · 每个真问题多报 {:.1} 条",
                s.precision() * 100.0,
                s.recall() * 100.0,
                s.noise()
            ),
            format!(
                "precision {:.1}% · recall {:.1}% · {:.1} extra reports per bug",
                s.precision() * 100.0,
                s.recall() * 100.0,
                s.noise()
            ),
        )
    );
    if run.tokens > 0 {
        println!(
            "{}",
            pick(
                format!("消耗 {} tokens", run.tokens),
                format!("{} tokens used", run.tokens)
            )
        );
    }
    if !s.missed.is_empty() {
        println!("{}", pick("漏报：".to_string(), "Missed:".to_string()));
        for m in &s.missed {
            println!("  - {m}");
        }
    }
    if !s.extra.is_empty() {
        println!(
            "{}",
            pick("误报：".to_string(), "False positives:".to_string())
        );
        for e in &s.extra {
            println!("  - {e}");
        }
    }
}

// ---------- 第三段：检索 ----------
//
// 前两段量的都是「模型看到代码之后判得准不准」。真实仓库上还有一道更早的闸：
// 模型根本看不到全部代码，只能看到检索挑出来的那几十个文件。挑错了，后面再准也没用。
//
// 这一段是纯函数：不调模型、不花钱、能在 CI 里跑。

pub struct Retrieval {
    pub hit: usize,
    pub total: usize,
    /// 没被检索到的标注（文件:行 + 说明）
    pub misses: Vec<String>,
    /// 平均每批塞进上下文的文件数，用来看预算有没有被浪费
    pub files_per_batch: f32,
}

impl Retrieval {
    pub fn recall(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.hit as f32 / self.total as f32
        }
    }
}

fn same_name(a: &str, b: &str) -> bool {
    std::path::Path::new(a)
        .file_name()
        .map(|n| n.to_string_lossy() == b)
        .unwrap_or(false)
}

/// 每条标注的文件，有没有被它所在的那一批检索出来
pub fn retrieval_recall(root: &Path, rows: &[Row], budget: usize) -> Retrieval {
    use std::collections::HashMap;

    let checklist = crate::vuln_checklist::VulnChecklist::new();
    let index = crate::scan_ai::Index::build(root);
    let batches = crate::scan_ai::batches_with(
        &checklist,
        &crate::scan::covered_ids(),
        crate::scan_ai::BATCH,
        crate::scan_ai::SECURITY_CATEGORIES,
    );

    // 每批检索一次就够：同一批里十几条标注共用同一份上下文
    let mut cache: HashMap<usize, Vec<String>> = HashMap::new();
    let mut hit = 0;
    let mut total = 0;
    let mut misses = Vec::new();

    for row in rows.iter().filter(|r| r.kind == Kind::Bug) {
        let Some(id) = &row.id else { continue };
        let Some(bi) = batches
            .iter()
            .position(|b| b.items.iter().any(|i| &i.id == id))
        else {
            continue;
        };
        let files = cache.entry(bi).or_insert_with(|| {
            index
                .retrieve(&batches[bi], budget)
                .into_iter()
                .map(|(p, _)| p)
                .collect()
        });
        total += 1;
        if files.iter().any(|f| same_name(f, &row.file)) {
            hit += 1;
        } else {
            misses.push(format!("{}:{} {}", row.file, row.line, row.label));
        }
    }

    let files_per_batch = if cache.is_empty() {
        0.0
    } else {
        cache.values().map(Vec::len).sum::<usize>() as f32 / cache.len() as f32
    };

    Retrieval {
        hit,
        total,
        misses,
        files_per_batch,
    }
}

/// 往目录里铺一批干扰文件：名字和内容都沾安全词汇，
/// 逼检索真的分辨，而不是靠「全仓库只有它提到 auth」蒙对。
pub fn fill_with_distractors(dir: &Path, n: usize) -> Result<()> {
    const NOUNS: &[&str] = &[
        "user", "session", "invoice", "project", "order", "coupon", "token", "upload", "report",
        "webhook", "audit", "billing", "tenant", "apikey", "job", "mail", "search", "cache",
    ];
    const VERBS: &[&str] = &[
        "load", "save", "fetch", "verify", "render", "dispatch", "sync", "flush", "attach",
        "resolve",
    ];
    for i in 0..n {
        let noun = NOUNS[i % NOUNS.len()];
        let verb = VERBS[(i / NOUNS.len()) % VERBS.len()];
        let module = format!("{noun}_{verb}_{i:03}");
        let body = format!(
            "\"\"\"{noun} {verb} helpers.\"\"\"\n\
             \n\
             from models import {noun_title}\n\
             from client import api\n\
             \n\
             \n\
             def {verb}_{noun}(request, **kwargs):\n\
             \x20   \"\"\"Handle {noun} {verb}; called from the API layer.\"\"\"\n\
             \x20   item = {noun_title}.query.get(request.args.get(\"id\"))\n\
             \x20   if item is None:\n\
             \x20       return None\n\
             \x20   data = item.as_dict()\n\
             \x20   for key, value in kwargs.items():\n\
             \x20       data[key] = value\n\
             \x20   api.session.commit()\n\
             \x20   return data\n\
             \n\
             \n\
             def {verb}_all_{noun}():\n\
             \x20   return [{noun_title}.query.all()]\n",
            noun_title = format!("{}{}", noun[..1].to_uppercase(), &noun[1..]),
        );
        std::fs::write(dir.join(format!("{module}.py")), body)
            .with_context(|| format!("写不进干扰文件 {module}.py"))?;
    }
    Ok(())
}

/// 跑一遍检索标定并打印。不花 token，所以也给命令行开了一个入口，
/// 免得「检索准不准」只能靠 `cargo test -- --nocapture` 看
pub fn report_retrieval(dir: &Path, distractors: usize) -> Result<()> {
    let dir = if dir.is_dir() {
        dir.to_path_buf()
    } else {
        root().join(dir)
    };
    let rows = load(&dir)?;
    let staged = std::env::temp_dir().join(format!("vibeguard-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staged);
    staged_repo(&staged, distractors)?;
    let r = retrieval_recall(&staged, &rows, crate::scan_ai::RETRIEVE_BUDGET);
    let _ = std::fs::remove_dir_all(&staged);

    println!(
        "{}",
        pick(
            format!(
                "检索标定：{} 个文件（含 {} 个干扰），每批塞进上下文 {:.0} 个",
                distractors + 6,
                distractors,
                r.files_per_batch
            ),
            format!(
                "Retrieval: {} files ({} distractors), {:.0} per batch",
                distractors + 6,
                distractors,
                r.files_per_batch
            ),
        )
    );
    println!(
        "{}",
        pick(
            format!("召回 {}/{} = {:.0}%", r.hit, r.total, r.recall() * 100.0),
            format!("recall {}/{} = {:.0}%", r.hit, r.total, r.recall() * 100.0),
        )
    );
    if !r.misses.is_empty() {
        println!(
            "{}",
            pick("漏掉的标注：".to_string(), "Missed:".to_string())
        );
        for m in &r.misses {
            println!("  - {m}");
        }
    }
    Ok(())
}

/// 把样例仓库复制到 `dir`，再铺一批干扰文件。
/// 标定要在「真实尺寸」的仓库上跑，但仓库本身不该进版本库——
/// 干扰文件是生成的，样例是复制的，每次跑都从零开始。
pub fn staged_repo(dir: &Path, distractors: usize) -> Result<()> {
    let src = root().join("bench/ai/fixtures");
    std::fs::create_dir_all(dir)?;
    for entry in std::fs::read_dir(&src)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && let Some(name) = path.file_name()
        {
            std::fs::copy(&path, dir.join(name))?;
        }
    }
    fill_with_distractors(dir, distractors)?;
    Ok(())
}

#[cfg(test)]
mod ai_bench_tests {
    use super::*;
    use crate::finding::Severity;

    fn finding(file: &str, line: u32) -> Finding {
        Finding {
            id: "VG-001".into(),
            severity: Severity::High,
            title: "x".into(),
            cwe: None,
            file: file.into(),
            line,
            description: "[AUTH-01] x".into(),
            suggestion: String::new(),
        }
    }

    fn row(file: &str, line: u32, kind: Kind) -> Row {
        Row {
            file: file.into(),
            line,
            kind,
            label: "x".into(),
            id: None,
        }
    }

    /// 标注表自己也要能被解析：marker 不存在、出现两次、kind 写错，都会在这里红
    #[test]
    fn the_label_file_is_well_formed() {
        let rows = load(&root().join("bench/ai")).expect("bench/ai/expected.tsv");
        let bugs = rows.iter().filter(|r| r.kind == Kind::Bug).count();
        let traps = rows.iter().filter(|r| r.kind == Kind::Trap).count();
        assert!(bugs >= 10, "应当报出的标注太少：{bugs}");
        assert!(traps >= 5, "陷阱太少，误报就测不出来：{traps}");
        // marker 指向的行必须真的存在
        for r in &rows {
            assert!(r.line > 0, "{:?}", r);
        }
    }

    #[test]
    fn scoring_counts_hits_misses_and_trap_hits() {
        let rows = [
            row("a.py", 10, Kind::Bug),
            row("a.py", 40, Kind::Bug),
            row("b.py", 20, Kind::Trap),
        ];
        // 第一处差两行（容忍），第二处没报，陷阱上被误报一次
        let findings = [finding("a.py", 12), finding("b.py", 21)];
        let s = score(&findings, &rows, 3);
        assert_eq!(s.tp, 1);
        assert_eq!(s.missed.len(), 1);
        assert_eq!(s.fp, 1);
        assert_eq!(s.trap_hits, 1);
        assert!((s.precision() - 0.5).abs() < 1e-6);
        assert!((s.recall() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_second_report_on_the_same_line_is_a_duplicate_not_a_false_positive() {
        let rows = [row("a.py", 10, Kind::Bug)];
        let s = score(&[finding("a.py", 10), finding("a.py", 11)], &rows, 3);
        assert_eq!(s.tp, 1);
        assert_eq!(s.duplicates.len(), 1);
        assert_eq!(s.fp, 0, "同一处多报一条不算瞎报");
        assert!((s.noise() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_finding_far_from_the_label_does_not_count() {
        let rows = [row("a.py", 10, Kind::Bug)];
        let s = score(&[finding("a.py", 30)], &rows, 3);
        assert_eq!(s.tp, 0);
        assert_eq!(s.fp, 1);
        assert_eq!(s.missed.len(), 1);
    }

    #[test]
    fn one_finding_cannot_claim_two_labels() {
        let rows = [row("a.py", 10, Kind::Bug), row("a.py", 11, Kind::Bug)];
        let s = score(&[finding("a.py", 10)], &rows, 3);
        assert_eq!(s.tp, 1, "一条发现只能算命中一处");
        assert_eq!(s.missed.len(), 1);
    }

    #[test]
    fn a_leading_dot_slash_still_matches() {
        let rows = [row("a.py", 10, Kind::Bug)];
        let s = score(&[finding("./a.py", 10)], &rows, 3);
        assert_eq!(s.tp, 1);
    }
    /// 检索是模型之前的那道闸：文件没被挑出来，后面判得再准也没用。
    /// 小仓库里必然全中（六个文件全塞得下），所以真正要看的是加了干扰之后。
    #[test]
    fn retrieval_finds_the_vulnerable_files_in_a_real_sized_repo() {
        let rows = load(&root().join("bench/ai")).expect("标注表");
        let budget = crate::scan_ai::RETRIEVE_BUDGET;

        let small = retrieval_recall(&root().join("bench/ai/fixtures"), &rows, budget);
        assert_eq!(
            small.hit, small.total,
            "小仓库都不全中，说明标注和批次的对应写错了：{:?}",
            small.misses
        );

        let dir = tempfile::tempdir().unwrap();
        staged_repo(dir.path(), 300).unwrap();
        let big = retrieval_recall(dir.path(), &rows, budget);
        println!(
            "检索召回 {}/{} = {:.0}% · 每批塞进上下文 {:.0} 个文件",
            big.hit,
            big.total,
            big.recall() * 100.0,
            big.files_per_batch
        );
        for m in &big.misses {
            println!("  漏：{m}");
        }
        // 纯函数、无模型、无随机：这里就该是满分。掉一条说明检索退化了
        assert_eq!(
            big.hit,
            big.total,
            "306 个文件时检索漏了 {} 条：{:?}",
            big.total - big.hit,
            big.misses
        );
        // 预算没被浪费：每批挑出来的文件数应该远小于仓库规模
        assert!(
            big.files_per_batch < 20.0,
            "每批塞了 {:.0} 个文件，上下文预算被撑爆了",
            big.files_per_batch
        );
    }
}
