//! 对话的保存、恢复与搜索：每个对话一个 JSON 文件，放在配置目录旁的 `sessions/`。
//! 对话里可能有代码和密钥，所以文件权限与配置文件一样是 0600。

use crate::config::write_private;
use crate::i18n::{pick, tr};
use crate::llm::{Message, Role};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const TITLE_CHARS: usize = 60;
const SNIPPET_BEFORE: usize = 20;
const SNIPPET_AFTER: usize = 50;
/// 选择面板里能搜到的正文长度上限（每个对话）。更长的尾巴只能靠标题找。
const BODY_CHARS: usize = 20_000;
/// 所有对话正文加起来的上限：超出之后更早的对话只按标题匹配，
/// 免得几百个长对话把内存吃光。
const BODY_BUDGET: usize = 2_000_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    /// Unix 秒
    pub created: i64,
    pub updated: i64,
    pub title: String,
    pub model: String,
    /// 只含 user / assistant，system 提示词每次请求时现加
    pub messages: Vec<Message>,
}

/// 列表里展示用的摘要
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub updated: i64,
    pub turns: usize,
    pub model: String,
}

/// 选择面板用的一行：摘要 + 正文开头一段，打开面板时一次性读进内存
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogEntry {
    pub summary: Summary,
    /// 所有消息正文的开头一段；超出全局预算的对话这里是空的（只按标题匹配）
    pub body: String,
}

impl Conversation {
    pub fn new(id: String, now: i64, model: &str) -> Self {
        Self {
            id,
            created: now,
            updated: now,
            title: String::new(),
            model: model.to_string(),
            messages: Vec::new(),
        }
    }

    /// 追加一轮问答；第一轮的提问用作标题
    pub fn push_exchange(&mut self, user: &str, assistant: &str, now: i64) {
        if self.title.is_empty() {
            self.title = make_title(user);
        }
        self.messages.push(Message::user(user));
        self.messages.push(Message::assistant(assistant));
        self.updated = now;
    }

    pub fn turns(&self) -> usize {
        self.messages
            .iter()
            .filter(|m| m.role == Role::User)
            .count()
    }

    pub fn summary(&self) -> Summary {
        Summary {
            id: self.id.clone(),
            title: self.title.clone(),
            updated: self.updated,
            turns: self.turns(),
            model: self.model.clone(),
        }
    }
}

fn make_title(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut title = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.chars().count() > TITLE_CHARS {
        title = title.chars().take(TITLE_CHARS).collect::<String>() + "…";
    }
    if title.is_empty() {
        tr("（无标题）").to_string()
    } else {
        title
    }
}

/// 编号会拼进文件名，只允许字母、数字、`-` 和 `_`，杜绝 `../` 之类的路径穿越
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// 对话目录放在配置文件旁边：`<配置目录>/sessions`。
    /// 这样 `--config` 指向别处时，对话也跟着隔离。
    pub fn beside_config(config_path: &Path) -> Self {
        let base = config_path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        Self::new(base.join("sessions"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// 按时间生成一个还没被占用的编号
    pub fn fresh_id(&self, now_ms: i64) -> String {
        let base = chrono::DateTime::from_timestamp_millis(now_ms)
            .unwrap_or_default()
            .format("%Y%m%d-%H%M%S-%3f")
            .to_string();
        let mut id = base.clone();
        let mut n = 2;
        while self.file(&id).exists() {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }

    /// 先写临时文件再改名，中途崩溃不会留下写了一半的对话
    pub fn save(&self, c: &Conversation) -> Result<()> {
        if !valid_id(&c.id) {
            bail!(
                "{}",
                pick(
                    format!("非法的对话编号：{}", c.id),
                    format!("Invalid conversation id: {}", c.id)
                )
            );
        }
        fs::create_dir_all(&self.dir).with_context(|| {
            pick(
                format!("创建对话目录失败：{}", self.dir.display()),
                format!(
                    "Creating the conversation directory failed: {}",
                    self.dir.display()
                ),
            )
        })?;
        let text = serde_json::to_string_pretty(c).context(tr("序列化对话失败"))?;
        let path = self.file(&c.id);
        let tmp = self.dir.join(format!("{}.json.tmp", c.id));
        write_private(&tmp, &text).with_context(|| {
            pick(
                format!("写入对话失败：{}", tmp.display()),
                format!("Writing the conversation failed: {}", tmp.display()),
            )
        })?;
        fs::rename(&tmp, &path).with_context(|| {
            pick(
                format!("保存对话失败：{}", path.display()),
                format!("Saving the conversation failed: {}", path.display()),
            )
        })
    }

    pub fn load(&self, id: &str) -> Result<Conversation> {
        if !valid_id(id) {
            bail!(
                "{}",
                pick(
                    format!("非法的对话编号：{id}"),
                    format!("Invalid conversation id: {id}")
                )
            );
        }
        let path = self.file(id);
        let text = fs::read_to_string(&path).with_context(|| {
            pick(
                format!("读取对话失败：{}", path.display()),
                format!("Reading the conversation failed: {}", path.display()),
            )
        })?;
        let mut c: Conversation = serde_json::from_str(&text).with_context(|| {
            pick(
                format!("对话文件格式错误：{}", path.display()),
                format!("Malformed conversation file: {}", path.display()),
            )
        })?;
        // 以文件名为准，避免手动改名后保存到别的文件
        c.id = id.to_string();
        Ok(c)
    }

    fn read_all(&self) -> Result<(Vec<Conversation>, usize)> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
            Err(e) => {
                return Err(e).with_context(|| {
                    pick(
                        format!("读取对话目录失败：{}", self.dir.display()),
                        format!(
                            "Reading the conversation directory failed: {}",
                            self.dir.display()
                        ),
                    )
                });
            }
        };
        let mut all = Vec::new();
        let mut skipped = 0;
        for entry in entries {
            let Ok(entry) = entry else {
                skipped += 1;
                continue;
            };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            match self.load(id) {
                Ok(c) => all.push(c),
                Err(_) => skipped += 1,
            }
        }
        Ok((all, skipped))
    }

    /// 全部对话，最近更新的在前；还没有任何消息的对话不列出。
    /// 只要摘要：启动时就要调，别顺手把正文也读进内存（那是 `catalog` 的事）。
    pub fn list(&self) -> Result<Vec<Summary>> {
        let (all, _) = self.read_all()?;
        Ok(summaries(&all))
    }

    /// 最近更新的对话，可以排除当前这一个
    pub fn latest(&self, exclude: Option<&str>) -> Result<Option<Conversation>> {
        match self
            .list()?
            .into_iter()
            .find(|s| Some(s.id.as_str()) != exclude)
        {
            Some(s) => self.load(&s.id).map(Some),
            None => Ok(None),
        }
    }

    /// 列表 + 正文开头，供选择面板在内存里即时过滤。
    /// 只读一遍磁盘；正文按 `BODY_CHARS` 和 `BODY_BUDGET` 截断。
    pub fn catalog(&self) -> Result<(Vec<CatalogEntry>, usize)> {
        let (all, skipped) = self.read_all()?;
        let mut ordered: Vec<&Conversation> =
            all.iter().filter(|c| !c.messages.is_empty()).collect();
        ordered.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));

        let mut budget = BODY_BUDGET;
        let entries = ordered
            .into_iter()
            .map(|c| CatalogEntry {
                summary: c.summary(),
                body: body_of(c, &mut budget),
            })
            .collect();
        Ok((entries, skipped))
    }

    /// 删掉一个对话。找不到时报错，让界面能说清楚是没删还是删错了。
    pub fn delete(&self, id: &str) -> Result<()> {
        if !valid_id(id) {
            bail!(
                "{}",
                pick(
                    format!("非法的对话编号：{id}"),
                    format!("Invalid conversation id: {id}")
                )
            );
        }
        let path = self.file(id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                bail!(
                    "{}",
                    pick(
                        format!("对话已经不存在：{}", path.display()),
                        format!("The conversation no longer exists: {}", path.display())
                    )
                )
            }
            Err(e) => Err(e).with_context(|| {
                pick(
                    format!("删除对话失败：{}", path.display()),
                    format!("Deleting the conversation failed: {}", path.display()),
                )
            }),
        }
    }
}

/// 摘要列表，最近更新的在前
fn summaries(all: &[Conversation]) -> Vec<Summary> {
    let mut items: Vec<Summary> = all
        .iter()
        .filter(|c| !c.messages.is_empty())
        .map(Conversation::summary)
        .collect();
    items.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));
    items
}

/// 取对话正文的开头一段，并从预算里扣掉。预算用完就返回空串。
fn body_of(c: &Conversation, budget: &mut usize) -> String {
    let cap = BODY_CHARS.min(*budget);
    if cap == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (i, m) in c.messages.iter().enumerate() {
        if out.chars().count() >= cap {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        // 第一句已经变成标题了，跳过标题长度那一段：
        // 否则搜到标题里的词时，摘要只是把标题又念了一遍
        let skip = if i == 0 { TITLE_CHARS } else { 0 };
        out.extend(m.content.chars().skip(skip).take(cap));
    }
    let out: String = out.chars().take(cap).collect();
    *budget -= out.chars().count();
    out
}

/// 逐字符转小写，保证字符数不变，命中下标可以直接用在原文上
pub(crate) fn fold(s: &str) -> Vec<char> {
    s.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}

pub(crate) fn find_folded(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// 命中处前后各取一段，换行压成空格，被截断的一侧加省略号
pub(crate) fn snippet(text: &str, at: usize, len: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let start = at.saturating_sub(SNIPPET_BEFORE);
    let end = (at + len + SNIPPET_AFTER).min(chars.len());
    let body: String = chars[start..end].iter().collect();
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        body,
        if end < chars.len() { "…" } else { "" }
    )
}

/// 相对时间，一个月以前的显示日期
pub fn format_ago(ts: i64, now: i64) -> String {
    let secs = now - ts;
    match secs {
        i64::MIN..=59 => tr("刚刚").to_string(),
        60..=3599 => pick(
            format!("{} 分钟前", secs / 60),
            format!("{} minutes ago", secs / 60),
        ),
        3600..=86399 => pick(
            format!("{} 小时前", secs / 3600),
            format!("{} hours ago", secs / 3600),
        ),
        86400..=2_591_999 => pick(
            format!("{} 天前", secs / 86400),
            format!("{} days ago", secs / 86400),
        ),
        _ => chrono::DateTime::from_timestamp(ts, 0)
            .map(|d| {
                d.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(id: &str, updated: i64, user: &str, assistant: &str) -> Conversation {
        let mut c = Conversation::new(id.into(), updated, "deepseek-flash");
        c.push_exchange(user, assistant, updated);
        c
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("sessions"));
        (dir, store)
    }

    #[test]
    fn title_uses_first_line_collapses_spaces_and_truncates_by_chars() {
        assert_eq!(make_title("  \n hello   world \nsecond"), "hello world");
        assert_eq!(make_title("   "), "（无标题）");
        let long = "你".repeat(80);
        let t = make_title(&long);
        assert_eq!(t.chars().count(), TITLE_CHARS + 1);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn push_exchange_sets_title_once_and_tracks_turns() {
        let mut c = Conversation::new("a".into(), 10, "m");
        c.push_exchange("第一个问题", "答一", 20);
        c.push_exchange("第二个问题", "答二", 30);
        assert_eq!(c.title, "第一个问题");
        assert_eq!(c.turns(), 2);
        assert_eq!(c.messages.len(), 4);
        assert_eq!((c.created, c.updated), (10, 30));
    }

    #[test]
    fn save_then_load_roundtrips_and_file_is_private() {
        let (_dir, store) = store();
        let c = conv("one", 100, "你好", "世界");
        store.save(&c).unwrap();
        assert_eq!(store.load("one").unwrap(), c);
        assert!(!store.dir().join("one.json.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(store.dir().join("one.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn load_uses_file_name_as_id() {
        let (_dir, store) = store();
        let c = conv("original", 1, "q", "a");
        store.save(&c).unwrap();
        fs::rename(
            store.dir().join("original.json"),
            store.dir().join("renamed.json"),
        )
        .unwrap();
        assert_eq!(store.load("renamed").unwrap().id, "renamed");
    }

    #[test]
    fn ids_cannot_escape_the_directory() {
        let (_dir, store) = store();
        for bad in ["../x", "a/b", "", "a.b", "..", "x\\y"] {
            assert!(store.load(bad).is_err(), "{bad:?}");
        }
        let mut c = conv("ok", 1, "q", "a");
        c.id = "../evil".into();
        assert!(store.save(&c).is_err());
    }

    #[test]
    fn fresh_id_avoids_existing_files() {
        let (_dir, store) = store();
        let first = store.fresh_id(1_800_000_000_123);
        assert_eq!(first, "20270115-080000-123");
        store.save(&conv(&first, 1, "q", "a")).unwrap();
        let second = store.fresh_id(1_800_000_000_123);
        assert_eq!(second, format!("{first}-2"));
        assert!(valid_id(&second));
    }

    #[test]
    fn list_is_newest_first_and_skips_junk() {
        let (_dir, store) = store();
        store.save(&conv("old", 100, "旧的", "a")).unwrap();
        store.save(&conv("new", 300, "新的", "a")).unwrap();
        store.save(&conv("mid", 200, "中间", "a")).unwrap();
        store
            .save(&Conversation::new("empty".into(), 999, "m"))
            .unwrap();
        fs::write(store.dir().join("bad.json"), "not json").unwrap();
        fs::write(store.dir().join("half.json.tmp"), "{").unwrap();
        fs::write(store.dir().join("notes.txt"), "hi").unwrap();

        let listing = store.list().unwrap();
        let ids: Vec<_> = listing.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["new", "mid", "old"]);
        assert_eq!(listing[0].turns, 1);
        // 坏文件算「跳过」，但原样保留
        assert_eq!(store.catalog().unwrap().1, 1);
        assert!(store.dir().join("bad.json").exists());
    }

    #[test]
    fn missing_directory_means_no_conversations() {
        let (_dir, store) = store();
        let listing = store.list().unwrap();
        assert!(listing.is_empty());
        assert!(store.latest(None).unwrap().is_none());
        assert!(store.catalog().unwrap().0.is_empty());
    }

    #[test]
    fn latest_can_exclude_the_current_conversation() {
        let (_dir, store) = store();
        store.save(&conv("a", 100, "q", "a")).unwrap();
        store.save(&conv("b", 200, "q", "a")).unwrap();
        assert_eq!(store.latest(None).unwrap().unwrap().id, "b");
        assert_eq!(store.latest(Some("b")).unwrap().unwrap().id, "a");
        let (_d2, only) = {
            let (d, s) = self::store();
            s.save(&conv("solo", 1, "q", "a")).unwrap();
            (d, s)
        };
        assert!(only.latest(Some("solo")).unwrap().is_none());
    }

    #[test]
    fn catalog_carries_the_body_so_the_panel_can_filter_in_memory() {
        let (_dir, store) = store();
        let mut a = conv("a", 100, "如何防止 SQL 注入？", "使用参数化查询。");
        a.push_exchange("还有别的吗", "没有了", 110);
        store.save(&a).unwrap();
        store.save(&conv("b", 300, "聊聊天气", "今天不错")).unwrap();
        store
            .save(&Conversation::new("empty".into(), 999, "m"))
            .unwrap();

        let (entries, skipped) = store.catalog().unwrap();
        assert_eq!(skipped, 0);
        let ids: Vec<_> = entries.iter().map(|e| e.summary.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"], "最近更新的在前，空对话不列出");
        assert!(entries[1].body.contains("参数化查询"));
        assert!(entries[1].body.contains("还有别的吗"), "多轮都收进来");
        assert!(
            !entries[1].body.contains("如何防止 SQL 注入"),
            "第一句已经是标题了，正文里不该再重复一遍"
        );
        assert_eq!(entries[1].summary.turns, 2);
    }

    #[test]
    fn catalog_bodies_are_capped_per_conversation() {
        let (_dir, store) = store();
        let huge = "长".repeat(BODY_CHARS + 500);
        store.save(&conv("big", 100, &huge, &huge)).unwrap();
        let (entries, _) = store.catalog().unwrap();
        assert_eq!(entries[0].body.chars().count(), BODY_CHARS);
    }

    #[test]
    fn catalog_stops_collecting_bodies_once_the_budget_is_gone() {
        let (_dir, store) = store();
        // 每个对话都顶到单条上限，凑够预算还多一个
        let per = BODY_CHARS;
        let count = BODY_BUDGET / per + 1;
        let text = "x".repeat(per);
        for i in 0..count {
            store
                .save(&conv(&format!("s{i:04}"), 100 + i as i64, &text, &text))
                .unwrap();
        }
        let (entries, _) = store.catalog().unwrap();
        assert_eq!(entries.len(), count);
        assert!(!entries[0].body.is_empty(), "最新的还能搜正文");
        assert!(entries.last().unwrap().body.is_empty(), "预算用完就不带了");
    }

    #[test]
    fn delete_removes_only_that_conversation() {
        let (_dir, store) = store();
        store.save(&conv("keep", 100, "留着", "a")).unwrap();
        store.save(&conv("gone", 200, "删掉", "a")).unwrap();
        store.delete("gone").unwrap();
        assert!(!store.dir().join("gone.json").exists());
        assert!(store.dir().join("keep.json").exists());
        assert_eq!(store.list().unwrap().len(), 1);
        // 再删一次要报错，界面才能说清楚
        assert!(store.delete("gone").is_err());
    }

    #[test]
    fn delete_refuses_ids_that_could_escape_the_directory() {
        let (_dir, store) = store();
        let outside = store.dir().parent().unwrap().join("keep.txt");
        fs::write(&outside, "别动我").unwrap();
        for bad in ["../keep", "a/b", "", "..", "x\\y"] {
            assert!(store.delete(bad).is_err(), "{bad:?}");
        }
        assert!(outside.exists());
    }

    #[test]
    fn snippet_keeps_original_case_and_marks_truncation() {
        let text = format!("{}Needle{}", "a".repeat(30), "b".repeat(80));
        let at = find_folded(&fold(&text), &fold("nEEDLE")).unwrap();
        assert_eq!(at, 30);
        let s = snippet(&text, at, 6);
        assert_eq!(
            s,
            format!(
                "…{}Needle{}…",
                "a".repeat(SNIPPET_BEFORE),
                "b".repeat(SNIPPET_AFTER)
            )
        );
        // 命中在开头且文本很短：两侧都没有省略号
        assert_eq!(snippet("hi there", 0, 2), "hi there");
        // 换行压成空格
        assert_eq!(snippet("a\n\nb  c", 0, 1), "a b c");
    }

    #[test]
    fn fold_keeps_char_count_for_special_casing() {
        let s = "İstanbul 你好";
        assert_eq!(fold(s).len(), s.chars().count());
    }

    #[test]
    fn beside_config_puts_sessions_next_to_the_config_file() {
        let s = Store::beside_config(Path::new("/tmp/x/config.toml"));
        assert_eq!(s.dir(), Path::new("/tmp/x/sessions"));
        let s = Store::beside_config(Path::new("config.toml"));
        assert_eq!(s.dir(), Path::new("./sessions"));
    }

    #[test]
    fn format_ago_buckets() {
        let now = 10_000_000;
        assert_eq!(format_ago(now - 5, now), "刚刚");
        assert_eq!(format_ago(now - 120, now), "2 分钟前");
        assert_eq!(format_ago(now - 7200, now), "2 小时前");
        assert_eq!(format_ago(now - 3 * 86400, now), "3 天前");
        // 超过 30 天显示日期，格式 YYYY-MM-DD
        let old = format_ago(now - 40 * 86400, now);
        assert_eq!(old.len(), 10);
        assert_eq!(old.matches('-').count(), 2);
    }
}
