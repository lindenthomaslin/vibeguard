# VibeGuard 交接文档

## 需求（用户原话整理）
- 用 Rust 写一个开源 CLI 工具，交互风格对标 Claude Code 终端
- 默认接入 DeepSeek API（OpenAI 兼容接口：`https://api.deepseek.com/chat/completions`，模型 `deepseek-chat` / `deepseek-reasoner`）
- 核心功能：
  1. 扫描 vibecoding（AI 生成代码）项目中的安全漏洞
  2. 自动修复漏洞；也支持"先核对文档/报告，确认后再修复"的模式

## 当前进度
已完成：
- 环境：macOS，Rust 1.97.1 / cargo 1.97.1（Homebrew）
- `cargo new vibeguard` 已创建，依赖已添加，`cargo build` 已通过（只有默认的 hello world `src/main.rs`）

未开始：业务代码一行都还没写。

## 已添加的依赖（Cargo.toml）
anyhow, chrono, clap(derive, env), console 0.16, dialoguer 0.12, dirs 7, futures-util, ignore,
indicatif 0.18, once_cell, regex, reqwest 0.13（default-features=false, features=json,stream,rustls,http2）,
serde(derive), serde_json, similar 3.2, tokio(rt-multi-thread,macros,sync,time,fs), toml 1.1
dev: tempfile

## 踩过的坑 / API 确认
- reqwest 0.13 的 TLS feature 名称已从 `rustls-tls` 改为 `rustls`
- similar 3.2：`TextDiff::from_lines(old, new)`、`.iter_all_changes()`、`.grouped_ops(n)`、`.unified_diff()`；`Change` 有 `tag()`、`value()`、`old_index()`、`new_index()`
- toml 1.1：`toml::from_str`、`toml::to_string_pretty`
- dialoguer 0.12：`Confirm/Select/Input/Password` 都是 `.with_prompt().default().interact()`；`Password` 默认 feature 已启用
- indicatif 0.18：`ProgressBar::new_spinner()`、`enable_steady_tick`、`set_message`、`suspend`、`println`；`ProgressStyle::with_template`
- console 0.16：`style()`、`measure_text_width`、`truncate_str`
- cargo registry 路径：`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`

## 计划中的设计（尚未实现，供参考）
命令：
- `vibeguard`（无参数）→ 进入 REPL 交互模式，支持 `/scan` `/fix` `/report` `/config` `/help` 等斜杠命令
- `vibeguard scan [path]` → 扫描并输出报告（终端 + `vibeguard-report.md` / JSON）
- `vibeguard fix [path] [--auto | --review]` → `--review` 为默认：先生成报告，逐条展示 diff 并确认；`--auto` 直接修复
- `vibeguard config` → 设置 API Key（也读取环境变量 `DEEPSEEK_API_KEY`）、模型、base_url

模块划分：
- `config.rs`：`~/.config/vibeguard/config.toml`
- `llm.rs`：DeepSeek 客户端（流式 SSE）
- `scanner/rules.rs`：本地正则规则快速初筛（硬编码密钥、SQL 拼接、eval/exec、命令注入、XSS innerHTML、CORS `*`、Debug 模式、弱 JWT secret、不安全反序列化、路径穿越等）
- `scanner/ai.rs`：把文件 + 初筛结果发给 LLM 深度分析，要求返回 JSON（id、严重级别、CWE、文件、行号、说明、修复建议）
- `fixer.rs`：让 LLM 生成修复后的代码 → 用 similar 展示 diff → 确认后写入，写入前备份到 `.vibeguard/backup/`，支持回滚
- `ui.rs`：Claude Code 风格的终端输出（彩色、spinner、框线）
- 文件遍历用 `ignore` crate，尊重 .gitignore，跳过二进制/大文件

## 下一步
1. 按上面设计写代码并 `cargo build`
2. 写单元测试（规则匹配、diff、配置读写），可用 tempfile
3. 准备一个含已知漏洞的示例项目用于验证
4. 添加 README 与 LICENSE（开源）
