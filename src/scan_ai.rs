//! 扫描的第二段：把本地规则判不了的清单条目交给模型逐条判断。
//!
//! 和第一段的分工很清楚：
//! - 本地规则负责**证据确凿**的东西（字符串里有个密钥、代码里写了 `shell=True`）；
//! - 这一段负责**需要读上下文**的东西（这条查询带没带 owner 条件？这个路由挂没挂鉴权？）。
//!
//! 关键约束：**每条都必须给出 `文件:行号` 或「缺什么信息」**。允许模型说
//! 「无法判断」，不允许它说「看着没问题」——后者是最危险的一种输出，因为它
//! 长得像结论。

use crate::i18n::{pick, tr};
use crate::vuln_checklist::{VulnChecklist, VulnItem};
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use serde::Deserialize;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

/// AI 判定默认只跑这些分类：A~M、P、R 都是「能被利用」的东西。
///
/// 跳过的是 N（依赖与供应链）、O（云与基础设施）、Q（日志与监控）、S（代码质量）——
/// 这四类大多是工程规范（没有日志、没有迁移脚本、没有探针），让模型读代码判断
/// 只会得到一堆「存在」，把真正的漏洞淹掉。它们更适合交给专门的工具。
pub const SECURITY_CATEGORIES: &[char] = &[
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'P', 'R',
];

/// 分类字符串是「A. 认证与账号」，取首字母
pub fn category_letter(category: &str) -> char {
    category.chars().next().unwrap_or('?')
}

/// 这个分类会不会进 AI 判定
pub fn is_security_category(category: &str) -> bool {
    SECURITY_CATEGORIES.contains(&category_letter(category))
}

/// 一次喂给模型多少条。太多会漏条目、输出被截断；太少则请求次数爆炸。
/// 60 条大约 8 次请求，是个折中：再小就变成 20 次，一次真模型调用十几秒，太慢。
pub const BATCH: usize = 60;

/// 增量扫描时一批能装更多：摘要里只剩变过的几个文件，输入小了一个量级，
/// 输出（每条一行判定）成了瓶颈。130 条大约 4k tokens，留了余量。
pub const BATCH_INCREMENTAL: usize = 130;
/// 文件树最多列多少行
const TREE_LINES: usize = 300;

/// 模型的判定
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// 看到了确实有问题的代码
    Exists,
    /// 像是有，但证据不够硬
    Suspected,
    /// 看了相关代码，没有这个问题
    NotFound,
    /// 静态看不出来，得跑起来验证
    Undetermined,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Exists => "存在",
            Verdict::Suspected => tr("疑似"),
            Verdict::NotFound => tr("不存在"),
            Verdict::Undetermined => tr("无法判断"),
        }
    }

    fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "exists" | "exist" | "存在" | "present" => Some(Verdict::Exists),
            "suspected" | "suspect" | "疑似" => Some(Verdict::Suspected),
            "not_found" | "notfound" | "absent" | "不存在" => Some(Verdict::NotFound),
            "undetermined" | "unknown" | "无法判断" => Some(Verdict::Undetermined),
            _ => None,
        }
    }
}

/// 一次判定所基于的仓库快照指纹，存档时要带上
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanDigest {
    pub hash: u64,
    pub files: std::collections::BTreeMap<String, u64>,
}

/// 一条判定结果
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Judgement {
    pub id: String,
    pub verdict: Verdict,
    /// `文件:行号`，或者「缺什么信息」
    pub evidence: String,
    pub reason: String,
}

/// 一批要判的条目
pub struct Batch {
    /// 这一批覆盖的分类，用来显示进度
    pub label: String,
    pub items: Vec<VulnItem>,
}

impl Batch {
    /// 对半拆开。输出被截断时用它重试——比猜「max_tokens 该给多少」可靠。
    pub fn split(self) -> Option<(Batch, Batch)> {
        if self.items.len() < 2 {
            return None;
        }
        let mid = self.items.len() / 2;
        let mut items = self.items;
        let right = items.split_off(mid);
        Some((
            Batch {
                label: pick(
                    format!("{}（前半）", self.label),
                    format!("{} (first half)", self.label),
                ),
                items,
            },
            Batch {
                label: pick(
                    format!("{}（后半）", self.label),
                    format!("{} (second half)", self.label),
                ),
                items: right,
            },
        ))
    }
}

/// 按分类顺序把条目装进若干批，每批不超过 `BATCH` 条。
/// 跨分类装批是为了少发请求——真模型一次要十几秒，按 19 个分类发就是五分钟。
/// 指定批大小的版本：增量扫描时摘要小，可以一次判更多条
pub fn batches_with(
    checklist: &VulnChecklist,
    skip: &[&str],
    size: usize,
    only: &[char],
) -> Vec<Batch> {
    let size = size.max(1);
    let mut out: Vec<Batch> = Vec::new();
    let mut current: Vec<VulnItem> = Vec::new();
    let mut categories: Vec<String> = Vec::new();

    let flush = |items: &mut Vec<VulnItem>, cats: &mut Vec<String>, out: &mut Vec<Batch>| {
        if items.is_empty() {
            return;
        }
        let label = if cats.len() == 1 {
            cats[0].clone()
        } else {
            pick(
                format!("{} 等 {} 类", cats[0], cats.len()),
                format!("{} and {} more categories", cats[0], cats.len()),
            )
        };
        out.push(Batch {
            label,
            items: std::mem::take(items),
        });
        cats.clear();
    };

    for cat in checklist.categories() {
        if !only.is_empty() && !only.contains(&category_letter(cat)) {
            continue;
        }
        for item in checklist
            .by_category(cat)
            .into_iter()
            .filter(|i| !skip.contains(&i.id.as_str()))
        {
            if current.len() >= size {
                flush(&mut current, &mut categories, &mut out);
            }
            if categories.last().map(String::as_str) != Some(cat) {
                categories.push(cat.to_string());
            }
            current.push(item.clone());
        }
    }
    flush(&mut current, &mut categories, &mut out);
    out
}

/// 名字里带这些词的文件优先塞进摘要——安全问题和它们最相关
const HOT_WORDS: &[&str] = &[
    "auth",
    "login",
    "signin",
    "session",
    "token",
    "jwt",
    "user",
    "admin",
    "permission",
    "role",
    "acl",
    "api",
    "route",
    "handler",
    "controller",
    "view",
    "middleware",
    "config",
    "setting",
    "db",
    "sql",
    "model",
    "query",
    "upload",
    "file",
    "exec",
    "cmd",
    "shell",
    "crypto",
    "secret",
    "key",
    "docker",
    "compose",
    "deploy",
    "nginx",
    "server",
];

const CODE_EXTS: &[&str] = &[
    "rs",
    "py",
    "js",
    "jsx",
    "ts",
    "tsx",
    "go",
    "java",
    "kt",
    "rb",
    "php",
    "cs",
    "swift",
    "c",
    "cc",
    "cpp",
    "h",
    "hpp",
    "sql",
    "vue",
    "svelte",
    "yml",
    "yaml",
    "toml",
    "env",
    "tf",
    "Dockerfile",
];

fn hash_of(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn collect_paths(root: &Path) -> Vec<(String, bool)> {
    let mut paths: Vec<(String, bool)> = Vec::new(); // (相对路径, 是否热点)
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .git_ignore(true)
        .git_exclude(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                "node_modules"
                    | "target"
                    | "dist"
                    | "build"
                    | ".git"
                    | "vendor"
                    | ".venv"
                    | "__pycache__"
                    | "coverage"
                    | "assets"
            )
        })
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let is_code = CODE_EXTS.iter().any(|e| e.eq_ignore_ascii_case(&ext))
            || name.starts_with("Dockerfile")
            || name.starts_with(".env");
        if !is_code {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let lower = rel.to_ascii_lowercase();
        let hot = HOT_WORDS.iter().any(|w| lower.contains(w));
        paths.push((rel, hot));
    }

    // 热点文件排前面，同等条件下路径短的优先（根目录的入口文件更重要）
    paths.sort_by_key(|(p, hot)| (!*hot, p.matches('/').count(), p.len()));
    paths
}

fn tree_of(paths: &[(String, bool)]) -> String {
    paths
        .iter()
        .take(TREE_LINES)
        .map(|(p, _)| p.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 一个可检索的源文件
struct Doc {
    path: String,
    body: String,
    /// 小写副本，只用来打分；展示时用 `body`
    lower: String,
    hash: u64,
}

/// 仓库索引：文件树 + 每个文件的正文。
///
/// 上一版是「把一份 40KB 的摘要丢给所有批次」——390 条清单条目抢这 40KB，
/// 平均每条只有一百来字节的上下文，模型只能靠常识猜。现在改成按批检索：
/// 用这一批条目的关键词去仓库里找相关文件，命中多少给多少。
pub struct Index {
    pub tree: String,
    docs: Vec<Doc>,
    /// 太大或读不出来、没进索引的文件数
    pub skipped: usize,
}

/// 单个文件进索引的上限；再大就不是「读代码」，是「灌数据」
const INDEX_FILE_BUDGET: usize = 200_000;
/// 整个索引的上限：内存里同时放正文和小写副本，别把大仓库撑爆
const INDEX_TOTAL_BUDGET: usize = 8_000_000;
/// 一批最多附多少代码
const RETRIEVE_BUDGET: usize = 16_000;
/// 单个文件最多摘多少：给的是「命中位置前后几十行」，不是整个文件，
/// 所以可以多带几个文件，而不是让一个 10KB 的文件吃掉全部预算
const SNIPPET_BUDGET: usize = 3_000;

impl Index {
    pub fn build(root: &Path) -> Index {
        let paths = collect_paths(root);
        let tree = tree_of(&paths);
        let mut docs = Vec::new();
        let mut skipped = 0usize;
        let mut used = 0usize;
        for (rel, _) in &paths {
            let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
                skipped += 1;
                continue;
            };
            if text.len() > INDEX_FILE_BUDGET || used + text.len() > INDEX_TOTAL_BUDGET {
                skipped += 1;
                continue;
            }
            used += text.len();
            docs.push(Doc {
                path: rel.clone(),
                hash: hash_of(&text),
                lower: text.to_lowercase(),
                body: text,
            });
        }
        Index {
            tree,
            docs,
            skipped,
        }
    }

    /// 整个仓库的指纹：文件树 + 每个文件的内容
    pub fn hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.tree.hash(&mut h);
        for d in &self.docs {
            d.path.hash(&mut h);
            d.hash.hash(&mut h);
        }
        h.finish()
    }

    /// 参与索引的每个文件的内容哈希
    pub fn file_hashes(&self) -> std::collections::BTreeMap<String, u64> {
        self.docs.iter().map(|d| (d.path.clone(), d.hash)).collect()
    }

    /// 只保留指定文件的视图（增量扫描：只看变过的）
    pub fn scoped(&self, changed: &[String]) -> Index {
        Index {
            tree: self.tree.clone(),
            docs: self
                .docs
                .iter()
                .filter(|d| changed.iter().any(|c| c == &d.path))
                .map(|d| Doc {
                    path: d.path.clone(),
                    body: d.body.clone(),
                    lower: d.lower.clone(),
                    hash: d.hash,
                })
                .collect(),
            skipped: self.skipped,
        }
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// 按这一批条目的关键词挑文件。
    ///
    /// **每条条目各自检索，再取并集**。一开始是「把整批 60 条的关键词混在一起
    /// 打分」，结果注入类那一批挑出了 Cargo.toml——因为几百个词互相稀释，
    /// 谁命中一两个都能得分。按条目分开之后，每个文件至少要为某一条负责。
    pub fn retrieve(&self, batch: &Batch, budget: usize) -> Vec<(String, String)> {
        if self.docs.is_empty() {
            return Vec::new();
        }
        let total = self.docs.len();
        // 文档频率算一次，所有条目共用
        let all = keywords(batch);
        let df: std::collections::HashMap<&str, usize> = all
            .iter()
            .map(|k| {
                let n = self
                    .docs
                    .iter()
                    .filter(|d| d.lower.contains(k.as_str()))
                    .count();
                (k.as_str(), n)
            })
            .collect();

        // 文件下标 → (最好分数, 让它命中的关键词)
        let mut best: std::collections::HashMap<usize, (f32, Vec<String>)> = Default::default();
        for item in &batch.items {
            let keys = item_keywords(item);
            if keys.is_empty() {
                continue;
            }
            let mut scored: Vec<(f32, usize)> = self
                .docs
                .iter()
                .enumerate()
                .filter_map(|(i, d)| {
                    let path_lower = d.path.to_lowercase();
                    let mut score = 0.0f32;
                    for k in &keys {
                        // 没出现过，或者超过 60% 的文件都有（太常见，没区分度）
                        let n = *df.get(k.as_str()).unwrap_or(&0);
                        if n == 0 || n * 5 > total * 3 {
                            continue;
                        }
                        if d.lower.contains(k.as_str()) {
                            score += 1.0 / (1.0 + (n as f32).ln());
                            if path_lower.contains(k.as_str()) {
                                score += 0.5;
                            }
                        }
                    }
                    // 按文件长度开方归一：不归一的话，最大的文件总是赢，
                    // 因为它「碰巧」包含更多词
                    let norm = (d.body.len() as f32).sqrt().max(1.0);
                    let score = score / norm;
                    (score > 0.0).then_some((score, i))
                })
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            // 每条只取最相关的 3 个文件，避免一条条目霸占预算
            for (score, i) in scored.into_iter().take(3) {
                let e = best.entry(i).or_insert((0.0, Vec::new()));
                if score > e.0 {
                    *e = (score, keys.clone());
                }
            }
        }

        let mut ranked: Vec<(f32, &Doc, Vec<String>)> = best
            .into_iter()
            .map(|(i, (s, keys))| (s, &self.docs[i], keys))
            .collect();
        ranked.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.path.len().cmp(&b.1.path.len()))
                .then_with(|| a.1.path.cmp(&b.1.path))
        });

        let mut out = Vec::new();
        let mut used = 0usize;
        for (_, doc, keys) in &ranked {
            let piece = excerpt(doc, keys, SNIPPET_BUDGET);
            if used + piece.len() > budget {
                continue;
            }
            used += piece.len();
            out.push((doc.path.clone(), piece));
        }

        // 一个词都没命中（条目全是中文描述时会发生）：退回按顺序挑几个，
        // 总比让模型对着空上下文瞎猜强
        if out.is_empty() {
            for doc in self.docs.iter().take(4) {
                let piece = clip(&doc.body, SNIPPET_BUDGET);
                if used + piece.len() > budget {
                    break;
                }
                used += piece.len();
                out.push((doc.path.clone(), piece));
            }
        }
        out
    }
}

/// 摘出命中位置附近的代码，**每行带行号**。
///
/// 带行号是有意的：提示词要求模型给出 `文件:行号`，不给行号它只能数，
/// 数错的概率很高——而那正是我们刚刚开始核实的东西。
fn excerpt(doc: &Doc, keys: &[String], max: usize) -> String {
    let lines: Vec<&str> = doc.body.lines().collect();
    let hit = lines.iter().position(|l| {
        let low = l.to_lowercase();
        keys.iter().any(|k| low.contains(k.as_str()))
    });
    let Some(i) = hit else {
        return clip(&doc.body, max);
    };
    let start = i.saturating_sub(20);
    let end = (i + 40).min(lines.len());
    let mut out = String::new();
    if start > 0 {
        out.push_str(&pick(
            format!("…（前 {start} 行省略）\n"),
            format!("… (first {start} lines omitted)\n"),
        ));
    }
    for (n, line) in lines[start..end].iter().enumerate() {
        out.push_str(&format!("{:>5}  {line}\n", start + n + 1));
    }
    if end < lines.len() {
        out.push_str(&pick(
            format!("…（后面还有 {} 行）\n", lines.len() - end),
            format!("… ({} more lines below)\n", lines.len() - end),
        ));
    }
    clip(&out, max)
}

/// 单条条目的检索关键词
fn item_keywords(item: &VulnItem) -> Vec<String> {
    let mut out: std::collections::BTreeSet<String> = Default::default();
    for text in [&item.title, &item.detect_point] {
        for tok in tokens(text) {
            out.insert(tok);
        }
    }
    for hint in category_hints(&item.category) {
        out.insert((*hint).to_string());
    }
    // 一条条目用不着几十个词，太多会互相稀释
    let mut v: Vec<String> = out.into_iter().collect();
    v.truncate(24);
    v
}

/// 检索用的停用词：这些词在任何代码里都有，拿来打分只会把结果拉平
const STOP: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "are", "not", "but", "all", "any", "can",
    "has", "have", "its", "may", "use", "used", "when", "which", "will", "you", "your", "http",
    "https", "com", "www", "src",
];

/// 每一类漏洞该去哪些词里找。清单标题多是中文，直接拿去检索几乎命中不了，
/// 所以按分类补一组「代码里真会出现」的词。
pub fn category_hints(category: &str) -> &'static [&'static str] {
    match category.chars().next() {
        Some('A') => &[
            "auth", "login", "signin", "session", "password", "user", "token", "jwt", "register",
            "oauth",
        ],
        Some('B') => &[
            "api",
            "route",
            "router",
            "controller",
            "handler",
            "endpoint",
            "order",
            "user",
            "permission",
            "role",
            "admin",
            "owner",
        ],
        Some('C') => &[
            "session", "cookie", "token", "jwt", "refresh", "logout", "csrf", "expire",
        ],
        Some('D') => &[
            "sql",
            "query",
            "execute",
            "cursor",
            "subprocess",
            "exec",
            "eval",
            "shell",
            "cmd",
            "orm",
            "template",
            "render",
        ],
        Some('E') => &[
            "validate", "sanitize", "parse", "schema", "dto", "form", "input", "param", "query",
            "body",
        ],
        Some('F') => &[
            "upload",
            "file",
            "path",
            "filename",
            "ext",
            "mime",
            "zip",
            "tar",
            "download",
            "multipart",
        ],
        Some('G') => &[
            "http", "https", "tls", "ssl", "verify", "cors", "origin", "header", "proxy",
        ],
        Some('H') => &[
            "api",
            "route",
            "rate",
            "limit",
            "throttle",
            "middleware",
            "cors",
            "status",
            "error",
        ],
        Some('I') => &[
            "encrypt", "decrypt", "cipher", "hash", "md5", "sha", "aes", "rsa", "random", "salt",
            "iv",
        ],
        Some('J') => &[
            "key",
            "secret",
            "token",
            "password",
            "env",
            "config",
            "credential",
            "api_key",
        ],
        Some('K') => &[
            "price", "amount", "total", "coupon", "discount", "order", "balance", "quota", "points",
        ],
        Some('L') => &[
            "pii", "email", "phone", "address", "log", "export", "dump", "backup", "mask",
        ],
        Some('M') => &[
            "innerhtml",
            "dangerouslysetinnerhtml",
            "v-html",
            "localstorage",
            "document",
            "window",
            "eval",
            "cookie",
            "token",
            "script",
        ],
        Some('N') => &[
            "package.json",
            "requirements",
            "lock",
            "dependency",
            "version",
            "npm",
            "pip",
        ],
        Some('O') => &[
            "dockerfile",
            "docker-compose",
            "k8s",
            "yaml",
            "terraform",
            "iam",
            "s3",
            "bucket",
            "root",
        ],
        Some('P') => &[
            "lock", "mutex", "thread", "async", "await", "queue", "pool", "timeout", "retry",
            "limit",
        ],
        Some('Q') => &[
            "log",
            "logger",
            "audit",
            "trace",
            "error",
            "exception",
            "alert",
            "metric",
        ],
        Some('R') => &[
            "prompt",
            "llm",
            "agent",
            "model",
            "tool",
            "openai",
            "embedding",
            "completion",
        ],
        Some('S') => &[
            "todo",
            "unwrap",
            "expect",
            "panic",
            "null",
            "undefined",
            "any",
            "lint",
        ],
        _ => &[],
    }
}

/// 一批条目的关键词并集，只用来算文档频率
fn keywords(batch: &Batch) -> Vec<String> {
    let mut out: std::collections::BTreeSet<String> = Default::default();
    for item in &batch.items {
        for k in item_keywords(item) {
            out.insert(k);
        }
    }
    // 关键词太多会互相稀释，一批 60 条也就用到前 120 个
    let mut v: Vec<String> = out.into_iter().collect();
    v.truncate(120);
    v
}

/// 抽出代码里可能出现的词：ASCII 标识符，长度 ≥ 3
fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
            cur.push(c.to_ascii_lowercase());
        } else {
            push_token(&mut out, &mut cur);
        }
    }
    push_token(&mut out, &mut cur);
    out
}

fn push_token(out: &mut Vec<String>, cur: &mut String) {
    let numeric = cur.chars().all(|c| c.is_ascii_digit());
    if cur.len() >= 3 && !numeric && !STOP.contains(&cur.as_str()) {
        out.push(std::mem::take(cur));
    } else {
        cur.clear();
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    pick(
        format!("{head}\n…（已截断）"),
        format!("{head}\n… (truncated)"),
    )
}

/// 拼提示词。`json` 这个词必须出现，否则 DeepSeek 的 json 模式会报错。
pub fn prompt(batch: &Batch, index: &Index) -> String {
    // 这一批要看的代码：按条目关键词从索引里检索出来，而不是全仓库一份摘要
    let files = index.retrieve(batch, RETRIEVE_BUDGET);
    let mut out = String::new();
    out.push_str(
        "你是安全审计员，正在审计一个可能由 AI 生成的代码库（vibe coding）。\n\
         下面给出仓库文件树、与本批条目相关的代码，以及一批需要你判断的漏洞核对条目。\n\
         代码是按关键词挑出来的，可能不完整；看不出来就写 undetermined。\n\n\
         对每一条，判断它在本仓库中的状态，并给出证据：\n\
         - exists：你确实看到了有问题的代码\n\
         - suspected：看起来像，但证据不足\n\
         - not_found：看了相关代码，没有这个问题\n\
         - undetermined：需要把服务跑起来发请求才能确认（限流、爆破、越权、响应头等）\n\n\
         硬性要求：\n\
         1. 每一条都必须给 evidence：`文件:行号`；确实判断不了的，写清楚缺什么信息。\n\
         2. 不要跳过任何条目，不要编造条目。\n\
         3. **同一个问题只报一条**：多条清单项指的是同一处代码或同一个事实时，\
         选最贴切的那条报 exists/suspected，其余写 not_found，reason 写「与 XX 同源」。\
         不要为了凑数把一处问题贴到七八个条目上。\n\
         4. **仓库里没有的东西直接写 not_found**：没有 Agent、没有 K8s、没有前端，\
         就如实说没有，不要拿别处的问题去顶。\n\
         5. 只输出 json，不要解释，也不要包代码块围栏。\n\n\
         输出格式：\n\
         {\"results\":[{\"id\":\"AUTH-01\",\"verdict\":\"exists\",\"evidence\":\"app.py:12\",\"reason\":\"一句话依据\"}]}\n\n",
    );

    out.push_str(tr("## 仓库文件树\n```\n"));
    out.push_str(&index.tree);
    out.push_str("\n```\n\n");

    if files.is_empty() {
        out.push_str(
            "## 相关代码\n（这个仓库里没找到和本批条目相关的代码。\
             判断不了就写 undetermined，并在 reason 里说明缺什么。）\n\n",
        );
    } else {
        out.push_str(tr(
            "## 与本批条目相关的代码（按关键词检索挑出来的，不是全部）\n",
        ));
        for (path, body) in &files {
            out.push_str(&format!("### {path}\n```\n{body}\n```\n\n"));
        }
        let rest = index.len().saturating_sub(files.len());
        if rest > 0 {
            out.push_str(&pick(
                format!("（仓库里另有 {rest} 个文件与本批条目不相关，未列出）\n\n"),
                format!(
                    "(the repository has {rest} more files unrelated to this batch, not listed)\n\n"
                ),
            ));
        }
    }

    out.push_str(tr("## 待判断条目\n"));
    for item in &batch.items {
        out.push_str(&pick(
            format!(
                "- {} [{}] {}｜检测点：{}\n",
                item.id,
                item.severity.label(),
                item.title,
                item.detect_point
            ),
            format!(
                "- {} [{}] {} | check: {}\n",
                item.id,
                item.severity.label(),
                item.title,
                item.detect_point
            ),
        ));
    }
    out
}

/// 让模型给出修复后的**完整文件**。返回整个文件而不是 diff，
/// 是因为 diff 的上下文对不上时应用会失败，而全文替换是幂等的。
pub fn fix_prompt(finding: &crate::finding::Finding, content: &str) -> String {
    pick(
        format!(
            "你是安全工程师，正在修一个具体的安全问题。\n\n         文件：{file}\n         问题：[{id}] {title}\n         位置：第 {line} 行\n         说明：{desc}\n         修复要点：{fix}\n\n         要求：\n         1. 只改和这个问题相关的代码，其余原样保留，不要顺手重构、不要改格式。\n         2. 返回**修好之后的完整文件**，不要省略、不要用「...」代替、不要只给片段。\n         3. 如果这个文件里其实没有问题，把 content 原样返回，并在 note 里说明理由。\n\n         以 json 输出：{{\"content\":\"完整文件内容\",\"note\":\"改了什么\"}}\n\n         文件全文：\n```\n{content}\n```",
            file = finding.file,
            id = finding
                .description
                .split(']')
                .next()
                .unwrap_or("")
                .trim_start_matches('['),
            title = finding.title,
            line = finding.line,
            desc = finding.description,
            fix = finding.suggestion,
        ),
        format!(
            "You are a security engineer fixing one specific security issue.\n\n         File: {file}\n         Issue: [{id}] {title}\n         Location: line {line}\n         Detail: {desc}\n         How to fix: {fix}\n\n         Requirements:\n         1. Change only the code related to this issue and keep everything else as is: no opportunistic refactoring, no reformatting.\n         2. Return the **complete fixed file**; do not elide anything, do not use \"...\", do not return a fragment.\n         3. If this file actually has no problem, return content unchanged and explain why in note.\n\n         Output as json: {{\"content\":\"full file content\",\"note\":\"what changed\"}}\n\n         Full file:\n```\n{content}\n```",
            file = finding.file,
            id = finding
                .description
                .split(']')
                .next()
                .unwrap_or("")
                .trim_start_matches('['),
            title = finding.title,
            line = finding.line,
            desc = finding.description,
            fix = finding.suggestion,
        ),
    )
}

/// 解析修复结果。模型有时不守 json 的约定，直接吐一段带围栏的代码，这里一并接住。
pub fn parse_fix(text: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Raw {
        content: String,
    }
    if let Some(start) = text.find('{')
        && let Some(end) = text.rfind('}')
        && end > start
        && let Ok(raw) = serde_json::from_str::<Raw>(&text[start..=end])
    {
        return Ok(raw.content);
    }
    // 退路：剥掉代码块围栏，整段当文件内容
    let trimmed = text.trim();
    if trimmed.starts_with("```") {
        let body = trimmed
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or(trimmed);
        let body = body.strip_suffix("```").unwrap_or(body);
        return Ok(body.trim_end().to_string());
    }
    anyhow::bail!("{}", tr("模型没有返回可以解析的文件内容"))
}

/// 增量判定后的合并规则：
///
/// - 新的「存在 / 疑似」一律采纳——这一轮只看了变过的文件，能看到的都是新证据；
/// - 旧判定只有在**它自己的证据文件变过**时才被替换，否则沿用上次结论
///   （没变过的文件里，上次看到的问题还在）；
/// - 上次没有的条目直接收下。
///
/// 这条规则的偏向很明确：宁可多报一条让人看一眼，也不因为「这一轮没扫到」就把
/// 已知的问题抹掉。
pub fn merge(prev: &[Judgement], fresh: Vec<Judgement>, changed: &[String]) -> Vec<Judgement> {
    let mut out: Vec<Judgement> = Vec::with_capacity(prev.len().max(fresh.len()));
    for old in prev {
        let evidence_file = split_evidence(&old.evidence).0;
        let old_file_changed = changed.iter().any(|c| c == &evidence_file);
        match fresh.iter().find(|n| n.id == old.id) {
            Some(new) => {
                let take_new =
                    matches!(new.verdict, Verdict::Exists | Verdict::Suspected) || old_file_changed;
                out.push(if take_new { new.clone() } else { old.clone() });
            }
            // 这一轮没判到这条：证据文件没变就保留，变了就丢掉（它的依据已经不在了）
            None => {
                if !old_file_changed {
                    out.push(old.clone());
                }
            }
        }
    }
    for new in fresh {
        if !prev.iter().any(|p| p.id == new.id) {
            out.push(new);
        }
    }
    out
}

#[derive(Deserialize)]
struct Raw {
    results: Vec<RawItem>,
}

#[derive(Deserialize)]
struct RawItem {
    id: String,
    verdict: String,
    #[serde(default)]
    evidence: String,
    #[serde(default)]
    reason: String,
}

/// 解析模型输出。模型经常在 JSON 外面裹一层 ```json，或者前后带一句话，
/// 所以先截取第一个 `{` 到最后一个 `}` 再解析。
pub fn parse(text: &str) -> Result<Vec<Judgement>> {
    let start = text.find('{').context(tr("模型没有返回 JSON"))?;
    let end = text.rfind('}').context(tr("模型返回的 JSON 不完整"))?;
    if end <= start {
        anyhow::bail!("{}", tr("模型返回的 JSON 不完整"));
    }
    let raw: Raw =
        serde_json::from_str(&text[start..=end]).context(tr("模型返回的 JSON 解析失败"))?;

    let mut out = Vec::new();
    for r in raw.results {
        let Some(verdict) = Verdict::parse(&r.verdict) else {
            continue; // 认不出的判定直接丢掉，不要猜
        };
        let evidence = r.evidence.trim();
        out.push(Judgement {
            id: r.id.trim().to_string(),
            verdict,
            evidence: if evidence.is_empty() {
                tr("（未给出证据）").to_string()
            } else {
                evidence.to_string()
            },
            reason: r.reason.trim().to_string(),
        });
    }
    Ok(out)
}

/// 把判定转成 `(文件, 行号)`。模型给的证据五花八门：
/// `app.py:12`、`app.py:12-18`、`src/a.rs:3（函数 verify）`、`见 app.py`
pub fn split_evidence(evidence: &str) -> (String, u32) {
    let head = evidence
        .split(['（', '('])
        .next()
        .unwrap_or(evidence)
        .trim();
    if let Some((file, rest)) = head.rsplit_once(':') {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(line) = digits.parse::<u32>() {
            return (file.trim().to_string(), line);
        }
    }
    (head.to_string(), 0)
}

/// 把「存在 / 疑似」的判定并进 `Finding` 列表，和本地规则的结果放一起，
/// 后面 `/fix` 就能统一处理。严重级别和修复建议都从清单取。
/// 一条判定在磁盘上的落地情况
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub file: String,
    pub line: u32,
    /// 那一行的原文（读得到才有）
    pub text: Option<String>,
    /// 核实不了的原因；`None` 表示这一条站得住
    pub problem: Option<String>,
}

/// 核实「文件:行号」是不是真的存在。
///
/// 模型给出的位置经常是错的——文件不存在、行号超出范围、或者行号对但内容
/// 根本不是它说的那回事。不核实的话，报告里就会出现「IDOR 在 app.py:11」，
/// 而那一行是个纯函数。核实很便宜（读一次文件），收益是报告里每一条都能点开看。
pub fn verify_evidence(root: &Path, evidence: &str) -> Evidence {
    let (file, line) = split_evidence(evidence);
    let text = match std::fs::read_to_string(root.join(&file)) {
        Ok(t) => t,
        Err(_) => {
            return Evidence {
                file,
                line,
                text: None,
                problem: Some(tr("文件不存在").into()),
            };
        }
    };
    let rows: Vec<&str> = text.lines().collect();
    let idx = line as usize;
    if idx == 0 || idx > rows.len() {
        return Evidence {
            file,
            line,
            text: None,
            problem: Some(pick(
                format!("文件只有 {} 行", rows.len()),
                format!("The file has only {} lines", rows.len()),
            )),
        };
    }
    Evidence {
        file,
        line,
        text: Some(rows[idx - 1].trim().to_string()),
        problem: None,
    }
}

/// 合并后的结果：能定位到代码的，以及定位不到的
pub struct Merged {
    pub findings: Vec<crate::finding::Finding>,
    /// (清单编号, 模型给的证据, 核实不了的原因)
    pub unverified: Vec<(String, String, String)>,
}

pub fn to_findings(
    root: &Path,
    judgements: &[Judgement],
    checklist: &VulnChecklist,
    seq_start: usize,
) -> Merged {
    let mut findings = Vec::new();
    let mut unverified = Vec::new();
    let mut seen: std::collections::BTreeSet<(String, String, u32)> = Default::default();

    for j in judgements
        .iter()
        .filter(|j| matches!(j.verdict, Verdict::Exists | Verdict::Suspected))
    {
        let Some(item) = checklist.get(&j.id) else {
            continue;
        };
        let ev = verify_evidence(root, &j.evidence);
        if let Some(problem) = ev.problem {
            unverified.push((j.id.clone(), j.evidence.clone(), problem));
            continue;
        }
        // 同一条清单项落在同一行，只留一条：模型经常把同一个问题换个说法重复报
        if !seen.insert((j.id.clone(), ev.file.clone(), ev.line)) {
            continue;
        }
        let code = ev
            .text
            .as_deref()
            .map(|t| {
                let clipped: String = t.chars().take(90).collect();
                pick(
                    format!(" · 该行：`{clipped}`"),
                    format!(" · that line: `{clipped}`"),
                )
            })
            .unwrap_or_default();
        findings.push(crate::finding::Finding {
            id: format!("VG-{:03}", seq_start + findings.len() + 1),
            severity: item.severity,
            title: item.title.clone(),
            cwe: item.cwe.clone(),
            file: ev.file,
            line: ev.line,
            description: pick(
                format!(
                    "[{}] 模型判定为「{}」：{}{}",
                    j.id,
                    j.verdict.label(),
                    j.reason,
                    code
                ),
                format!(
                    "[{}] model says \"{}\": {}{}",
                    j.id,
                    j.verdict.label(),
                    j.reason,
                    code
                ),
            ),
            suggestion: item.fix_point.clone(),
        });
    }
    Merged {
        findings,
        unverified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn batches_cover_every_item_exactly_once() {
        let list = VulnChecklist::new();
        let bs = batches_with(&list, &[], BATCH, &[]);
        let total: usize = bs.iter().map(|b| b.items.len()).sum();
        assert_eq!(total, list.len(), "有条目被漏掉或重复");
        assert!(bs.iter().all(|b| b.items.len() <= BATCH));
        assert!(bs.len() > 1, "410 条不可能塞进一批");
    }

    #[test]
    fn the_default_scope_leaves_out_the_engineering_categories() {
        let list = VulnChecklist::new();
        let skip = crate::scan::covered_ids();

        let security = batches_with(&list, &skip, BATCH, SECURITY_CATEGORIES);
        let ids: Vec<&str> = security
            .iter()
            .flat_map(|b| b.items.iter().map(|i| i.id.as_str()))
            .collect();
        for id in &ids {
            let item = list.get(id).unwrap();
            assert!(
                is_security_category(&item.category),
                "{id}（{}）不该进安全类判定",
                item.category
            );
        }

        // --all 时四类工程规范都要回来
        let everything = batches_with(&list, &skip, BATCH, &[]);
        let total_all: usize = everything.iter().map(|b| b.items.len()).sum();
        let total_sec: usize = security.iter().map(|b| b.items.len()).sum();
        assert!(total_sec < total_all, "安全类应当少一些");
        assert_eq!(total_all, list.len() - skip.len());
    }

    #[test]
    fn the_category_letters_match_the_checklist() {
        assert!(is_security_category("A. 认证与账号"));
        assert!(is_security_category("R. Agent 与 LLM 特有"));
        assert!(!is_security_category("S. 代码质量与正确性"));
        assert!(!is_security_category("Q. 日志、监控与应急"));
        assert!(!is_security_category("N. 依赖与供应链"));
        assert!(!is_security_category("O. 云与基础设施"));
    }

    #[test]
    fn batches_skip_what_the_local_rules_already_decided() {
        let list = VulnChecklist::new();
        let skip = crate::scan::covered_ids();
        let bs = batches_with(&list, &skip, BATCH, &[]);
        let total: usize = bs.iter().map(|b| b.items.len()).sum();
        assert_eq!(
            total,
            list.len() - skip.len(),
            "本地判过的条目不该再问一遍模型"
        );
        for b in &bs {
            for item in &b.items {
                assert!(!skip.contains(&item.id.as_str()), "{} 重复了", item.id);
            }
        }
    }

    #[test]
    fn bigger_batches_need_fewer_requests() {
        let list = VulnChecklist::new();
        let skip = crate::scan::covered_ids();
        let normal = batches_with(&list, &skip, BATCH, &[]).len();
        let big = batches_with(&list, &skip, BATCH_INCREMENTAL, &[]).len();
        assert!(big < normal, "{big} 应该比 {normal} 少");
        let total: usize = batches_with(&list, &skip, BATCH_INCREMENTAL, &[])
            .iter()
            .map(|b| b.items.len())
            .sum();
        assert_eq!(total, list.len() - skip.len(), "条数不能丢");
    }

    #[test]
    fn the_index_skips_dependency_directories_and_lists_the_tree() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("auth.rs"), "fn login() {}".repeat(50)).unwrap();
        fs::write(dir.path().join("random.rs"), "// nothing").unwrap();
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        fs::write(dir.path().join("node_modules/x.js"), "junk").unwrap();

        let idx = Index::build(dir.path());
        assert!(idx.tree.contains("auth.rs"));
        assert!(!idx.tree.contains("node_modules"), "依赖目录不该进索引");
        assert_eq!(idx.len(), 2);
    }

    #[test]
    fn retrieval_picks_the_file_that_actually_talks_about_the_item() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("api")).unwrap();
        fs::write(
            dir.path().join("api/orders.py"),
            "def get_order(oid):\n    return db.execute('select * from orders where id = ?', (oid,))\n",
        )
        .unwrap();
        fs::write(dir.path().join("styles.css"), "body { color: red }\n").unwrap();
        fs::write(dir.path().join("README.md"), "# demo\n").unwrap();

        let idx = Index::build(dir.path());
        // 只留一条 AUTHZ（授权/越权）类的条目
        let list = VulnChecklist::new();
        let item = list
            .by_category("B. 授权与越权")
            .into_iter()
            .next()
            .expect("清单里应当有授权类条目");
        let batch = Batch {
            label: "B".into(),
            items: vec![item.clone()],
        };
        let got = idx.retrieve(&batch, RETRIEVE_BUDGET);
        let paths: Vec<&str> = got.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"api/orders.py"), "{paths:?}");
        assert!(!paths.contains(&"styles.css"), "不相关的文件不该被选中");
    }

    #[test]
    fn excerpts_carry_line_numbers_so_the_model_can_cite_them() {
        let dir = tempdir().unwrap();
        let body: Vec<String> = (1..=80).map(|i| format!("line{i}")).collect();
        fs::write(dir.path().join("a.py"), body.join("\n")).unwrap();
        let idx = Index::build(dir.path());
        let doc = &idx.docs[0];
        let out = excerpt(doc, &["line40".to_string()], SNIPPET_BUDGET);
        assert!(out.contains("   40  line40"), "{out}");
        assert!(out.contains("行省略"), "要标出省略，{out}");
        assert!(out.contains("后面还有"), "{out}");
    }

    #[test]
    fn retrieval_respects_the_budget() {
        let dir = tempdir().unwrap();
        for i in 0..5 {
            fs::write(
                dir.path().join(format!("f{i}.py")),
                "query = 'select * from users where id = ?'\n".repeat(200),
            )
            .unwrap();
        }
        let idx = Index::build(dir.path());
        let list = VulnChecklist::new();
        let batch = Batch {
            label: "D".into(),
            items: list
                .by_category("D. 注入类")
                .into_iter()
                .take(3)
                .cloned()
                .collect(),
        };
        let got = idx.retrieve(&batch, 5_000);
        let total: usize = got.iter().map(|(_, b)| b.len()).sum();
        assert!(total <= 5_000, "预算要守住，实际 {total}");
    }

    #[test]
    fn a_scoped_index_only_looks_at_the_files_that_changed() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.py"), "login()\n").unwrap();
        fs::write(dir.path().join("b.py"), "login()\n").unwrap();
        let idx = Index::build(dir.path());
        let scoped = idx.scoped(&["b.py".to_string()]);
        assert_eq!(scoped.len(), 1);
        let list = VulnChecklist::new();
        let batch = Batch {
            label: "A".into(),
            items: list
                .by_category("A. 认证与账号")
                .into_iter()
                .take(2)
                .cloned()
                .collect(),
        };
        let paths: Vec<String> = scoped
            .retrieve(&batch, RETRIEVE_BUDGET)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        assert!(paths.iter().all(|p| p == "b.py"), "{paths:?}");
    }

    #[test]
    fn prompt_lists_every_item_and_asks_for_json() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.py"), "print(1)\n").unwrap();
        let idx = Index::build(dir.path());
        let list = VulnChecklist::new();
        let b = &batches_with(&list, &[], BATCH, &[])[0];
        let p = prompt(b, &idx);
        for item in &b.items {
            assert!(p.contains(&item.id), "提示词里少了 {}", item.id);
        }
        assert!(
            p.contains("json"),
            "DeepSeek 的 json 模式要求提示词里出现 json"
        );
        assert!(p.contains("a.py"));
    }

    #[test]
    fn parse_survives_code_fences_and_extra_prose() {
        let text = "好的，结果如下：\n```json\n{\"results\":[\
            {\"id\":\"AUTH-01\",\"verdict\":\"exists\",\"evidence\":\"app.py:12\",\"reason\":\"没挂鉴权\"},\
            {\"id\":\"AUTH-02\",\"verdict\":\"not_found\",\"evidence\":\"auth.py:30\"}\
        ]}\n```\n以上。";
        let v = parse(text).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].verdict, Verdict::Exists);
        assert_eq!(v[0].evidence, "app.py:12");
        assert_eq!(v[1].verdict, Verdict::NotFound);
    }

    #[test]
    fn parse_accepts_chinese_verdicts_and_drops_unknown_ones() {
        let text = r#"{"results":[
            {"id":"A","verdict":"存在","evidence":"x.py:1"},
            {"id":"B","verdict":"无法判断","evidence":"需要跑起来"},
            {"id":"C","verdict":"胡说八道","evidence":"y"}
        ]}"#;
        let v = parse(text).unwrap();
        assert_eq!(v.len(), 2, "认不出的判定要丢掉，不能猜");
        assert_eq!(v[1].verdict, Verdict::Undetermined);
    }

    #[test]
    fn only_exists_and_suspected_become_findings() {
        let repo = tempdir().unwrap();
        fs::create_dir_all(repo.path().join("api")).unwrap();
        fs::write(repo.path().join("api/orders.py"), "a\nb\nc\n").unwrap();
        let list = VulnChecklist::new();
        let js = vec![
            Judgement {
                id: "AUTHZ-01".into(),
                verdict: Verdict::Exists,
                evidence: "api/orders.py:2".into(),
                reason: "查询没带 owner 条件".into(),
            },
            Judgement {
                id: "AUTH-07".into(),
                verdict: Verdict::Undetermined,
                evidence: "需要跑起来".into(),
                reason: "限流只能运行时看".into(),
            },
            Judgement {
                id: "NOPE-99".into(),
                verdict: Verdict::Exists,
                evidence: "x:1".into(),
                reason: "清单里没有这个编号".into(),
            },
        ];
        let m = to_findings(repo.path(), &js, &list, 0);
        assert_eq!(
            m.findings.len(),
            1,
            "无法判断的不算发现，编号不存在的也要丢掉"
        );
        let f = &m.findings[0];
        assert_eq!(f.file, "api/orders.py");
        assert_eq!(f.line, 2);
        assert_eq!(f.severity, crate::finding::Severity::Critical);
        assert!(f.description.contains("AUTHZ-01"));
        // 报告里带上那一行的原文，用户一眼就能判断真假
        assert!(f.description.contains("该行：`b`"), "{}", f.description);
    }

    #[test]
    fn evidence_pointing_at_nothing_is_dropped_and_reported() {
        let repo = tempdir().unwrap();
        fs::write(repo.path().join("app.py"), "line1\nline2\n").unwrap();
        let list = VulnChecklist::new();
        let js = vec![
            Judgement {
                id: "AUTHZ-01".into(),
                verdict: Verdict::Exists,
                evidence: "app.py:99".into(),
                reason: "行号是编的".into(),
            },
            Judgement {
                id: "AUTH-01".into(),
                verdict: Verdict::Suspected,
                evidence: "nope.py:3".into(),
                reason: "文件根本不存在".into(),
            },
        ];
        let m = to_findings(repo.path(), &js, &list, 0);
        assert!(m.findings.is_empty(), "核实不了的不能进结果");
        assert_eq!(m.unverified.len(), 2);
        assert!(
            m.unverified[0].2.contains("只有 2 行"),
            "{:?}",
            m.unverified
        );
        assert!(
            m.unverified[1].2.contains("文件不存在"),
            "{:?}",
            m.unverified
        );
    }

    #[test]
    fn the_same_item_on_the_same_line_is_reported_once() {
        let repo = tempdir().unwrap();
        fs::write(repo.path().join("a.py"), "x\ny\n").unwrap();
        let list = VulnChecklist::new();
        let js = vec![
            Judgement {
                id: "AUTHZ-01".into(),
                verdict: Verdict::Exists,
                evidence: "a.py:1".into(),
                reason: "第一次".into(),
            },
            Judgement {
                id: "AUTHZ-01".into(),
                verdict: Verdict::Suspected,
                evidence: "a.py:1".into(),
                reason: "换个说法又说一遍".into(),
            },
        ];
        let m = to_findings(repo.path(), &js, &list, 0);
        assert_eq!(m.findings.len(), 1, "同一条清单项同一行只留一条");
    }

    #[test]
    fn fix_prompt_carries_the_file_and_forbids_shortcuts() {
        let f = crate::finding::Finding {
            id: "VG-001".into(),
            severity: crate::finding::Severity::Critical,
            title: "硬编码密钥".into(),
            cwe: Some("CWE-798".into()),
            file: "app.py".into(),
            line: 3,
            description: "[CONF-01] 密钥写死".into(),
            suggestion: "改从环境变量读".into(),
        };
        let p = fix_prompt(&f, "API_KEY = \"sk-1\"\n");
        assert!(p.contains("app.py"));
        assert!(p.contains("CONF-01"), "要把清单编号告诉模型");
        assert!(p.contains("完整文件"), "必须要求返回全文，否则没法整体替换");
        assert!(p.contains("API_KEY"));
        assert!(p.contains("json"));
    }

    #[test]
    fn parse_fix_takes_json_or_a_fenced_block() {
        assert_eq!(
            parse_fix(r#"{"content":"line1\nline2","note":"改好了"}"#).unwrap(),
            "line1\nline2"
        );
        assert_eq!(parse_fix("```python\nprint(1)\n```").unwrap(), "print(1)");
        assert!(parse_fix("我不知道").is_err(), "解析不了就要报错，不能瞎猜");
    }

    fn judged(id: &str, verdict: Verdict, evidence: &str) -> Judgement {
        Judgement {
            id: id.into(),
            verdict,
            evidence: evidence.into(),
            reason: String::new(),
        }
    }

    #[test]
    fn incremental_merge_keeps_old_findings_in_untouched_files() {
        let prev = vec![
            judged("AUTHZ-01", Verdict::Exists, "api/orders.py:42"),
            judged("INJ-01", Verdict::NotFound, "api/orders.py:10"),
        ];
        // 这一轮只看 utils.py，两条都没在里头
        let fresh = vec![
            judged("AUTHZ-01", Verdict::NotFound, "utils.py"),
            judged("INJ-01", Verdict::NotFound, "utils.py"),
        ];
        let merged = merge(&prev, fresh, &["utils.py".to_string()]);
        assert_eq!(merged.len(), 2);
        assert_eq!(
            merged[0].verdict,
            Verdict::Exists,
            "没变过的文件里的问题要留着"
        );
        assert_eq!(merged[0].evidence, "api/orders.py:42");
    }

    #[test]
    fn a_finding_whose_own_file_changed_is_replaced_even_if_it_disappears() {
        let prev = vec![judged("AUTHZ-01", Verdict::Exists, "api/orders.py:42")];
        let fresh = vec![judged("AUTHZ-01", Verdict::NotFound, "api/orders.py")];
        let merged = merge(&prev, fresh, &["api/orders.py".to_string()]);
        assert_eq!(merged[0].verdict, Verdict::NotFound, "修好了就该消失");
    }

    #[test]
    fn new_evidence_always_wins() {
        let prev = vec![judged("INJ-01", Verdict::NotFound, "app.py")];
        let fresh = vec![judged("INJ-01", Verdict::Exists, "app.py:11")];
        let merged = merge(&prev, fresh, &["app.py".to_string()]);
        assert_eq!(merged[0].verdict, Verdict::Exists);
        assert_eq!(merged[0].evidence, "app.py:11");
    }

    #[test]
    fn merge_keeps_items_that_only_exist_on_one_side() {
        let prev = vec![judged("A", Verdict::Exists, "a.py:1")];
        let fresh = vec![judged("B", Verdict::Exists, "b.py:2")];
        let merged = merge(&prev, fresh, &["b.py".to_string()]);
        let ids: Vec<&str> = merged.iter().map(|j| j.id.as_str()).collect();
        assert_eq!(ids, ["A", "B"], "两边独有的条目都要留下");
    }

    #[test]
    fn evidence_parsing_handles_the_shapes_models_actually_emit() {
        assert_eq!(split_evidence("app.py:12"), ("app.py".into(), 12));
        assert_eq!(split_evidence("src/a.rs:3-8"), ("src/a.rs".into(), 3));
        assert_eq!(
            split_evidence("app.py:12（verify 函数）"),
            ("app.py".into(), 12)
        );
        assert_eq!(split_evidence("见 app.py"), ("见 app.py".into(), 0));
        assert_eq!(split_evidence("需要运行验证"), ("需要运行验证".into(), 0));
    }
}
