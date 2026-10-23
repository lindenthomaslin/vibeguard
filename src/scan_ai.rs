//! 扫描的第二段：把本地规则判不了的清单条目交给模型逐条判断。
//!
//! 和第一段的分工很清楚：
//! - 本地规则负责**证据确凿**的东西（字符串里有个密钥、代码里写了 `shell=True`）；
//! - 这一段负责**需要读上下文**的东西（这条查询带没带 owner 条件？这个路由挂没挂鉴权？）。
//!
//! 关键约束：**每条都必须给出 `文件:行号` 或「缺什么信息」**。允许模型说
//! 「无法判断」，不允许它说「看着没问题」——后者是最危险的一种输出，因为它
//! 长得像结论。

use crate::vuln_checklist::{VulnChecklist, VulnItem};
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use serde::Deserialize;
use std::path::Path;

/// 一次喂给模型多少条。太多会漏条目、输出被截断；太少则请求次数爆炸。
/// 60 条大约 8 次请求，是个折中：再小就变成 20 次，一次真模型调用十几秒，太慢。
const BATCH: usize = 60;
/// 仓库摘要里正文的总预算（字符），超了就只留文件树
const DIGEST_BUDGET: usize = 40_000;
/// 单个文件最多塞进去多少字符
const FILE_BUDGET: usize = 8_000;
/// 文件树最多列多少行
const TREE_LINES: usize = 300;

/// 模型的判定
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
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
            Verdict::Suspected => "疑似",
            Verdict::NotFound => "不存在",
            Verdict::Undetermined => "无法判断",
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

/// 一条判定结果
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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

/// 按分类顺序把条目装进若干批，每批不超过 `BATCH` 条。
/// 跨分类装批是为了少发请求——真模型一次要十几秒，按 19 个分类发就是五分钟。
pub fn batches(checklist: &VulnChecklist, skip: &[&str]) -> Vec<Batch> {
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
            format!("{} 等 {} 类", cats[0], cats.len())
        };
        out.push(Batch {
            label,
            items: std::mem::take(items),
        });
        cats.clear();
    };

    for cat in checklist.categories() {
        for item in checklist.by_category(cat).into_iter().filter(|i| !skip.contains(&i.id.as_str())) {
            if current.len() >= BATCH {
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

/// 仓库摘要：文件树 + 挑出来的源码
pub struct Digest {
    pub tree: String,
    pub files: Vec<(String, String)>,
    /// 因为预算被省略的文件数
    pub omitted: usize,
}

/// 名字里带这些词的文件优先塞进摘要——安全问题和它们最相关
const HOT_WORDS: &[&str] = &[
    "auth", "login", "signin", "session", "token", "jwt", "user", "admin", "permission",
    "role", "acl", "api", "route", "handler", "controller", "view", "middleware", "config",
    "setting", "db", "sql", "model", "query", "upload", "file", "exec", "cmd", "shell",
    "crypto", "secret", "key", "docker", "compose", "deploy", "nginx", "server",
];

const CODE_EXTS: &[&str] = &[
    "rs", "py", "js", "jsx", "ts", "tsx", "go", "java", "kt", "rb", "php", "cs", "swift", "c",
    "cc", "cpp", "h", "hpp", "sql", "vue", "svelte", "yml", "yaml", "toml", "env", "tf",
    "Dockerfile",
];

pub fn digest(root: &Path) -> Digest {
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
                "node_modules" | "target" | "dist" | "build" | ".git" | "vendor" | ".venv"
                    | "__pycache__" | "coverage" | "assets"
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

    let tree = paths
        .iter()
        .take(TREE_LINES)
        .map(|(p, _)| p.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let mut files = Vec::new();
    let mut used = 0usize;
    let mut omitted = 0usize;
    for (rel, _) in &paths {
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
            omitted += 1;
            continue;
        };
        if used + text.len() > DIGEST_BUDGET {
            omitted += 1;
            continue;
        }
        used += text.len();
        files.push((rel.clone(), clip(&text, FILE_BUDGET)));
    }

    Digest {
        tree,
        files,
        omitted,
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}\n…（已截断）")
}

/// 拼提示词。`json` 这个词必须出现，否则 DeepSeek 的 json 模式会报错。
pub fn prompt(batch: &Batch, digest: &Digest) -> String {
    let mut out = String::new();
    out.push_str(
        "你是安全审计员，正在审计一个可能由 AI 生成的代码库（vibe coding）。\n\
         下面给出仓库摘要，以及一批需要你判断的漏洞核对条目。\n\n\
         对每一条，判断它在本仓库中的状态，并给出证据：\n\
         - exists：你确实看到了有问题的代码\n\
         - suspected：看起来像，但证据不足\n\
         - not_found：看了相关代码，没有这个问题\n\
         - undetermined：需要把服务跑起来发请求才能确认（限流、爆破、越权、响应头等）\n\n\
         硬性要求：\n\
         1. 每一条都必须给 evidence：`文件:行号`；确实判断不了的，写清楚缺什么信息。\n\
         2. 不要跳过任何条目，不要编造条目。\n\
         3. 只输出 json，不要解释，也不要包代码块围栏。\n\n\
         输出格式：\n\
         {\"results\":[{\"id\":\"AUTH-01\",\"verdict\":\"exists\",\"evidence\":\"app.py:12\",\"reason\":\"一句话依据\"}]}\n\n",
    );

    out.push_str("## 仓库文件树\n```\n");
    out.push_str(&digest.tree);
    out.push_str("\n```\n\n");

    if !digest.files.is_empty() {
        out.push_str("## 关键文件内容\n");
        for (path, body) in &digest.files {
            out.push_str(&format!("### {path}\n```\n{body}\n```\n\n"));
        }
        if digest.omitted > 0 {
            out.push_str(&format!(
                "（另有 {} 个文件因篇幅未列出）\n\n",
                digest.omitted
            ));
        }
    }

    out.push_str("## 待判断条目\n");
    for item in &batch.items {
        out.push_str(&format!(
            "- {} [{}] {}｜检测点：{}\n",
            item.id,
            item.severity.label(),
            item.title,
            item.detect_point
        ));
    }
    out
}

/// 让模型给出修复后的**完整文件**。返回整个文件而不是 diff，
/// 是因为 diff 的上下文对不上时应用会失败，而全文替换是幂等的。
pub fn fix_prompt(finding: &crate::finding::Finding, content: &str) -> String {
    format!(
        "你是安全工程师，正在修一个具体的安全问题。\n\n         文件：{file}\n         问题：[{id}] {title}\n         位置：第 {line} 行\n         说明：{desc}\n         修复要点：{fix}\n\n         要求：\n         1. 只改和这个问题相关的代码，其余原样保留，不要顺手重构、不要改格式。\n         2. 返回**修好之后的完整文件**，不要省略、不要用「...」代替、不要只给片段。\n         3. 如果这个文件里其实没有问题，把 content 原样返回，并在 note 里说明理由。\n\n         以 json 输出：{{\"content\":\"完整文件内容\",\"note\":\"改了什么\"}}\n\n         文件全文：\n```\n{content}\n```",
        file = finding.file,
        id = finding.description.split(']').next().unwrap_or("").trim_start_matches('['),
        title = finding.title,
        line = finding.line,
        desc = finding.description,
        fix = finding.suggestion,
    )
}

/// 解析修复结果。模型有时不守 json 的约定，直接吐一段带围栏的代码，这里一并接住。
pub fn parse_fix(text: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Raw {
        content: String,
    }
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            if end > start {
                if let Ok(raw) = serde_json::from_str::<Raw>(&text[start..=end]) {
                    return Ok(raw.content);
                }
            }
        }
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
    anyhow::bail!("模型没有返回可以解析的文件内容")
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
    let start = text.find('{').context("模型没有返回 JSON")?;
    let end = text.rfind('}').context("模型返回的 JSON 不完整")?;
    if end <= start {
        anyhow::bail!("模型返回的 JSON 不完整");
    }
    let raw: Raw =
        serde_json::from_str(&text[start..=end]).context("模型返回的 JSON 解析失败")?;

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
                "（未给出证据）".to_string()
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
    let head = evidence.split(['（', '(']).next().unwrap_or(evidence).trim();
    if let Some((file, rest)) = head.rsplit_once(':') {
        let digits: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(line) = digits.parse::<u32>() {
            return (file.trim().to_string(), line);
        }
    }
    (head.to_string(), 0)
}

/// 把「存在 / 疑似」的判定并进 `Finding` 列表，和本地规则的结果放一起，
/// 后面 `/fix` 就能统一处理。严重级别和修复建议都从清单取。
pub fn to_findings(
    judgements: &[Judgement],
    checklist: &VulnChecklist,
    seq_start: usize,
) -> Vec<crate::finding::Finding> {
    judgements
        .iter()
        .filter(|j| matches!(j.verdict, Verdict::Exists | Verdict::Suspected))
        .enumerate()
        .filter_map(|(i, j)| {
            let item = checklist.get(&j.id)?;
            let (file, line) = split_evidence(&j.evidence);
            Some(crate::finding::Finding {
                id: format!("VG-{:03}", seq_start + i + 1),
                severity: item.severity,
                title: item.title.clone(),
                cwe: item.cwe.clone(),
                file,
                line,
                description: format!(
                    "[{}] 模型判定为「{}」：{}",
                    j.id,
                    j.verdict.label(),
                    j.reason
                ),
                suggestion: item.fix_point.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn batches_cover_every_item_exactly_once() {
        let list = VulnChecklist::new();
        let bs = batches(&list, &[]);
        let total: usize = bs.iter().map(|b| b.items.len()).sum();
        assert_eq!(total, list.len(), "有条目被漏掉或重复");
        assert!(bs.iter().all(|b| b.items.len() <= BATCH));
        assert!(bs.len() > 1, "410 条不可能塞进一批");
    }

    #[test]
    fn batches_skip_what_the_local_rules_already_decided() {
        let list = VulnChecklist::new();
        let skip = crate::scan::covered_ids();
        let bs = batches(&list, &skip);
        let total: usize = bs.iter().map(|b| b.items.len()).sum();
        assert_eq!(total, list.len() - skip.len(), "本地判过的条目不该再问一遍模型");
        for b in &bs {
            for item in &b.items {
                assert!(!skip.contains(&item.id.as_str()), "{} 重复了", item.id);
            }
        }
    }

    #[test]
    fn digest_prefers_security_relevant_files_and_respects_the_budget() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("auth.rs"), "fn login() {}".repeat(50)).unwrap();
        fs::write(dir.path().join("random.rs"), "// nothing").unwrap();
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        fs::write(dir.path().join("node_modules/x.js"), "junk").unwrap();

        let d = digest(dir.path());
        assert!(d.tree.contains("auth.rs"));
        assert!(!d.tree.contains("node_modules"), "依赖目录不该进摘要");
        assert_eq!(d.files[0].0, "auth.rs", "热点文件排第一");
        let total: usize = d.files.iter().map(|(_, b)| b.len()).sum();
        assert!(total <= DIGEST_BUDGET + FILE_BUDGET * 2);
    }

    #[test]
    fn prompt_lists_every_item_and_asks_for_json() {
        let list = VulnChecklist::new();
        let b = &batches(&list, &[])[0];
        let d = Digest {
            tree: "a.py".into(),
            files: vec![("a.py".into(), "print(1)".into())],
            omitted: 0,
        };
        let p = prompt(b, &d);
        for item in &b.items {
            assert!(p.contains(&item.id), "提示词里少了 {}", item.id);
        }
        assert!(p.contains("json"), "DeepSeek 的 json 模式要求提示词里出现 json");
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
        let list = VulnChecklist::new();
        let js = vec![
            Judgement {
                id: "AUTHZ-01".into(),
                verdict: Verdict::Exists,
                evidence: "api/orders.py:42".into(),
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
        let f = to_findings(&js, &list, 0);
        assert_eq!(f.len(), 1, "无法判断的不算发现，编号不存在的也要丢掉");
        assert_eq!(f[0].file, "api/orders.py");
        assert_eq!(f[0].line, 42);
        assert_eq!(f[0].severity, crate::finding::Severity::Critical);
        assert!(f[0].description.contains("AUTHZ-01"));
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
        assert_eq!(
            parse_fix("```python\nprint(1)\n```").unwrap(),
            "print(1)"
        );
        assert!(parse_fix("我不知道").is_err(), "解析不了就要报错，不能瞎猜");
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
