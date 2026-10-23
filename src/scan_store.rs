//! 扫描结果的持久化。
//!
//! 一次扫描存一个 JSON，放在配置目录旁的 `scans/`（和 `sessions/` 一个思路）。
//! 除了结果本身，还记下**每个涉及文件的指纹**——文件在扫描后改过，那条发现就
//! 不再可信，宁可跳过也不要拿着旧行号去改新代码。
//!
//! 指纹用的是标准库的 `DefaultHasher`，不是密码学哈希：它只用来回答「这个文件
//! 还是不是扫描时那个」，不用于任何安全判定。

use crate::finding::Finding;
use crate::scan_ai::Judgement;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

/// 每个根目录保留多少次扫描，超出的按时间清掉
const KEEP_PER_ROOT: usize = 10;

/// 一个文件在扫描时的样子
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub len: u64,
    /// 内容哈希（非密码学）
    pub hash: u64,
}

impl Fingerprint {
    pub fn of(path: &Path) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        Some(Self {
            len: bytes.len() as u64,
            hash: hasher.finish(),
        })
    }
}

/// 一次扫描的完整存档
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredScan {
    /// 文件名主干：`<根目录哈希>-<时间戳>`
    #[serde(skip)]
    pub id: String,
    /// 扫描的目录（绝对路径）
    pub root: String,
    /// Unix 秒
    pub created: i64,
    pub version: String,
    pub files_scanned: usize,
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub judgements: Vec<Judgement>,
    #[serde(default)]
    pub failed_batches: Vec<String>,
    /// 相对路径 → 指纹
    #[serde(default)]
    pub fingerprints: BTreeMap<String, Fingerprint>,
    /// 这次判定所基于的仓库快照指纹（增量扫描靠它判断「有没有变」）
    #[serde(default)]
    pub digest_hash: u64,
    /// 参与判定的每个文件的内容哈希
    #[serde(default)]
    pub digest_files: BTreeMap<String, u64>,
}

impl StoredScan {
    /// 哪些文件在扫描之后改过（或没了）。返回相对路径。
    pub fn stale_files(&self, root: &Path) -> Vec<String> {
        let mut stale = Vec::new();
        for (rel, then) in &self.fingerprints {
            match Fingerprint::of(&root.join(rel)) {
                Some(now) if now == *then => {}
                // 变了或者读不出来了，都算过期：宁可让用户重扫一次
                _ => stale.push(rel.clone()),
            }
        }
        stale
    }
}

/// 这条发现所在的文件还是不是扫描时那个。
/// 没有指纹（比如存档是手改的）一律按「不新鲜」处理——不敢替用户打包票。
pub fn is_fresh(root: &Path, prints: &BTreeMap<String, Fingerprint>, finding: &Finding) -> bool {
    let Some(then) = prints.get(&finding.file) else {
        return false;
    };
    Fingerprint::of(&root.join(&finding.file)) == Some(*then)
}

/// `scans/` 目录
pub struct ScanStore {
    dir: PathBuf,
}

impl ScanStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// 和对话记录一样，放在配置文件旁边
    pub fn beside_config(config_path: &Path) -> Self {
        let base = config_path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        Self::new(base.join("scans"))
    }

    fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// 存档并顺手清理旧记录，返回写到哪里了
    pub fn save(&self, scan: &StoredScan) -> Result<PathBuf> {
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("创建扫描目录失败：{}", self.dir.display()))?;
        let path = self.file(&scan.id);
        let text = serde_json::to_string_pretty(scan).context("序列化扫描结果失败")?;
        let tmp = self.dir.join(format!("{}.json.tmp", scan.id));
        fs::write(&tmp, text).with_context(|| format!("写入失败：{}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("保存失败：{}", path.display()))?;
        let _ = self.prune(Path::new(&scan.root), KEEP_PER_ROOT);
        Ok(path)
    }

    /// 某个目录最近一次的扫描
    pub fn latest_for(&self, root: &Path) -> Result<Option<StoredScan>> {
        let prefix = format!("{:016x}-", hash_str(&abs(root)));
        let mut newest: Option<(String, StoredScan)> = None;
        for entry in self.entries()? {
            let (id, scan) = entry;
            if !id.starts_with(&prefix) {
                continue;
            }
            if newest
                .as_ref()
                .is_none_or(|(_, best)| scan.created > best.created)
            {
                newest = Some((id, scan));
            }
        }
        Ok(newest.map(|(id, mut scan)| {
            scan.id = id;
            scan
        }))
    }

    /// 所有存档，最近的在前面
    pub fn list(&self) -> Result<Vec<StoredScan>> {
        let mut all: Vec<StoredScan> = self
            .entries()?
            .into_iter()
            .map(|(id, mut scan)| {
                scan.id = id;
                scan
            })
            .collect();
        all.sort_by_key(|s| std::cmp::Reverse(s.created));
        Ok(all)
    }

    /// 每个根目录只留最近 `keep` 次
    pub fn prune(&self, root: &Path, keep: usize) -> Result<usize> {
        let prefix = format!("{:016x}-", hash_str(&abs(root)));
        let mut mine: Vec<(String, i64)> = self
            .entries()?
            .into_iter()
            .filter(|(id, _)| id.starts_with(&prefix))
            .map(|(id, scan)| (id, scan.created))
            .collect();
        mine.sort_by_key(|(_, created)| std::cmp::Reverse(*created));
        let mut removed = 0;
        for (id, _) in mine.into_iter().skip(keep) {
            if fs::remove_file(self.file(&id)).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn entries(&self) -> Result<Vec<(String, StoredScan)>> {
        let dir = match fs::read_dir(&self.dir) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).context("读取扫描目录失败"),
        };
        let mut out = Vec::new();
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            // 坏文件跳过，不要因为一条存档读不出来就让 /findings 整个失败
            if let Ok(scan) = serde_json::from_str::<StoredScan>(&text) {
                out.push((id.to_string(), scan));
            }
        }
        Ok(out)
    }
}

/// 生成一次扫描的编号：根目录哈希 + 毫秒时间戳
pub fn new_id(root: &Path, now_ms: i64) -> String {
    format!("{:016x}-{:013}", hash_str(&abs(root)), now_ms)
}

fn abs(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn hash_str(s: &Path) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::Severity;
    use tempfile::tempdir;

    fn finding(file: &str, line: u32) -> Finding {
        Finding {
            id: "VG-001".into(),
            severity: Severity::High,
            title: "硬编码密钥".into(),
            cwe: Some("CWE-798".into()),
            file: file.into(),
            line,
            description: "[CONF-01]".into(),
            suggestion: "改环境变量".into(),
        }
    }

    fn scan(root: &Path, created: i64, findings: Vec<Finding>) -> StoredScan {
        let mut fingerprints = BTreeMap::new();
        for f in &findings {
            if let Some(fp) = Fingerprint::of(&root.join(&f.file)) {
                fingerprints.insert(f.file.clone(), fp);
            }
        }
        StoredScan {
            id: new_id(root, created * 1000),
            root: root.to_string_lossy().to_string(),
            created,
            version: "0.1.0".into(),
            files_scanned: 1,
            findings,
            judgements: Vec::new(),
            failed_batches: Vec::new(),
            fingerprints,
            digest_hash: 0,
            digest_files: BTreeMap::new(),
        }
    }

    #[test]
    fn save_then_load_the_latest_for_that_root() {
        let cfg = tempdir().unwrap();
        let repo = tempdir().unwrap();
        fs::write(repo.path().join("a.py"), "eval(x)\n").unwrap();
        let store = ScanStore::beside_config(&cfg.path().join("config.toml"));

        let older = scan(repo.path(), 100, vec![finding("a.py", 1)]);
        store.save(&older).unwrap();
        let newer = scan(repo.path(), 200, vec![finding("a.py", 1)]);
        store.save(&newer).unwrap();

        let got = store.latest_for(repo.path()).unwrap().expect("latest");
        assert_eq!(got.created, 200);
        assert_eq!(got.findings.len(), 1);
    }

    #[test]
    fn a_different_root_does_not_return_someone_elses_scan() {
        let cfg = tempdir().unwrap();
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let store = ScanStore::beside_config(&cfg.path().join("config.toml"));
        store.save(&scan(a.path(), 100, Vec::new())).unwrap();
        assert!(store.latest_for(b.path()).unwrap().is_none());
    }

    #[test]
    fn a_file_that_changed_after_the_scan_is_reported_stale() {
        let repo = tempdir().unwrap();
        fs::write(repo.path().join("a.py"), "eval(x)\n").unwrap();
        let s = scan(repo.path(), 100, vec![finding("a.py", 1)]);
        assert!(s.stale_files(repo.path()).is_empty(), "刚扫完不该过期");
        assert!(is_fresh(repo.path(), &s.fingerprints, &finding("a.py", 1)));

        fs::write(repo.path().join("a.py"), "eval(y)\n").unwrap();
        assert_eq!(s.stale_files(repo.path()), vec!["a.py".to_string()]);
        assert!(!is_fresh(repo.path(), &s.fingerprints, &finding("a.py", 1)));
    }

    #[test]
    fn a_deleted_file_is_stale_too() {
        let repo = tempdir().unwrap();
        fs::write(repo.path().join("gone.py"), "eval(x)\n").unwrap();
        let s = scan(repo.path(), 100, vec![finding("gone.py", 1)]);
        fs::remove_file(repo.path().join("gone.py")).unwrap();
        assert_eq!(s.stale_files(repo.path()), vec!["gone.py".to_string()]);
    }

    #[test]
    fn a_finding_without_a_fingerprint_is_treated_as_stale() {
        let repo = tempdir().unwrap();
        fs::write(repo.path().join("a.py"), "x\n").unwrap();
        let s = scan(repo.path(), 100, Vec::new());
        // 没有指纹的发现（比如手改过的存档）不能假定它是新鲜的
        assert!(!is_fresh(repo.path(), &s.fingerprints, &finding("a.py", 1)));
    }

    #[test]
    fn keeps_only_the_latest_n_per_root() {
        let cfg = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let store = ScanStore::beside_config(&cfg.path().join("config.toml"));
        for i in 0..KEEP_PER_ROOT + 3 {
            store
                .save(&scan(repo.path(), 100 + i as i64, Vec::new()))
                .unwrap();
        }
        assert_eq!(store.list().unwrap().len(), KEEP_PER_ROOT);
        // 留下的应该是最新的那些
        let newest = store.latest_for(repo.path()).unwrap().unwrap();
        assert_eq!(newest.created, 100 + (KEEP_PER_ROOT + 2) as i64);
    }

    #[test]
    fn a_broken_archive_is_skipped_instead_of_failing_everything() {
        let cfg = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let store = ScanStore::beside_config(&cfg.path().join("config.toml"));
        store.save(&scan(repo.path(), 100, Vec::new())).unwrap();
        fs::write(
            cfg.path().join("scans/deadbeef-0000000000001.json"),
            "{ not json",
        )
        .unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(store.latest_for(repo.path()).unwrap().is_some());
    }
}
