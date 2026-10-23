//! 扫描编排（第一段：本地规则）。
//!
//! 这一层的职责是**不花一分钱先拿到硬证据**：用正则扫出「肉眼可见」的问题
//! （硬编码密钥、`shell=True`、`eval`、ECB、关掉的证书校验…），每条都带
//! `文件:行号`，可以直接定位。
//!
//! scan:ignore-file —— 本文件全是规则定义（正则字面量、示例代码），扫自己会
//! 把自己匹配一遍，所以显式排除。

use crate::finding::{Finding, Severity};
use crate::vuln_checklist::VulnChecklist;
use ignore::WalkBuilder;
use regex::Regex;
use std::path::Path;
use std::sync::OnceLock;

/// 单个文件最大读取体积：再大就不是源码了
const MAX_FILE_BYTES: u64 = 1_500_000;
/// 同一个规则在同一个文件里最多报几处，避免一个文件刷屏
const PER_RULE_PER_FILE: usize = 3;
/// 一次扫描最多留多少条，防止在巨型仓库里把界面撑爆
const MAX_FINDINGS: usize = 300;

/// 一条本地规则。`id` 必须能在清单里找到，严重级别和修复建议都从清单取。
struct Rule {
    id: &'static str,
    pattern: &'static str,
    /// 只在这些后缀上跑；空表示所有文本文件
    exts: &'static [&'static str],
    /// 只在这些文件名上跑（Dockerfile 这类没有后缀的）
    names: &'static [&'static str],
    /// 命中后补一句具体说明
    note: &'static str,
    /// 行里出现这些词就放过——比如已经带了 noopener 的 target=_blank
    unless: &'static [&'static str],
}

const CODE: &[&str] = &[
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
    "scala",
    "sh",
    "bash",
    "sql",
    "vue",
    "svelte",
    "astro",
    "erb",
    "html",
    "htm",
    "ejs",
    "jinja",
    "j2",
    "twig",
    "yml",
    "yaml",
    "toml",
    "ini",
    "conf",
    "env",
    "properties",
    "xml",
    "tf",
    "tfvars",
];

/// 规则表。加规则时先问一句：这条会不会在正常代码上误报？
const RULES: &[Rule] = &[
    Rule {
        id: "CONF-01",
        pattern: r#"(?i)\b(api[_-]?key|secret[_-]?key|access[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret|password|passwd)\b\s*[:=]\s*["'][^"'\s]{8,}["']"#,
        exts: CODE,
        names: &[],
        note: "密钥/口令写死在源码里，一旦提交就会永久留在 git 历史中",
        unless: &[],
    },
    Rule {
        id: "AUTH-05",
        pattern: r#"(?i)\b(md5|sha1)\s*\(""#,
        exts: CODE,
        names: &[],
        note: "MD5/SHA1 已不适合用于口令或签名",
        unless: &[],
    },
    Rule {
        id: "CRYPTO-02",
        pattern: r#"(?i)(aes[-_]?(128|192|256)[-_]?ecb|aes\.mode_ecb|MODE_ECB|"AES/ECB)"#,
        exts: CODE,
        names: &[],
        note: "ECB 模式相同明文产生相同密文，会泄露结构信息",
        unless: &[],
    },
    Rule {
        id: "CRYPTO-07",
        pattern: r#"(?i)\b(math\.random|random\.random|rand::random|new Random\(\))"#,
        exts: CODE,
        names: &[],
        note: "非密码学随机数，不能用来生成 token、验证码或密钥",
        unless: &[],
    },
    Rule {
        id: "INJ-09",
        pattern: r#"(?i)shell\s*=\s*True"#,
        exts: CODE,
        names: &[],
        note: "shell=True 会让参数里的 `;` `$()` 被解释执行，构成命令注入",
        unless: &[],
    },
    Rule {
        id: "INJ-08",
        pattern: r#"(?i)\b(os\.system|os\.popen|subprocess\.(call|run|Popen|check_output))\s*\([^)]*(\+|%|\{|\$\{)"#,
        exts: CODE,
        names: &[],
        note: "命令串里拼了变量或格式化，用户输入会变成 shell 指令",
        unless: &[],
    },
    Rule {
        id: "INJ-27",
        pattern: r#"(?i)(^|[^.\w])eval\s*\(|new\s+Function\s*\(|Runtime\.getRuntime\(\)\.exec"#,
        exts: CODE,
        names: &[],
        note: "把字符串当代码执行，输入可控时等同于任意代码执行",
        unless: &[],
    },
    Rule {
        id: "INJ-28",
        pattern: r#"(?i)(pickle\.loads?|yaml\.load\s*\(|readObject\s*\(|unserialize\s*\()"#,
        exts: CODE,
        names: &[],
        note: "反序列化不可信数据可导致任意代码执行",
        unless: &[],
    },
    Rule {
        id: "INJ-16",
        pattern: r#"(dangerouslySetInnerHTML|\.innerHTML\s*=|document\.write\s*\()"#,
        exts: CODE,
        names: &[],
        note: "直接写入 HTML，用户输入会变成 XSS",
        unless: &[],
    },
    Rule {
        id: "INJ-01",
        pattern: r#"(?i)["']\s*(select|insert\s+into|update|delete\s+from)\b[^"']*["']\s*(\+|%|\.format\(|\$\{)"#,
        exts: CODE,
        names: &[],
        note: "SQL 语句用字符串拼接，应改为参数化查询",
        unless: &[],
    },
    Rule {
        id: "INJ-29",
        pattern: r#"(?i)(resolve_entities|external[-_]entities|DocumentBuilderFactory|XMLParser\(|libxml_disable_entity_loader)"#,
        exts: CODE,
        names: &[],
        note: "XML 解析器要显式禁用外部实体，否则可被 XXE 读取本地文件",
        unless: &[],
    },
    Rule {
        id: "NET-13",
        pattern: r#"(?i)(verify\s*=\s*False|rejectUnauthorized\s*:\s*false|InsecureSkipVerify\s*:\s*true|CURLOPT_SSL_VERIFYPEER\s*,\s*0)"#,
        exts: CODE,
        names: &[],
        note: "关闭了 TLS 证书校验，中间人可以任意篡改流量",
        unless: &[],
    },
    Rule {
        id: "NET-07",
        pattern: r#"(?i)(access-control-allow-origin["']?\s*[:,]\s*["']?\*|allow_origins\s*=\s*\[\s*["']\*)"#,
        exts: CODE,
        names: &[],
        note: "CORS 允许任意来源，配合 Cookie 凭据即可跨站读取数据",
        unless: &[],
    },
    Rule {
        id: "CONF-08",
        pattern: r#"(?i)^\s*(DEBUG|debug)\s*[:=]\s*(True|true|1)\s*,?\s*$"#,
        exts: &[
            "py", "js", "ts", "yaml", "yml", "toml", "conf", "ini", "env",
        ],
        names: &[],
        note: "调试模式上线会泄露堆栈、配置和内部路径",
        unless: &[],
    },
    Rule {
        id: "SESS-07",
        pattern: r#"(?i)localStorage\.setItem\s*\(\s*["'][^"']*(token|jwt|session|auth)"#,
        exts: &["js", "jsx", "ts", "tsx", "vue", "svelte"],
        names: &[],
        note: "token 存在 localStorage 里，任何 XSS 都能直接读走",
        unless: &[],
    },
    Rule {
        id: "FE-06",
        pattern: r#"target\s*=\s*["']_blank["']"#,
        exts: &["html", "htm", "jsx", "tsx", "vue", "svelte"],
        names: &[],
        note: "新窗口打开外链却没带 rel=noopener，对方页面可以操作本页",
        unless: &["noopener", "noreferrer"],
    },
    Rule {
        id: "QUAL-03",
        pattern: r#"(?i)catch\s*(\([^)]*\))?\s*\{\s*\}|except[^:]*:\s*pass\s*$"#,
        exts: CODE,
        names: &[],
        note: "空 catch 会吞掉异常，故障时既没有日志也没有失败信号",
        unless: &[],
    },
    Rule {
        id: "FILE-04",
        pattern: r#"(?i)(os\.path\.join|path\.join|filepath\.Join)\([^)]*(filename|file_name|originalname|upload\.name)"#,
        exts: CODE,
        names: &[],
        note: "用用户提供的文件名拼路径，`../../` 可以写到目录外",
        unless: &[],
    },
    Rule {
        id: "CONF-18",
        pattern: r"(?i)^\s*USER\s+(root|0)\s*$",
        exts: &[],
        names: &["Dockerfile", "dockerfile", "Containerfile"],
        note: "容器以 root 运行，逃逸后直接拿到宿主机高权限",
        unless: &[],
    },
    Rule {
        id: "CONF-19",
        pattern: r"(?i)(privileged\s*:\s*true|/var/run/docker\.sock|--privileged)",
        exts: &["yml", "yaml"],
        names: &[
            "docker-compose.yml",
            "docker-compose.yaml",
            "compose.yml",
            "compose.yaml",
        ],
        note: "特权容器或挂载 docker.sock，等于把宿主机交给容器",
        unless: &[],
    },
];

/// 行内标注：带这个标记的行不报（和 gitleaks:allow / nosec 一个意思）
const IGNORE_LINE: &str = "scan:ignore";
/// 文件级标注：前 10 行内出现就不扫这个文件（要给文件头的注释留出空间）。
const IGNORE_FILE: &str = "scan:ignore-file";

/// 命中这些词的行不报——它们是占位符或从环境变量读，不是真密钥
const SECRET_ALLOW: &[&str] = &[
    "example",
    "placeholder",
    "changeme",
    "change_me",
    "your_",
    "your-",
    "<your",
    "xxxx",
    "****",
    "os.getenv",
    "os.environ",
    "process.env",
    "env::var",
    "${",
    "{{",
    "getenv(",
    "System.getenv",
];

struct Compiled {
    rule: &'static Rule,
    regex: Regex,
}

fn compiled() -> &'static [Compiled] {
    static COMPILED: OnceLock<Vec<Compiled>> = OnceLock::new();
    COMPILED.get_or_init(|| {
        RULES
            .iter()
            .filter_map(|rule| {
                Regex::new(rule.pattern)
                    .ok()
                    .map(|regex| Compiled { rule, regex })
            })
            .collect()
    })
}

/// 本地规则覆盖的清单编号。第二段据此跳过——已经判过的不必再花 token 问一遍。
pub fn covered_ids() -> Vec<&'static str> {
    RULES.iter().map(|r| r.id).collect()
}

/// 一次本地扫描的结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalReport {
    pub findings: Vec<Finding>,
    pub files_scanned: usize,
    /// 因为太大或不是文本而跳过的文件
    pub files_skipped: usize,
    /// 命中了规则的文件数（用于「扫了 N 个文件，M 个有问题」）
    pub files_with_hits: usize,
    /// 是否因为 `MAX_FINDINGS` 提前截断
    pub truncated: bool,
    /// 本地规则覆盖了多少条清单（让用户知道这次没盖住全部）
    pub rules: usize,
    /// 清单总条数
    pub checklist_total: usize,
}

impl LocalReport {
    /// 后台任务被取消时用的空结果
    pub fn empty() -> Self {
        Self {
            findings: Vec::new(),
            files_scanned: 0,
            files_skipped: 0,
            files_with_hits: 0,
            truncated: false,
            rules: 0,
            checklist_total: 0,
        }
    }
}

/// 扫描一个目录。只读，不改任何文件。
pub fn scan_local(root: &Path, checklist: &VulnChecklist) -> LocalReport {
    let rules = compiled();
    let mut findings: Vec<Finding> = Vec::new();
    let mut files_scanned = 0usize;
    let mut files_skipped = 0usize;
    let mut files_with_hits = 0usize;
    let mut truncated = false;

    let walker = WalkBuilder::new(root)
        .hidden(false) // .env 这类点文件恰恰是要查的
        .require_git(false) // 不是 git 仓库也读 .gitignore
        .git_ignore(true)
        .git_global(false)
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
            )
        })
        .build();

    for entry in walker.flatten() {
        if findings.len() >= MAX_FINDINGS {
            truncated = true;
            break;
        }
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        // 文件级规则先跑：这类问题不看内容，只看「这个文件在不在」
        if let Some(f) = file_level(path, root, checklist, &mut findings) {
            let _ = f;
        }
        let Ok(text) = read_text(path) else {
            files_skipped += 1;
            continue;
        };
        // 文件自己声明了「别扫我」：规则表、测试夹具这类文件，
        // 里面的正则和示例代码会把自己匹配一遍
        if text.lines().take(10).any(|l| l.contains(IGNORE_FILE)) {
            files_skipped += 1;
            continue;
        }
        files_scanned += 1;

        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let before = findings.len();

        for c in rules {
            if !applies(c.rule, name, &ext) {
                continue;
            }
            let Some(item) = checklist.get(c.rule.id) else {
                continue; // 清单里没有这条，宁可跳过也不报一个没有依据的问题
            };
            let mut hits = 0;
            for (idx, line) in text.lines().enumerate() {
                if hits >= PER_RULE_PER_FILE || findings.len() >= MAX_FINDINGS {
                    break;
                }
                if !c.regex.is_match(line)
                    || looks_like_placeholder(line)
                    || line.contains(IGNORE_LINE)
                {
                    continue;
                }
                let lower = line.to_ascii_lowercase();
                if c.rule.unless.iter().any(|w| lower.contains(w)) {
                    continue;
                }
                hits += 1;
                findings.push(make_finding(
                    findings.len(),
                    item.severity,
                    item,
                    c.rule.note,
                    root,
                    path,
                    (idx + 1) as u32,
                ));
            }
        }
        if findings.len() > before {
            files_with_hits += 1;
        }
    }

    LocalReport {
        findings,
        files_scanned,
        files_skipped,
        files_with_hits,
        truncated,
        rules: rules.len(),
        checklist_total: checklist.len(),
    }
}

fn applies(rule: &Rule, name: &str, ext: &str) -> bool {
    if !rule.names.is_empty() {
        return rule.names.iter().any(|n| n.eq_ignore_ascii_case(name));
    }
    rule.exts.is_empty() || rule.exts.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// 占位符和「从环境变量读」不是密钥，别报
fn looks_like_placeholder(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    SECRET_ALLOW
        .iter()
        .any(|w| lower.contains(&w.to_ascii_lowercase()))
}

/// 文件级规则：只看文件是否存在。返回是否命中（目前只用于统计）。
fn file_level(
    path: &Path,
    root: &Path,
    checklist: &VulnChecklist,
    findings: &mut Vec<Finding>,
) -> Option<()> {
    let name = path.file_name().and_then(|s| s.to_str())?;
    let rel = path.strip_prefix(root).unwrap_or(path);

    // CONF-02：.env 进了仓库
    if name == ".env" || (name.starts_with(".env.") && name != ".env.example") {
        if let Some(item) = checklist.get("CONF-02") {
            findings.push(make_finding(
                findings.len(),
                item.severity,
                item,
                "环境变量文件被提交进了仓库，里面通常含真实密钥",
                root,
                path,
                1,
            ));
        }
    }
    // CONF-22：备份 / 版本控制残留暴露
    let backup = [".bak", ".old", ".swp", ".sql", ".zip", ".tar", ".gz"]
        .iter()
        .any(|s| name.to_ascii_lowercase().ends_with(s));
    if backup && !rel.starts_with("assets") {
        if let Some(item) = checklist.get("CONF-22") {
            findings.push(make_finding(
                findings.len(),
                item.severity,
                item,
                "备份或打包文件留在项目里，部署后往往能被直接下载",
                root,
                path,
                1,
            ));
        }
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn make_finding(
    seq: usize,
    severity: Severity,
    item: &crate::vuln_checklist::VulnItem,
    note: &str,
    root: &Path,
    path: &Path,
    line: u32,
) -> Finding {
    Finding {
        id: format!("VG-{:03}", seq + 1),
        severity,
        title: item.title.clone(),
        cwe: item.cwe.clone(),
        file: path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string(),
        line,
        description: format!("[{}] {note}", item.id),
        suggestion: item.fix_point.clone(),
    }
}

/// 读成文本；二进制、超大、非 UTF-8 都返回 Err（由调用方计入 skipped）
fn read_text(path: &Path) -> Result<String, ()> {
    let meta = std::fs::metadata(path).map_err(|_| ())?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(());
    }
    let bytes = std::fs::read(path).map_err(|_| ())?;
    // 前 8KB 里有 NUL 就按二进制处理
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Err(());
    }
    String::from_utf8(bytes).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn scan(files: &[(&str, &str)]) -> (tempfile::TempDir, LocalReport) {
        let dir = tempdir().unwrap();
        for (name, body) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, body).unwrap();
        }
        let report = scan_local(dir.path(), &VulnChecklist::new());
        (dir, report)
    }

    fn ids(report: &LocalReport) -> Vec<&str> {
        report
            .findings
            .iter()
            .map(|f| {
                f.description
                    .split(']')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('[')
            })
            .collect()
    }

    #[test]
    fn finds_a_hardcoded_secret_and_points_at_the_line() {
        let (_d, r) = scan(&[(
            "app.py",
            "import os\nAPI_KEY = \"sk-live-9f8a7b6c5d4e3f2a\"\nprint(1)\n",
        )]);
        assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
        let f = &r.findings[0];
        assert_eq!(f.file, "app.py");
        assert_eq!(f.line, 2);
        assert_eq!(f.severity, Severity::Critical);
        assert!(f.description.contains("CONF-01"));
        assert_eq!(f.cwe.as_deref(), Some("CWE-798"));
        assert!(!f.suggestion.is_empty(), "修复建议来自清单");
    }

    #[test]
    fn placeholders_and_env_lookups_are_not_reported() {
        let (_d, r) = scan(&[(
            "config.py",
            "API_KEY = \"your-api-key-here\"\nPASSWORD = os.getenv(\"DB_PASSWORD\")\nSECRET_KEY = \"${SECRET}\"\nTOKEN = \"changeme123456\"\n",
        )]);
        assert!(r.findings.is_empty(), "{:?}", r.findings);
    }

    #[test]
    fn catches_shell_true_eval_and_ecb() {
        let (_d, r) = scan(&[(
            "run.py",
            "import subprocess\nsubprocess.run(cmd, shell=True)\neval(user_input)\ncipher = AES.new(k, AES.MODE_ECB)\n",
        )]);
        let found = ids(&r);
        assert!(found.contains(&"INJ-09"), "{found:?}");
        assert!(found.contains(&"INJ-27"), "{found:?}");
        assert!(found.contains(&"CRYPTO-02"), "{found:?}");
    }

    #[test]
    fn an_env_file_in_the_repo_is_reported_even_if_empty() {
        let (_d, r) = scan(&[(".env", "DB_PASSWORD=real-secret-value\n")]);
        assert!(
            ids(&r).contains(&"CONF-02"),
            "点文件不该被 hidden 规则跳过：{:?}",
            r.findings
        );
    }

    #[test]
    fn dockerfile_running_as_root_is_reported() {
        let (_d, r) = scan(&[(
            "Dockerfile",
            "FROM python:3.12\nUSER root\nCMD [\"python\"]\n",
        )]);
        assert!(ids(&r).contains(&"CONF-18"), "{:?}", r.findings);
    }

    #[test]
    fn same_rule_stops_at_three_hits_per_file() {
        let body: String = (0..10).map(|_| "eval(x)\n").collect();
        let (_d, r) = scan(&[("a.py", &body)]);
        assert_eq!(r.findings.len(), PER_RULE_PER_FILE);
    }

    #[test]
    fn gitignored_and_dependency_directories_are_skipped() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "secret.py\n").unwrap();
        fs::write(dir.path().join("secret.py"), "eval(x)\n").unwrap();
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        fs::write(dir.path().join("node_modules/a.js"), "eval(x)\n").unwrap();
        fs::write(dir.path().join("app.py"), "eval(x)\n").unwrap();

        let r = scan_local(dir.path(), &VulnChecklist::new());
        let files: Vec<&str> = r.findings.iter().map(|f| f.file.as_str()).collect();
        assert_eq!(files, ["app.py"], "被忽略的和依赖目录都不该扫");
    }

    #[test]
    fn binary_files_are_counted_as_skipped() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("img.png"),
            [0x89, 0x50, 0x4e, 0x47, 0, 0, 0],
        )
        .unwrap();
        fs::write(dir.path().join("a.py"), "print(1)\n").unwrap();
        let r = scan_local(dir.path(), &VulnChecklist::new());
        assert_eq!(r.files_scanned, 1);
        assert_eq!(r.files_skipped, 1);
    }

    #[test]
    fn an_inline_marker_silences_that_line_only() {
        let (_d, r) = scan(&[("a.py", "eval(x)  # scan:ignore\neval(y)\n")]);
        assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
        assert_eq!(r.findings[0].line, 2);
    }

    #[test]
    fn a_file_that_declares_itself_exempt_is_not_scanned() {
        let (_d, r) = scan(&[(
            "rules.py",
            "# scan:ignore-file\nRULES = [\"eval(\", \"shell=True\"]\n",
        )]);
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        assert_eq!(r.files_skipped, 1);
        assert_eq!(r.files_scanned, 0);
    }

    #[test]
    fn every_rule_points_at_an_item_that_exists_in_the_checklist() {
        let list = VulnChecklist::new();
        for rule in RULES {
            assert!(
                list.get(rule.id).is_some(),
                "规则 {} 在清单里不存在",
                rule.id
            );
        }
        // 正则本身必须能编译
        assert_eq!(compiled().len(), RULES.len(), "有正则编译失败");
    }
}
