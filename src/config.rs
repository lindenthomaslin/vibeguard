//! 配置管理：`~/.config/vibeguard/config.toml` + 环境变量覆盖。
//!
//! 优先级：命令行参数 > 环境变量 > 配置文件 > 默认值。
//! 环境变量只作用于“生效配置”，永远不会被写回配置文件。

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use crate::tools::PermissionMode;

pub const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
pub const DEFAULT_MODEL: &str = "deepseek-flash";
/// 向导中提供的候选模型（DeepSeek 当前在售模型）
pub const KNOWN_MODELS: &[&str] = &["deepseek-flash", "deepseek-v4-pro"];

/// `config set` / `config unset` 支持的配置项
pub const KEYS: &[&str] = &[
    "api_key",
    "base_url",
    "model",
    "thinking",
    "temperature",
    "max_tokens",
    "timeout_secs",
    "permission_mode",
    "workspace_root",
    "theme",
    "font_style",
];

/// 按优先级读取的 API Key 环境变量
pub const API_KEY_ENVS: &[&str] = &["VIBEGUARD_API_KEY", "DEEPSEEK_API_KEY"];
pub const BASE_URL_ENV: &str = "VIBEGUARD_BASE_URL";
pub const MODEL_ENV: &str = "VIBEGUARD_MODEL";

/// 思考模式。`Auto` 表示不发送相关参数，由服务端决定（DeepSeek 默认开启、强度 high），
/// 这样也能兼容不认识 `thinking` 参数的其他 OpenAI 兼容服务。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Thinking {
    #[default]
    Auto,
    Off,
    Low,
    High,
    Max,
}

impl Thinking {
    pub const ALL: [Thinking; 5] = [
        Thinking::Auto,
        Thinking::Off,
        Thinking::Low,
        Thinking::High,
        Thinking::Max,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Thinking::Auto => "auto",
            Thinking::Off => "off",
            Thinking::Low => "low",
            Thinking::High => "high",
            Thinking::Max => "max",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Thinking::Auto => "由服务端决定（DeepSeek 默认开启思考）",
            Thinking::Off => "关闭思考，响应最快",
            Thinking::Low => "轻度思考",
            Thinking::High => "深度思考",
            Thinking::Max => "最大思考强度，最慢",
        }
    }
}

impl fmt::Display for Thinking {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Thinking {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_ascii_lowercase();
        Thinking::ALL
            .into_iter()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| format!("thinking 可选值为 auto / off / low / high / max，收到 `{s}`"))
    }
}

/// API Key 的来源，仅用于展示
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeySource {
    #[default]
    Unset,
    File,
    Env(&'static str),
}

impl KeySource {
    pub fn label(&self) -> String {
        match self {
            KeySource::Unset => "未设置".to_string(),
            KeySource::File => "配置文件".to_string(),
            KeySource::Env(name) => format!("环境变量 {name}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub api_key: Option<String>,
    /// OpenAI 兼容接口的根地址，请求发往 `{base_url}/chat/completions`
    pub base_url: String,
    pub model: String,
    pub thinking: Thinking,
    /// 采样温度；开启思考时服务端会忽略，因此不会发送
    pub temperature: f64,
    pub max_tokens: Option<u32>,
    /// 读超时（秒）：两次收到数据之间的最长等待时间
    pub timeout_secs: u64,
    /// 权限模式：只读/可编辑/全权限
    pub permission_mode: PermissionMode,
    /// 工作目录根路径，用于限制工具访问范围；None 时使用启动时的当前目录
    pub workspace_root: Option<PathBuf>,
    /// UI 主题：dark / light
    pub theme: String,
    /// 字体风格：default / anime
    pub font_style: String,

    #[serde(skip)]
    pub key_source: KeySource,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
            thinking: Thinking::Auto,
            temperature: 0.0,
            max_tokens: None,
            timeout_secs: 120,
            permission_mode: PermissionMode::ReadOnly,
            workspace_root: None,
            theme: "dark".to_string(),
            font_style: "default".to_string(),
            key_source: KeySource::Unset,
        }
    }
}

/// 默认配置文件路径：`$XDG_CONFIG_HOME/vibeguard/config.toml`，
/// 未设置时为 `~/.config/vibeguard/config.toml`（macOS 上同样使用这个路径）。
pub fn default_path() -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => dirs::home_dir()
            .context("无法确定用户主目录")?
            .join(".config"),
    };
    Ok(base.join("vibeguard").join("config.toml"))
}

/// 读取真实进程环境变量，传给 [`Config::with_env`]
pub fn std_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// 已弃用的模型名给出替换建议
pub fn legacy_model_note(model: &str) -> Option<String> {
    let replacement = match model {
        "deepseek-chat" => "deepseek-flash（thinking = off）",
        "deepseek-reasoner" => "deepseek-flash（thinking = high）",
        "deepseek-v4-flash" | "deepseek-v4-flash-vision-exp" => "deepseek-flash",
        _ => return None,
    };
    Some(format!(
        "模型 `{model}` 已被 DeepSeek 弃用，建议改用 {replacement}"
    ))
}

impl Config {
    /// 从文件加载；文件不存在时返回默认配置。这里不做校验，
    /// 以免一个错误的值导致连 `config set` 都无法运行。
    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(e).with_context(|| format!("读取配置文件失败：{}", path.display()));
            }
        };
        let mut cfg: Config = toml::from_str(&text)
            .with_context(|| format!("配置文件格式错误：{}", path.display()))?;
        cfg.api_key = cfg
            .api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
        if cfg.api_key.is_some() {
            cfg.key_source = KeySource::File;
        }
        Ok(cfg)
    }

    /// 保存到文件。Unix 下文件权限为 0600，因为其中可能包含 API Key。
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)
                .with_context(|| format!("创建配置目录失败：{}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self).context("序列化配置失败")?;
        write_private(path, &text).with_context(|| format!("写入配置文件失败：{}", path.display()))
    }

    /// 应用环境变量覆盖。`read` 用于注入环境，测试中可以传入假环境。
    pub fn with_env(mut self, read: impl Fn(&str) -> Option<String>) -> Self {
        let get = |name: &str| {
            read(name)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        for &name in API_KEY_ENVS {
            if let Some(key) = get(name) {
                self.api_key = Some(key);
                self.key_source = KeySource::Env(name);
                break;
            }
        }
        if let Some(url) = get(BASE_URL_ENV) {
            self.base_url = url;
        }
        if let Some(model) = get(MODEL_ENV) {
            self.model = model;
        }
        self
    }

    pub fn validate(&self) -> Result<()> {
        let raw = self.base_url.trim();
        let url =
            reqwest::Url::parse(raw).with_context(|| format!("base_url 不是合法的 URL：{raw}"))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            bail!("base_url 必须是 http:// 或 https:// 地址：{raw}");
        }
        if self.model.trim().is_empty() {
            bail!("model 不能为空");
        }
        if !(0.0..=2.0).contains(&self.temperature) {
            bail!(
                "temperature 取值范围为 0.0 ~ 2.0，当前为 {}",
                self.temperature
            );
        }
        if self.max_tokens == Some(0) {
            bail!("max_tokens 必须大于 0");
        }
        if self.timeout_secs == 0 {
            bail!("timeout_secs 必须大于 0");
        }
        Ok(())
    }

    /// 不阻止运行、但值得提醒用户的问题
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(note) = legacy_model_note(self.model.trim()) {
            out.push(note);
        }
        if let Ok(url) = reqwest::Url::parse(self.base_url.trim()) {
            let host = url.host_str().unwrap_or_default();
            let local = matches!(host, "localhost" | "[::1]") || host.starts_with("127.");
            if url.scheme() == "http" && !local {
                out.push(format!(
                    "base_url 使用 http:// 访问远程主机 {host}，API Key 会被明文传输"
                ));
            }
        }
        out
    }

    /// 读取单个配置项的字符串形式（api_key 返回原文，调用方负责遮蔽）
    pub fn get(&self, key: &str) -> Result<String> {
        Ok(match key {
            "api_key" => self.api_key.clone().unwrap_or_default(),
            "base_url" => self.base_url.clone(),
            "model" => self.model.clone(),
            "thinking" => self.thinking.to_string(),
            "temperature" => self.temperature.to_string(),
            "max_tokens" => self
                .max_tokens
                .map_or_else(|| "none".to_string(), |n| n.to_string()),
            "timeout_secs" => self.timeout_secs.to_string(),
            "permission_mode" => self.permission_mode.label().to_string(),
            "workspace_root" => self
                .workspace_root
                .as_ref()
                .map_or_else(|| "default".to_string(), |p| p.display().to_string()),
            "theme" => self.theme.clone(),
            "font_style" => self.font_style.clone(),
            _ => bail!("未知配置项 `{key}`，可选：{}", KEYS.join(", ")),
        })
    }

    /// 设置单个配置项（用于 `config set`）。先在副本上修改并校验，
    /// 失败时原配置保持不变，避免一次错误输入污染后续操作。
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let mut next = self.clone();
        next.apply(key, value)?;
        next.validate()?;
        *self = next;
        Ok(())
    }

    fn apply(&mut self, key: &str, value: &str) -> Result<()> {
        let v = value.trim();
        match key {
            "api_key" => {
                self.api_key = (!v.is_empty()).then(|| v.to_string());
                self.key_source = if self.api_key.is_some() {
                    KeySource::File
                } else {
                    KeySource::Unset
                };
            }
            "base_url" => self.base_url = v.to_string(),
            "model" => self.model = v.to_string(),
            "thinking" => self.thinking = v.parse().map_err(anyhow::Error::msg)?,
            "temperature" => {
                self.temperature = v
                    .parse()
                    .with_context(|| format!("temperature 需要是数字：{v}"))?
            }
            "max_tokens" => {
                self.max_tokens = if v.is_empty() || v.eq_ignore_ascii_case("none") {
                    None
                } else {
                    Some(
                        v.parse()
                            .with_context(|| format!("max_tokens 需要是正整数：{v}"))?,
                    )
                }
            }
            "timeout_secs" => {
                self.timeout_secs = v
                    .parse()
                    .with_context(|| format!("timeout_secs 需要是正整数：{v}"))?
            }
            "permission_mode" => {
                self.permission_mode = match v.to_lowercase().as_str() {
                    "readonly" => PermissionMode::ReadOnly,
                    "editable" => PermissionMode::Editable,
                    "full" => PermissionMode::Full,
                    _ => bail!("permission_mode 可选值为 readonly / editable / full，收到 `{v}`"),
                }
            }
            "workspace_root" => {
                self.workspace_root = if v.is_empty() || v.eq_ignore_ascii_case("default") {
                    None
                } else {
                    Some(PathBuf::from(v))
                }
            }
            "theme" => {
                let theme = v.to_lowercase();
                if matches!(theme.as_str(), "dark" | "light") {
                    self.theme = theme;
                } else {
                    bail!("theme 可选值为 dark / light，收到 `{v}`");
                }
            }
            "font_style" => {
                let style = v.to_lowercase();
                if matches!(style.as_str(), "default" | "anime") {
                    self.font_style = style;
                } else {
                    bail!("font_style 可选值为 default / anime，收到 `{v}`");
                }
            }
            _ => bail!("未知配置项 `{key}`，可选：{}", KEYS.join(", ")),
        }
        Ok(())
    }

    /// 将单个配置项恢复为默认值
    pub fn unset(&mut self, key: &str) -> Result<()> {
        let d = Config::default();
        match key {
            "api_key" => {
                self.api_key = None;
                self.key_source = KeySource::Unset;
            }
            "base_url" => self.base_url = d.base_url,
            "model" => self.model = d.model,
            "thinking" => self.thinking = d.thinking,
            "temperature" => self.temperature = d.temperature,
            "max_tokens" => self.max_tokens = d.max_tokens,
            "timeout_secs" => self.timeout_secs = d.timeout_secs,
            "permission_mode" => self.permission_mode = d.permission_mode,
            "workspace_root" => self.workspace_root = d.workspace_root,
            "theme" => self.theme = d.theme,
            "font_style" => self.font_style = d.font_style,
            _ => bail!("未知配置项 `{key}`，可选：{}", KEYS.join(", ")),
        }
        Ok(())
    }

    /// 规范化后的 API 根地址：去掉末尾的 `/` 和误填的 `/chat/completions`
    pub fn api_base(&self) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        base.strip_suffix("/chat/completions")
            .unwrap_or(base)
            .trim_end_matches('/')
            .to_string()
    }
}

/// 遮蔽 API Key，只显示首尾少量字符
pub fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 10 {
        return "****".to_string();
    }
    let head: String = chars[..3].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

#[cfg(unix)]
pub(crate) fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // 文件已存在时 mode() 不生效，写入内容前先收紧权限
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(contents.as_bytes())
}

#[cfg(not(unix))]
pub(crate) fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    fs::write(path, contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn fake_env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn missing_file_gives_default() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::load(&dir.path().join("nope.toml")).unwrap();
        assert_eq!(cfg, Config::default());
        assert_eq!(cfg.key_source, KeySource::Unset);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.toml");
        let mut cfg = Config::default();
        cfg.set("api_key", "sk-test-1234567890").unwrap();
        cfg.set("model", "deepseek-v4-pro").unwrap();
        cfg.set("thinking", "HIGH").unwrap();
        cfg.set("max_tokens", "4096").unwrap();
        cfg.save(&path).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded, cfg);
        assert_eq!(loaded.thinking, Thinking::High);
        assert_eq!(loaded.key_source, KeySource::File);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("thinking = \"high\""), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        Config::default().save(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "model = \"deepseek-v4-pro\"\napi_key = \"  \"\n").unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.model, "deepseek-v4-pro");
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.thinking, Thinking::Auto);
        assert_eq!(cfg.api_key, None, "空白字符串视为未设置");
    }

    #[test]
    fn invalid_file_reports_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for bad in ["model = ", "thinking = \"turbo\""] {
            fs::write(&path, bad).unwrap();
            let err = format!("{:#}", Config::load(&path).unwrap_err());
            assert!(err.contains("config.toml"), "{err}");
        }
    }

    #[test]
    fn env_overrides_file_with_priority() {
        let mut cfg = Config::default();
        cfg.set("api_key", "sk-from-file-xxxx").unwrap();

        let cfg = cfg.with_env(fake_env(&[
            ("VIBEGUARD_API_KEY", "  "), // 空白值忽略
            ("DEEPSEEK_API_KEY", "sk-from-env-yyyy"),
            ("VIBEGUARD_MODEL", "deepseek-v4-pro"),
        ]));
        assert_eq!(cfg.api_key.as_deref(), Some("sk-from-env-yyyy"));
        assert_eq!(cfg.key_source, KeySource::Env("DEEPSEEK_API_KEY"));
        assert_eq!(cfg.model, "deepseek-v4-pro");

        let cfg = cfg.with_env(fake_env(&[("VIBEGUARD_API_KEY", "sk-vg-zzzz")]));
        assert_eq!(cfg.key_source, KeySource::Env("VIBEGUARD_API_KEY"));
    }

    #[test]
    fn set_rejects_bad_values() {
        let mut cfg = Config::default();
        assert!(cfg.set("unknown", "x").is_err());
        assert!(cfg.set("temperature", "abc").is_err());
        assert!(cfg.set("temperature", "3.5").is_err());
        assert!(cfg.set("base_url", "ftp://example.com").is_err());
        assert!(cfg.set("base_url", "not a url").is_err());
        assert!(cfg.set("timeout_secs", "0").is_err());
        assert!(cfg.set("thinking", "turbo").is_err());
        cfg.set("max_tokens", "none").unwrap();
        assert_eq!(cfg.max_tokens, None);
        assert_eq!(cfg, Config::default(), "失败的 set 不应修改配置");
    }

    #[test]
    fn get_and_unset() {
        let mut cfg = Config::default();
        cfg.set("timeout_secs", "30").unwrap();
        cfg.set("thinking", "off").unwrap();
        assert_eq!(cfg.get("timeout_secs").unwrap(), "30");
        assert_eq!(cfg.get("thinking").unwrap(), "off");
        assert_eq!(cfg.get("max_tokens").unwrap(), "none");
        cfg.unset("timeout_secs").unwrap();
        cfg.unset("thinking").unwrap();
        assert_eq!(cfg, Config::default());
        assert!(cfg.unset("nope").is_err());
    }

    #[test]
    fn warnings_for_legacy_model_and_plain_http() {
        let mut cfg = Config::default();
        assert!(cfg.warnings().is_empty());

        cfg.model = "deepseek-reasoner".into();
        cfg.base_url = "http://api.example.com".into();
        let w = cfg.warnings();
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].contains("deepseek-flash"));
        assert!(w[1].contains("明文"));

        cfg.model = DEFAULT_MODEL.into();
        for local in [
            "http://localhost:8080",
            "http://127.0.0.1:1234/v1",
            "http://[::1]:80",
        ] {
            cfg.base_url = local.into();
            assert!(cfg.warnings().is_empty(), "{local}");
        }
    }

    #[test]
    fn api_base_is_normalized() {
        let mut cfg = Config::default();
        for (input, expected) in [
            ("https://api.deepseek.com", "https://api.deepseek.com"),
            ("https://api.deepseek.com/", "https://api.deepseek.com"),
            (
                "https://api.deepseek.com/v1/",
                "https://api.deepseek.com/v1",
            ),
            (
                "https://api.deepseek.com/chat/completions",
                "https://api.deepseek.com",
            ),
        ] {
            cfg.base_url = input.to_string();
            assert_eq!(cfg.api_base(), expected, "input: {input}");
        }
    }

    #[test]
    fn key_masking() {
        assert_eq!(mask_key("sk-1234567890abcdef"), "sk-…cdef");
        assert_eq!(mask_key("short"), "****");
    }
}
