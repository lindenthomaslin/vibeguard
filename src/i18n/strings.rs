//! 中文 → English 对照表。
//!
//! 以中文原文为键：调用处不用动，`tr("中文")` 在英文模式下换成右边的译文。
//! 带 `{}` 的是 format! 模板，由 `pick(format!(zh), format!(en))` 使用。
//!
//! 查不到的字符串原样返回中文——漏译不会 panic，只会露一句中文。

pub static TABLE: &[(&str, &str)] = &[
    (
        "  /model 表单：↑↓ 切换项 · 直接输入修改 · ←→ 调整思考力度 · Ctrl-S 保存 · Esc 取消",
        "/model form: ↑↓ switch field · type to edit · ←→ thinking effort · Ctrl-S save · Esc cancel",
    ),
    (
        "  /permissions <模式> <目录>      同时设置权限模式和工作目录",
        "/permissions <mode> <dir>       set the permission mode and working directory",
    ),
    (
        "  /permissions editable           可编辑，能修改文件，不能执行命令",
        "/permissions editable           edit files, but cannot run commands",
    ),
    (
        "  /permissions full               全权限，能修改文件和执行命令（删除除外）",
        "/permissions full               edit files and run commands (except deletes)",
    ),
    (
        "  /permissions readonly           只读，不能修改文件或执行命令",
        "/permissions readonly           read only: no file changes, no commands",
    ),
    (
        "  Ctrl-J 换行 · PgUp/PgDn 滚动 · Ctrl-B 到顶部 · Ctrl-G 到底部",
        "Ctrl-J newline · PgUp/PgDn scroll · Ctrl-B top · Ctrl-G bottom",
    ),
    (
        "  Ctrl-T 换主题 · /theme light · /font anime · /config set theme light 长期生效",
        "Ctrl-T theme · /theme light · /font anime · /config set theme light to persist",
    ),
    (
        "  Shift-Tab / Ctrl-P 切换模式：plan 只读出报告 · confirm 每改一处确认 · auto 全自动",
        "Shift-Tab / Ctrl-P cycle: plan reports only · confirm per change · auto all the way",
    ),
    (
        "  Shift-Tab / Ctrl-P 循环切换，也可以 /mode plan",
        "Shift-Tab / Ctrl-P to cycle, or /mode plan",
    ),
    (
        "  {undetermined} 条静态看不出来——它们要跑起来发请求才能验，别当成「没问题」",
        "{undetermined} cannot be judged statically — they need a running app to verify, so do not read this as \"clean\"",
    ),
    ("  … 还有 {} 条未列出", "… and {} more not listed"),
    ("  对话目录      ", "Sessions      "),
    ("  工作目录           ", "Working dir         "),
    (
        "  已写入 {path}，以后每次启动都会使用",
        "Wrote {path}; it will be used on every start from now on",
    ),
    ("  有一批没跑成功：{f}", "Some batches failed: {f}"),
    (
        "  本地规则覆盖清单里的 {rules}/{checklist_total} 条；其余需要模型做语义判断，这一轮还没跑",
        "Local rules cover {rules}/{checklist_total} checklist items; the rest need model judgement, which did not run this round",
    ),
    ("  权限模式           ", "Permission mode     "),
    (
        "  结果超过上限，已截断（先修掉手头这些，或调大 MAX_FINDINGS）",
        "Result hit the cap and was truncated (fix these first, or raise MAX_FINDINGS)",
    ),
    ("  （无改动）", "(no changes)"),
    (" · {} 轮", " · {} rounds"),
    (
        " · 不存在 {not_found} · 无法判断 {undetermined}",
        " · missing {not_found} · undetermined {undetermined}",
    ),
    (" · 该行：`{clipped}`", " · that line: `{clipped}`"),
    (" 个文件，", " files, "),
    (" 个有问题", " with problems"),
    (" 个问题", " issues"),
    (" 条清单：", " checklist items: "),
    (" 等 {more} 个", " and {more} more"),
    ("# VibeGuard 扫描报告\n\n", "# VibeGuard scan report\n\n"),
    (
        "## 与本批条目相关的代码（按关键词检索挑出来的，不是全部）\n",
        "## Code related to this batch (picked by keyword retrieval, not exhaustive)\n",
    ),
    ("## 仓库文件树\n```\n", "## Repository tree\n```\n"),
    ("## 发现明细\n\n", "## Findings\n\n"),
    ("## 待判断条目\n", "## Items to judge\n"),
    ("## 结论\n\n", "## Conclusion\n\n"),
    ("## 说明\n\n", "## Notes\n\n"),
    ("- **位置**：`{}:{}`\n", "- **Location**: `{}:{}`\n"),
    ("- **修复建议**：{}\n", "- **Fix**: {}\n"),
    ("- **编号**：{} · {}\n", "- **ID**: {} · {}\n"),
    ("- **编号**：{}\n", "- **ID**: {}\n"),
    ("- **说明**：{}\n", "- **Detail**: {}\n"),
    ("- {} [{}] {}｜检测点：{}\n", "- {} [{}] {} | check: {}\n"),
    (
        "AI 判定只跑安全类分类（A–M、P、R），跳过 N/O/Q/S 的 {skipped} 条工程规范条目；要全跑加 --all",
        "AI judging covers security categories only (A–M, P, R), skipping {skipped} engineering-hygiene items in N/O/Q/S; add --all to run everything",
    ),
    (
        "AI 生成代码的安全扫描与修复",
        "Security scanning and fixing for AI-generated code",
    ),
    (
        "API Key 不能包含空白字符，请检查是否多复制了内容",
        "API key must not contain whitespace — check for a stray copy-paste",
    ),
    (
        "API Key 无效或已过期，请输入 /model 重新设置（命令行：vibeguard config set api_key）",
        "API key is invalid or expired; set it again with /model (CLI: vibeguard config set api_key)",
    ),
    (
        "API Key（在 platform.deepseek.com 获取）",
        "API key (get one at platform.deepseek.com)",
    ),
    (
        "API Key（当前 {}，回车保留）",
        "API key (current {}, Enter to keep)",
    ),
    (
        "API Key（当前使用环境变量 {name}，回车跳过）",
        "API key (using env var {name}, Enter to skip)",
    ),
    ("API 地址", "API base URL"),
    (
        "API 地址不能为空，默认是 {DEFAULT_BASE_URL}",
        "API base URL must not be empty; default is {DEFAULT_BASE_URL}",
    ),
    (
        "API 地址不能包含空白字符",
        "API base URL must not contain whitespace",
    ),
    ("API 连接正常", "API connection OK"),
    (
        "CORS 允许任意来源，配合 Cookie 凭据即可跨站读取数据",
        "CORS allows any origin, so cookie credentials let any site read the data",
    ),
    (
        "ECB 模式相同明文产生相同密文，会泄露结构信息",
        "ECB mode maps equal plaintext to equal ciphertext, leaking structure",
    ),
    (
        "Enter 保存 · ↑↓ 切换项 · Esc 取消",
        "Enter save · ↑↓ switch · Esc cancel",
    ),
    (
        "Enter 发送 · Ctrl-J 换行 · / 命令 · Ctrl-D 退出",
        "Enter send · Ctrl-J newline · / commands · Ctrl-D quit",
    ),
    (
        "Enter 发送 · Ctrl-J 换行 · Shift-Tab 换模式 · / 命令 · Ctrl-D 退出",
        "Enter send · Ctrl-J newline · Shift-Tab mode · / commands · Ctrl-D quit",
    ),
    ("Esc 取消 · PgUp/PgDn 滚动", "Esc cancel · PgUp/PgDn scroll"),
    (
        "JSON 模式要求提示词中包含 \"json\" 字样（DeepSeek API 限制）",
        "JSON mode requires the word \"json\" in the prompt (DeepSeek API limit)",
    ),
    (
        "MD5/SHA1 已不适合用于口令或签名",
        "MD5/SHA1 are no longer fit for passwords or signatures",
    ),
    (
        "SQL 语句用字符串拼接，应改为参数化查询",
        "SQL built by string concatenation; use parameterised queries",
    ),
    (
        "XML 解析器要显式禁用外部实体，否则可被 XXE 读取本地文件",
        "The XML parser must disable external entities, otherwise XXE can read local files",
    ),
    (
        "[{}] 模型判定为「{}」：{}{}",
        "[{}] model says \"{}\": {}{}",
    ),
    (
        "\n> ⚠ 有 {} 条模型判定找不到对应代码（文件不存在或行号超范围），**已从下面的清单里剔除**。\n> 剔除不等于没问题：这些条目可能确实存在，只是模型指错了地方，值得手工看一眼。\n\n",
        "\n> ⚠ {} model verdicts could not be matched to code (missing file, or line out of range) and **were removed from the list below**.\n> Removed is not the same as clean: these may well be real, the model just pointed at the wrong place. Worth a manual look.\n\n",
    ),
    (
        "base_url 不是合法的 URL：{raw}",
        "base_url is not a valid URL: {raw}",
    ),
    (
        "base_url 使用 http:// 访问远程主机 {host}，API Key 会被明文传输",
        "base_url uses http:// for remote host {host}; the API key would travel in clear text",
    ),
    (
        "base_url 必须是 http:// 或 https:// 地址：{raw}",
        "base_url must be an http:// or https:// address: {raw}",
    ),
    (
        "font_style 可选值为 default / anime，收到 `{v}`",
        "font_style must be default / anime, got `{v}`",
    ),
    (
        "lang 可选值为 en / zh，收到 `{raw}`",
        "lang must be en / zh, got `{raw}`",
    ),
    (
        "lang 可选值为 en / zh，收到 `{v}`",
        "lang must be en / zh, got `{v}`",
    ),
    ("max_tokens 必须大于 0", "max_tokens must be greater than 0"),
    (
        "max_tokens 需要是正整数：{v}",
        "max_tokens must be a positive integer: {v}",
    ),
    (
        "max_tool_rounds 取值范围为 1 ~ {MAX_TOOL_ROUNDS_CEILING}，当前为 {}",
        "max_tool_rounds must be 1 ~ {MAX_TOOL_ROUNDS_CEILING}, currently {}",
    ),
    (
        "max_tool_rounds 需要是正整数：{v}",
        "max_tool_rounds must be a positive integer: {v}",
    ),
    ("model 不能为空", "model must not be empty"),
    (
        "permission_mode 可选值为 readonly / editable / full，收到 `{v}`",
        "permission_mode must be readonly / editable / full, got `{v}`",
    ),
    (
        "permission_mode 是 readonly · 用 /config set 改成 full",
        "permission_mode is readonly · change it with /config set permission_mode full",
    ),
    (
        "plan 模式强制只读 · Shift-Tab 切到 confirm/auto",
        "plan mode forces read-only · Shift-Tab to confirm/auto",
    ),
    (
        "results\":[{\"id\":\"AUTH-01\",\"verdict\":\"exists\",\"evidence\":\"app.py:12\",\"reason\":\"一句话依据\"}]}\n\n",
        "results\":[{\"id\":\"AUTH-01\",\"verdict\":\"exists\",\"evidence\":\"app.py:12\",\"reason\":\"one-line reason\"}]}\n\n",
    ),
    (
        "shell=True 会让参数里的 `;` `$()` 被解释执行，构成命令注入",
        "shell=True lets `;` and `$()` in arguments execute, which is command injection",
    ),
    (
        "temperature 取值范围为 0.0 ~ 2.0，当前为 {}",
        "temperature must be 0.0 ~ 2.0, currently {}",
    ),
    (
        "temperature 需要是数字：{v}",
        "temperature must be a number: {v}",
    ),
    (
        "theme 可选值为 dark / light，收到 `{v}`",
        "theme must be dark / light, got `{v}`",
    ),
    (
        "thinking 可选值为 auto / off / low / high / max，收到 `{s}`",
        "thinking must be auto / off / low / high / max, got `{s}`",
    ),
    (
        "timeout_secs 必须大于 0",
        "timeout_secs must be greater than 0",
    ),
    (
        "timeout_secs 需要是正整数：{v}",
        "timeout_secs must be a positive integer: {v}",
    ),
    (
        "token 存在 localStorage 里，任何 XSS 都能直接读走",
        "the token lives in localStorage, so any XSS can read it",
    ),
    ("{detail}（你拒绝了）", "{detail} (you declined)"),
    (
        "{detail}（计划模式，已挡回）",
        "{detail} (plan mode, blocked)",
    ),
    ("{head}\n…（已截断）", "{head}\n… (truncated)"),
    (
        "{key} 已恢复默认值：{}",
        "{key} restored to its default: {}",
    ),
    ("{label} 被拒绝", "{label} declined"),
    (
        "{label}（只有一条，返回的 JSON 不完整）",
        "{label} (single item, returned incomplete JSON)",
    ),
    (
        "{path} 写入失败，原因见下面的工具返回",
        "{path} write failed; see the tool result below",
    ),
    (
        "{stale} 条发现所在的文件在扫描后改过，行号可能已经不对了：先重扫再修",
        "{stale} findings live in files changed since the scan, so line numbers may be stale: rescan first",
    ),
    ("{} 写入失败：{e}", "{} write failed: {e}"),
    ("{} 分钟前", "{} minutes ago"),
    (
        "{} 在确认期间被改过，已跳过",
        "{} changed while you were confirming, skipped",
    ),
    ("{} 天前", "{} days ago"),
    ("{} 小时前", "{} hours ago"),
    (
        "{} 条判定的证据核实不了，已剔除（{}）",
        "{} verdicts could not be verified against the code and were dropped ({})",
    ),
    ("{} 等 {} 类", "{} and {} more categories"),
    ("{} 读不出来：{e}", "{} cannot be read: {e}"),
    ("{}（前半）", "{} (first half)"),
    ("{}（后半）", "{} (second half)"),
    (
        "{}（拆半后仍失败：{e:#}；正文 {} 字）",
        "{} (still failing after splitting: {e:#}; body {} chars)",
    ),
    (
        "{}（拆半后仍没有判定）",
        "{} (no verdict even after splitting)",
    ),
    (
        "| AI 判定范围 | 安全类 A–M、P、R（N/O/Q/S 的 {} 条工程规范未交给模型，`--all` 可全跑） |\n",
        "| AI judging | security A–M, P, R ({} engineering items in N/O/Q/S were not sent to the model; `--all` runs them) |\n",
    ),
    (
        "| 工具版本 | vibeguard {} |\n",
        "| Tool version | vibeguard {} |\n",
    ),
    ("| 扫描文件 | {} 个 |\n", "| Files scanned | {} |\n"),
    ("| 扫描目标 | `{}` |\n", "| Target | `{}` |\n"),
    (
        "| 模型判定 | 条数 |\n|---|---:|\n",
        "| Model verdicts | count |\n|---|---:|\n",
    ),
    (
        "| 清单覆盖 | {}/{} 条（本地规则 {} 条 + 模型判定 {} 条） |\n\n",
        "| Checklist coverage | {}/{} items ({} from local rules + {} from the model) |\n\n",
    ),
    ("| 生成时间 | {} |\n", "| Generated | {} |\n"),
    (
        "| 级别 | 数量 |\n|---|---:|\n",
        "| Severity | count |\n|---|---:|\n",
    ),
    (
        "…（前 {start} 行省略）\n",
        "… (first {start} lines omitted)\n",
    ),
    ("…（后面还有 {} 行）\n", "… ({} more lines below)\n"),
    (
        "←→ 调整思考力度 · ↑↓ 切换项 · Enter 下一项 · Ctrl-S 保存 · Esc 取消",
        "←→ thinking effort · ↑↓ switch · Enter next · Ctrl-S save · Esc cancel",
    ),
    (
        "↑↓ 选择 · Enter 确认 · y 应用 · n 跳过 · a 全部应用 · q 结束",
        "↑↓ select · Enter confirm · y apply · n skip · a apply all · q quit",
    ),
    (
        "↑↓ 选择 · Enter 确认 · y 应用 · n 跳过 · a 本次全部应用 · Esc 跳过",
        "↑↓ select · Enter confirm · y apply · n skip · a apply all this run · Esc skip",
    ),
    ("✻ 思考过程", "✻ Thinking"),
    ("不再逐条询问", "Stop asking for each one"),
    ("不写入，继续下一个", "Do not write, go to the next"),
    ("不存在", "missing"),
    ("不打断，一路做完", "Do not interrupt, run to the end"),
    ("不是一个目录：{}", "not a directory: {}"),
    ("严重", "Critical"),
    ("中危", "Medium"),
    ("中文", "Chinese"),
    (
        "为扫描结果生成修复，逐条确认后写入（/fix 数量 或 /fix VG-003）",
        "Generate fixes for the scan results, confirming each before writing (/fix N or /fix VG-003)",
    ),
    ("主题 {} · 字形 {}", "Theme {} · font {}"),
    ("主题可选值为 dark / light", "theme must be dark / light"),
    ("低危", "Low"),
    (
        "你是安全工程师，正在修一个具体的安全问题。\n\n         文件：{file}\n         问题：[{id}] {title}\n         位置：第 {line} 行\n         说明：{desc}\n         修复要点：{fix}\n\n         要求：\n         1. 只改和这个问题相关的代码，其余原样保留，不要顺手重构、不要改格式。\n         2. 返回**修好之后的完整文件**，不要省略、不要用「...」代替、不要只给片段。\n         3. 如果这个文件里其实没有问题，把 content 原样返回，并在 note 里说明理由。\n\n         以 json 输出：{{\"content\":\"完整文件内容\",\"note\":\"改了什么\"}}\n\n         文件全文：\n```\n{content}\n```",
        "You are a security engineer fixing one specific security issue.\n\n         File: {file}\n         Issue: [{id}] {title}\n         Location: line {line}\n         Detail: {desc}\n         How to fix: {fix}\n\n         Requirements:\n         1. Change only the code related to this issue and keep everything else as is: no opportunistic refactoring, no reformatting.\n         2. Return the **complete fixed file**; do not elide anything, do not use \"...\", do not return a fragment.\n         3. If this file actually has no problem, return content unchanged and explain why in note.\n\n         Output as json: {{\"content\":\"full file content\",\"note\":\"what changed\"}}\n\n         Full file:\n```\n{content}\n```",
    ),
    ("保存", "Save"),
    (
        "保存失败，配置文件没有改动：{e:#}",
        "Save failed, the config file is unchanged: {e:#}",
    ),
    ("保存失败：{}", "Save failed: {}"),
    ("保存对话失败：{}", "Saving the conversation failed: {}"),
    ("保持原样，看下一条", "Keep as is, next one"),
    ("修复 {}/{}：{} · {}:{}", "Fixing {}/{}: {} · {}:{}"),
    ("修改", "Edit"),
    ("修改并保存配置项", "Edit and save a config key"),
    ("全权限", "full"),
    ("全自动", "Auto"),
    (
        "共 {} 个 · 输入过滤 · Enter 打开 · Del 删除 · Esc 关闭",
        "{} total · type to filter · Enter open · Del delete · Esc close",
    ),
    ("共写入 {written} 个文件", "wrote {written} files"),
    ("共发现 ", "Found "),
    ("共发现 **{}** 个问题：\n\n", "Found **{}** issues:\n\n"),
    (
        "关闭了 TLS 证书校验，中间人可以任意篡改流量",
        "TLS verification is off, so a man in the middle can rewrite traffic",
    ),
    ("关闭思考，响应最快", "Thinking off, fastest response"),
    ("再按一次 Ctrl-C 退出", "Press Ctrl-C again to quit"),
    ("写入", "Write"),
    ("写入失败：{}", "Write failed: {}"),
    ("写入对话失败：{}", "Writing the conversation failed: {}"),
    ("写入或创建文件", "Write or create a file"),
    ("写入这个修复", "Write this fix"),
    ("写入这个改动", "Write this change"),
    ("写入配置文件失败：{}", "Writing the config file failed: {}"),
    (
        "写入配置文件，以后每次启动都会使用",
        "Write to the config file, used on every start",
    ),
    ("切换主题（dark/light）", "Switch theme (dark/light)"),
    ("切换字体风格", "Switch font style"),
    (
        "切换字体风格（default/anime）",
        "Switch font style (default/anime)",
    ),
    (
        "切换字形：default / anime",
        "Switch glyphs: default / anime",
    ),
    (
        "切换工作模式：计划 / 逐条确认 / 全自动",
        "Switch work mode: plan / confirm / auto",
    ),
    ("切换界面主题", "Switch the colour theme"),
    (
        "切换配色：dark / light（Ctrl-T）",
        "Switch palette: dark / light (Ctrl-T)",
    ),
    ("列出目录内容", "List directory contents"),
    ("列目录", "List dir"),
    ("刚刚", "just now"),
    (
        "创建对话目录失败：{}",
        "Creating the conversation directory failed: {}",
    ),
    (
        "创建扫描目录失败：{}",
        "Creating the scan directory failed: {}",
    ),
    ("创建目录失败：{}", "Creating the directory failed: {}"),
    (
        "创建配置目录失败：{}",
        "Creating the config directory failed: {}",
    ),
    (
        "初始化 HTTP 客户端失败",
        "Failed to initialise the HTTP client",
    ),
    (
        "删除「{}」？再按 Del 确认 · Esc 取消",
        "Delete \"{}\"? Press Del again · Esc cancel",
    ),
    (
        "删除命令需要用户确认（当前不支持自动执行）",
        "Delete commands need user confirmation (not supported yet)",
    ),
    ("删除对话失败：{}", "Deleting the conversation failed: {}"),
    ("动漫", "Anime"),
    ("历史对话 {}", "Conversation {}"),
    ("历史对话 {}/{}", "Conversation {}/{}"),
    (
        "历史对话：搜索、打开、删除",
        "Conversations: search, open, delete",
    ),
    ("参数格式错误：{e}", "Bad arguments: {e}"),
    (
        "反序列化不可信数据可导致任意代码执行",
        "Deserialising untrusted data can lead to arbitrary code execution",
    ),
    ("只读", "read-only"),
    (
        "只读分析，产出漏洞报告，不改文件",
        "Read-only analysis that produces a report and touches nothing",
    ),
    ("可编辑", "editable"),
    (
        "同一份数据的机器可读版本：{}",
        "Machine-readable version of the same data: {}",
    ),
    ("命令", "Command"),
    (
        "命令串里拼了变量或格式化，用户输入会变成 shell 指令",
        "The command is assembled from variables or formatting, so user input becomes shell syntax",
    ),
    ("命令参数", "Command arguments"),
    ("命令名（如 ls, git 等）", "Command name (e.g. ls, git)"),
    (
        "响应流意外结束，结果可能不完整",
        "The response stream ended unexpectedly; the result may be incomplete",
    ),
    (
        "回答达到 max_tokens 上限被截断",
        "The answer hit the max_tokens limit and was truncated",
    ),
    ("圆角框 + 星星爱心", "Rounded frame + stars and hearts"),
    (
        "圆角框 + 星星爱心，主色樱花粉",
        "Rounded frame + stars and hearts, sakura pink accent",
    ),
    (
        "备份或打包文件留在项目里，部署后往往能被直接下载",
        "Backup and archive files left in the project are often downloadable once deployed",
    ),
    ("外观 ", "Appearance "),
    (
        "字体风格可选值为 default / anime",
        "font_style must be default / anime",
    ),
    (
        "字体风格已切换为：{style}",
        "Font style switched to: {style}",
    ),
    (
        "字形可选值为 default / anime",
        "glyphs must be default / anime",
    ),
    ("字形：{}（{hint}）", "Font: {} ({hint})"),
    ("存在", "present"),
    ("存在 {exists}", "{exists} present"),
    (
        "存档失败（结果只在这次会话里）：{e:#}",
        "Archiving failed (results only live in this session): {e:#}",
    ),
    (
        "容器以 root 运行，逃逸后直接拿到宿主机高权限",
        "The container runs as root, so an escape hands over the host",
    ),
    (
        "密钥/口令写死在源码里，一旦提交就会永久留在 git 历史中",
        "Secrets hardcoded in source stay in git history forever once committed",
    ),
    (
        "对话保存失败：{e:#}",
        "Saving the conversation failed: {e:#}",
    ),
    (
        "对话已经不存在：{}",
        "The conversation no longer exists: {}",
    ),
    ("对话文件格式错误：{}", "Malformed conversation file: {}"),
    (
        "尚未设置 API Key，扫描和修复功能将无法使用",
        "API key not set; scanning and fixing will not work",
    ),
    (
        "尚未配置 API Key：输入 /config init，或设置环境变量 DEEPSEEK_API_KEY",
        "API key not configured: run /config init, or set DEEPSEEK_API_KEY",
    ),
    (
        "尚未配置 API Key：输入 /model 填写 API 地址、Key 和模型，或设置环境变量 DEEPSEEK_API_KEY",
        "API key not configured: run /model to set the base URL, key and model, or set DEEPSEEK_API_KEY",
    ),
    ("工作模式 ", "Work mode "),
    ("已保存 {key} = {}", "Saved {key} = {}"),
    ("已修复 {}", "Fixed {}"),
    ("已写入 {path}", "Wrote {path}"),
    ("已切换到「{}」模式：{}", "Switched to \"{}\" mode: {}"),
    ("已删除「{title}」", "Deleted \"{title}\""),
    ("已取消", "Cancelled"),
    ("已取消，配置没有改动", "Cancelled, the config is unchanged"),
    ("已开始新对话", "Started a new conversation"),
    (
        "已开始新对话，之前的对话已保存，可用 /resume 或 /sessions 找回",
        "Started a new conversation; the old one is saved, /resume or /sessions brings it back",
    ),
    (
        "已恢复对话「{}」· {} 轮 · {}",
        "Resumed \"{}\" · {} rounds · {}",
    ),
    ("已清空对话历史", "Conversation history cleared"),
    ("已经在这个对话里了", "Already in this conversation"),
    ("已跳过 {}", "Skipped {}"),
    (
        "已选中 {} 个修复：{}（未写入文件 · {reason}）",
        "{} fixes selected: {} (nothing written · {reason})",
    ),
    (
        "已选中的保留，其余不修复",
        "Keep the selected ones, skip the rest",
    ),
    (
        "带关键词打开历史对话：/search 关键词",
        "Open history with a keyword: /search <keyword>",
    ),
    ("序列化对话失败", "Serialising the conversation failed"),
    ("序列化扫描结果失败", "Serialising the scan results failed"),
    ("序列化配置失败", "Serialising the config failed"),
    ("应用", "Apply"),
    ("应用对 {} 的改动？", "Apply the change to {}?"),
    ("应用此条及之后全部", "Apply this and everything after it"),
    ("应用这个修复？", "Apply this fix?"),
    (
        "建立仓库索引失败，跳过模型判定",
        "Building the repository index failed, skipping model judgement",
    ),
    ("建议：{}", "Suggestion: {}"),
    (
        "开始新对话（旧对话会保留）",
        "Start a new conversation (the old one is kept)",
    ),
    ("当前「{}」", "Currently \"{}\""),
    ("当前主题：{}", "Current theme: {}"),
    ("当前字体风格：{}", "Current font style: {}"),
    ("当前思考模式：{}", "Current thinking mode: {}"),
    (
        "当前思考模式：{}（/model 可以连同模型一起保存）",
        "Current thinking mode: {} (/model saves it along with the model)",
    ),
    (
        "当前是计划模式 / 只读权限：会给出修复方案，但不会写入文件",
        "Plan mode / read-only: fixes are proposed but nothing is written",
    ),
    ("当前模型：{}", "Current model: {}"),
    ("当前模式：「{}」{}", "Current mode: \"{}\"{}"),
    ("思考中…", "Thinking…"),
    ("思考力度", "Thinking effort"),
    ("思考模式", "Thinking mode"),
    ("思考过程", "Thinking"),
    ("恢复上一次对话", "Resume the last conversation"),
    (
        "执行终端命令。只读 / 可编辑模式下仅允许查看类命令                 （rg、grep、find、fd、ls、tree、cat、head、tail、wc、diff、jq、                 git log/diff/show/status/blame 等），不能跑 python、sed、awk；                 full 模式下可执行任意命令（删除命令除外）",
        "Run a terminal command. In read-only / editable mode only inspection commands are allowed (rg, grep, find, fd, ls, tree, cat, head, tail, wc, diff, jq, git log/diff/show/status/blame, …); python, sed and awk are not; in full mode any command runs (except deletes)",
    ),
    ("扫描中…", "Scanning…"),
    ("扫描中（含模型判定）…", "Scanning (with model judgement)…"),
    ("扫描了 ", "Scanned "),
    (
        "扫描目录：本地规则；加 --ai 让模型逐条判定（/scan 路径 --ai）",
        "Scan a directory with local rules; add --ai for per-item model judgement (/scan <path> --ai)",
    ),
    (
        "找不到唯一匹配的字符串（找到 {} 处）",
        "No unique match for that string ({} matches)",
    ),
    (
        "把字符串当代码执行，输入可控时等同于任意代码执行",
        "Running strings as code is arbitrary code execution whenever the input is attacker-controlled",
    ),
    (
        "把扫描结果导出成 .md + .json（/report 路径）",
        "Export the scan results as .md + .json (/report <path>)",
    ),
    (
        "报告已写入 {}（共 {} 个问题）",
        "Report written to {} ({} issues)",
    ),
    (
        "接口地址不存在，请输入 /model 检查 API 地址和模型名",
        "That endpoint does not exist; check the base URL and model name with /model",
    ),
    ("提示", "Tip"),
    (
        "提示：命令行里的 API Key 可能被记录到 shell 历史，省略 value 可改为交互输入",
        "Tip: an API key on the command line can end up in shell history; omit the value to be prompted instead",
    ),
    ("搜索 ", "Search "),
    (
        "文件一个都没变：直接复用上次的 {} 条判定，这一轮不发任何请求",
        "No file changed: reusing the {} verdicts from last time, no request this round",
    ),
    ("文件不存在", "the file does not exist"),
    ("文件内容", "File content"),
    ("文件只有 {} 行", "The file has only {} lines"),
    (
        "文件都没变，可以直接 /fix",
        "No file changed, /fix can run right away",
    ),
    (
        "新窗口打开外链却没带 rel=noopener，对方页面可以操作本页",
        "A new-window link without rel=noopener lets the other page control this one",
    ),
    ("无权访问该接口", "No access to that endpoint"),
    ("无法创建模型客户端", "Cannot create the model client"),
    ("无法判断", "undetermined"),
    ("无法确定用户主目录", "Cannot determine the home directory"),
    ("无法解析响应数据：{}", "Cannot parse the response: {}"),
    ("无法连接 {}", "Cannot connect to {}"),
    ("显示帮助", "Show help"),
    ("替换为此内容", "Replace with this"),
    ("最大思考强度，最慢", "Maximum thinking effort, slowest"),
    (
        "有 {skipped} 个对话文件无法读取，已跳过（{}）",
        "{skipped} conversation files could not be read and were skipped ({})",
    ),
    (
        "有 {} 个文件变过，只把它们重新交给模型（其余文件沿用上次结论）：{}",
        "{} files changed; only those go back to the model (the rest keep their previous verdicts): {}",
    ),
    (
        "有 {} 个文件在扫描后改过（{}{}）：相关发现可能过期，/fix 会跳过它们",
        "{} files changed since the scan ({}{}): related findings may be stale, /fix will skip them",
    ),
    ("服务暂时不可用", "Service temporarily unavailable"),
    ("服务端内部错误", "Server internal error"),
    ("服务端返回错误：{}", "The server returned an error: {}"),
    ("未发现安全问题", "No security issues found"),
    (
        "未知命令 /{name}，/help 查看可用命令",
        "Unknown command /{name}; /help lists the commands",
    ),
    (
        "未知命令 /{other}，/help 查看可用命令",
        "Unknown command /{other}; /help lists the commands",
    ),
    ("未知工具：{name}", "Unknown tool: {name}"),
    (
        "未知配置项 `{key}`，可选：{}",
        "Unknown config key `{key}`; options: {}",
    ),
    ("未设置", "not set"),
    (
        "未配置 API Key：运行 `vibeguard config` 进行设置，或导出环境变量 DEEPSEEK_API_KEY",
        "API key not configured: run `vibeguard config`, or export DEEPSEEK_API_KEY",
    ),
    (
        "本次会话思考模式：{mode}",
        "Thinking mode for this session: {mode}",
    ),
    (
        "本次会话改用模型 {model}（持久保存请用 /config set model）",
        "Switched this session to model {model} (use /config set model to persist)",
    ),
    ("本次全部应用", "Apply all this run"),
    (
        "本次未发现已确认的问题。\n\n",
        "No confirmed issues this run.\n\n",
    ),
    ("本次运行覆盖", "This run covers"),
    ("朴素框线，圆点标记", "Plain frame, dot markers"),
    (
        "权限不足：不允许写文件",
        "Permission denied: writing files is not allowed",
    ),
    (
        "权限不足：不允许编辑文件",
        "Permission denied: editing files is not allowed",
    ),
    ("权限模式已保存：{}", "Permission mode saved: {}"),
    ("权限模式是 readonly", "permission mode is readonly"),
    ("权限设置", "Permissions"),
    ("查看或修改配置", "Show or change the config"),
    ("查看或切换思考模式", "Show or switch the thinking mode"),
    (
        "查看或切换思考模式（仅本次）",
        "Show or switch the thinking mode (this session only)",
    ),
    (
        "查看或切换思考模式：auto / off / low / high / max",
        "Show or switch the thinking mode: auto / off / low / high / max",
    ),
    (
        "查看或切换本次会话的模型",
        "Show or switch the model for this session",
    ),
    ("查看或切换模型", "Show or switch the model"),
    (
        "查看配置 / 运行配置向导",
        "Show the config / run the setup wizard",
    ),
    ("模型", "Model"),
    (
        "模型 `{model}` 已被 DeepSeek 弃用，建议改用 {replacement}",
        "Model `{model}` is deprecated by DeepSeek; {replacement} is recommended instead",
    ),
    (
        "模型不能为空，例如 deepseek-flash",
        "model must not be empty, e.g. deepseek-flash",
    ),
    (
        "模型判定 {}/{total}：{}（{} 条）",
        "Model judgement {}/{total}: {} ({} items)",
    ),
    ("模型判定了 ", "Model judged "),
    (
        "模型判定结束，共消耗 {tokens} tokens",
        "Model judgement finished, {tokens} tokens used",
    ),
    (
        "模型名不能包含空白字符",
        "model must not contain whitespace",
    ),
    ("模型名称", "Model name"),
    ("模型没有返回 JSON", "The model returned no JSON"),
    (
        "模型没有返回可以解析的文件内容",
        "The model returned no parseable file content",
    ),
    ("模型返回的 JSON 不完整", "The model's JSON is incomplete"),
    (
        "模型返回的 JSON 解析失败",
        "Parsing the model's JSON failed",
    ),
    ("模型配置", "Model config"),
    (
        "模式可选值为 plan / confirm / auto",
        "mode must be plan / confirm / auto",
    ),
    (
        "正在为 {total} 个问题生成修复（每个问题一次请求，不会写文件）…",
        "Generating fixes for {total} issues (one request each, nothing is written)…",
    ),
    (
        "正在忙，等这次结束或者按 Esc 取消",
        "Busy; wait for this to finish or press Esc to cancel",
    ),
    (
        "正在扫描 {}（本地规则，只读）…",
        "Scanning {} (local rules, read-only)…",
    ),
    ("每改一处都先确认", "Confirm every change"),
    ("没有匹配的对话", "No matching conversation"),
    (
        "没有可以修的发现（文件不存在，或者编号对不上）",
        "Nothing fixable (files missing, or the IDs do not match)",
    ),
    ("没有可确认的修复", "No fixes to confirm"),
    (
        "没有生成出可用的修复：模型要么没改，要么返回的内容不完整",
        "No usable fix was produced: the model either changed nothing or returned incomplete content",
    ),
    ("没有选中任何修复", "No fixes selected"),
    ("浅色", "Light"),
    ("浅色底，深色字", "Light background, dark text"),
    ("测试 API 连接", "Test the API connection"),
    ("消息列表不能为空", "The message list must not be empty"),
    ("深度思考", "Deep thinking"),
    ("深色", "Dark"),
    ("深色底，亮色字", "Dark background, light text"),
    ("清空对话历史", "Clear the conversation history"),
    (
        "漏报（规则没命中）：{missed:?}；实际命中：{got:?}",
        "Missed (the rule did not fire): {missed:?}; actually matched: {got:?}",
    ),
    ("演示数据", "Demo data"),
    (
        "演示模式：以下是示例数据，不会读取或修改任何文件",
        "Demo mode: the data below is sample data, no file is read or changed",
    ),
    (
        "特权容器或挂载 docker.sock，等于把宿主机交给容器",
        "A privileged container, or a mounted docker.sock, hands the host to the container",
    ),
    ("环境变量 {name}", "Env var {name}"),
    (
        "环境变量 {name} 已设置，会覆盖配置文件里的{label}；表单显示的是配置文件里的值",
        "Env var {name} is set and overrides the {label} in the config file; the form shows the config value",
    ),
    (
        "环境变量 {name} 已设置，它会覆盖配置文件中的 api_key",
        "Env var {name} is set and overrides api_key in the config",
    ),
    (
        "环境变量 {name} 已设置，它会覆盖配置文件中的 {key}",
        "Env var {name} is set and overrides {key} in the config",
    ),
    (
        "环境变量 {name} 已设置，运行时它仍会覆盖刚保存的{label}",
        "Env var {name} is set and will still override the {label} you just saved",
    ),
    (
        "环境变量文件被提交进了仓库，里面通常含真实密钥",
        "A committed env file usually contains real secrets",
    ),
    ("现在测试 API 连接？", "Test the API connection now?"),
    ("生成中…", "Generating…"),
    ("生成修复中…", "Generating fixes…"),
    (
        "用户拒绝了这个改动。不要重试，继续处理下一个问题。",
        "The user declined this change. Do not retry; move on to the next issue.",
    ),
    ("用法：", "Usage: "),
    (
        "用法：/config [show|init|set <key> [value]|unset <key>]",
        "Usage: /config [show|init|set <key> [value]|unset <key>]",
    ),
    (
        "用法：/config [show|set <key> <value>|unset <key>]",
        "Usage: /config [show|set <key> <value>|unset <key>]",
    ),
    (
        "用法：/font [default|anime]",
        "Usage: /font [default|anime]",
    ),
    (
        "用法：/mode [plan|confirm|auto]",
        "Usage: /mode [plan|confirm|auto]",
    ),
    ("用法：/model [name]", "Usage: /model [name]"),
    (
        "用法：/model 打开配置表单，或 /model <模型名> 直接换模型",
        "Usage: /model opens the form, or /model <name> to switch directly",
    ),
    (
        "用法：/permissions [readonly|editable|full] [工作目录]",
        "Usage: /permissions [readonly|editable|full] [dir]",
    ),
    ("用法：/theme [dark|light]", "Usage: /theme [dark|light]"),
    (
        "用法：/thinking [auto|off|low|high|max]",
        "Usage: /thinking [auto|off|low|high|max]",
    ),
    (
        "用用户提供的文件名拼路径，`../../` 可以写到目录外",
        "Paths are built from user-supplied filenames, so `../../` can write outside the directory",
    ),
    (
        "由服务端决定（DeepSeek 默认开启思考）",
        "Decided by the server (DeepSeek enables thinking by default)",
    ),
    ("界面主题已切换为：{theme}", "Theme switched to: {theme}"),
    ("界面主题：{}", "Theme: {}"),
    (
        "界面语言：英文（默认）/ 中文",
        "Language: English (default) / Chinese",
    ),
    ("疑似", "suspected"),
    ("疑似 {suspected}", "{suspected} suspected"),
    (
        "直接写入 HTML，用户输入会变成 XSS",
        "Writing straight into HTML turns user input into XSS",
    ),
    (
        "直接输入修改 · ↑↓ 切换项 · Enter 下一项 · Ctrl-S 保存 · Esc 取消",
        "Type to edit · ↑↓ switch · Enter next · Ctrl-S save · Esc cancel",
    ),
    (
        "直接输入问题与 AI 对话，/help 查看命令",
        "Type a question to talk to the AI; /help lists the commands",
    ),
    (
        "直接输入问题与 AI 对话，输入 / 查看命令",
        "Type a question to talk to the AI; type / for commands",
    ),
    (
        "相对于工作目录或绝对路径",
        "relative to the working directory, or an absolute path",
    ),
    (
        "相对于工作目录或绝对路径，默认为 .",
        "relative to the working directory, or an absolute path; defaults to .",
    ),
    ("确认", "confirm"),
    (
        "空 catch 会吞掉异常，故障时既没有日志也没有失败信号",
        "An empty catch swallows the exception, so a failure leaves neither logs nor a signal",
    ),
    ("等你确认…", "Waiting for you…"),
    ("等待响应…", "Waiting for a response…"),
    ("结束确认", "End review"),
    ("结果已存档：{}", "Results archived: {}"),
    (
        "编辑文件：查找并替换字符串",
        "Edit a file: find and replace a string",
    ),
    ("缺少 cmd 参数", "missing cmd argument"),
    ("缺少 content 参数", "missing content argument"),
    ("缺少 new_str 参数", "missing new_str argument"),
    ("缺少 old_str 参数", "missing old_str argument"),
    ("缺少 path 参数", "missing path argument"),
    ("自动", "auto"),
    ("自定义…", "Custom…"),
    (
        "要查找的原始字符串（必须精确匹配一处）",
        "the original string to find (must match exactly once)",
    ),
    ("计划", "plan"),
    ("计划模式只读", "plan mode is read-only"),
    (
        "误报（命中了不该命中的）：{extra:?}",
        "False positive (matched what it should not): {extra:?}",
    ),
    (
        "请提供 value：/config set <key> <value>",
        "Provide a value: /config set <key> <value>",
    ),
    ("请求 {} 失败", "Request {} failed"),
    ("请求 {} 超时", "Request {} timed out"),
    (
        "请求参数错误，请输入 /model 检查模型和思考力度",
        "Bad request parameters; check the model and thinking effort with /model",
    ),
    ("请求失败", "Request failed"),
    (
        "请求失败，{:.1}s 后第 {attempt} 次重试：{reason}",
        "Request failed, retrying in {:.1}s (attempt {attempt}): {reason}",
    ),
    ("请求格式错误", "Malformed request"),
    (
        "请求过于频繁，已触发限流",
        "Rate limited; too many requests",
    ),
    (
        "请输入命令，/help 查看可用命令",
        "Type a command; /help lists them",
    ),
    ("读取", "Read"),
    (
        "读取响应流失败（连接中断或超时）",
        "Reading the response stream failed (connection dropped or timed out)",
    ),
    ("读取对话失败：{}", "Reading the conversation failed: {}"),
    (
        "读取对话目录失败：{}",
        "Reading the conversation directory failed: {}",
    ),
    ("读取扫描目录失败", "Reading the scan directory failed"),
    ("读取文件内容", "Read file contents"),
    ("读取配置文件失败：{}", "Reading the config file failed: {}"),
    (
        "调试模式上线会泄露堆栈、配置和内部路径",
        "Debug mode in production leaks stack traces, config and internal paths",
    ),
    (
        "账户余额不足，请到服务商的控制台充值",
        "Insufficient balance; top up in the provider console",
    ),
    ("跳过", "Skip"),
    (
        "跳过 {stale} 条：文件在扫描后改过，先重扫这些文件再修",
        "Skipping {stale} findings: their files changed since the scan, rescan them before fixing",
    ),
    (
        "跳过 {}：文件太大（{} 字节）",
        "Skipping {}: file too large ({} bytes)",
    ),
    (
        "跳过 {}：模型没改动任何内容",
        "Skipping {}: the model changed nothing",
    ),
    (
        "跳过 {}：返回的内容只有原文的 {}%，像是被截断了",
        "Skipping {}: the returned content is only {}% of the original, so it looks truncated",
    ),
    ("轻度思考", "Light thinking"),
    (
        "载入 {ago}的扫描结果（{}）· 共 {} 个问题",
        "Loading the scan from {ago} ({}) · {} issues",
    ),
    ("输入 {}", "Input {}"),
    ("输入关键词过滤", "Type a keyword to filter"),
    (
        "输入过滤 · ↑↓ 选择 · Enter 打开 · Del 删除 · Esc 关闭",
        "Type to filter · ↑↓ select · Enter open · Del delete · Esc close",
    ),
    ("输出 {}", "Output {}"),
    ("运行", "Run"),
    ("还没有任何扫描记录：{}", "No scan records yet: {}"),
    ("还没有历史对话", "No history yet"),
    ("还没有可以恢复的历史对话", "No conversation to resume yet"),
    (
        "还没有扫描结果，先跑 /scan 或 /scan --ai",
        "No scan results yet; run /scan or /scan --ai first",
    ),
    (
        "这个目录还没有扫描记录：{}（存档里有 {others} 条其他目录的记录）",
        "No scan record for this directory: {} (the archive has {others} records for other directories)",
    ),
    (
        "这些文件里另有 {} 条发现已作废（改动会让行号漂移），建议重新扫一次",
        "Another {} findings in these files are now void (edits shift line numbers); rescanning is recommended",
    ),
    (
        "这些规则还没有标定样例：{missing:?}（在 bench/fixtures 里加一个，并更新 expected.tsv）",
        "These rules have no seeded sample yet: {missing:?} (add one under bench/fixtures and update expected.tsv)",
    ),
    (
        "这份报告是「没发现问题」，不是「没有问题」——它只看了清单里的 {}/{} 条",
        "This report says \"nothing found\", not \"nothing wrong\" — it only covered {}/{} checklist items",
    ),
    (
        "这次请求里剩下的改动不再询问",
        "The rest of the changes in this request will not be asked about",
    ),
    ("连接 {}（{}）", "Connecting to {} ({})"),
    ("退出", "Quit"),
    ("退出（也可按 Ctrl-D）", "Quit (or Ctrl-D)"),
    ("逐条确认", "confirm each"),
    (
        "配置 API 地址、API Key、模型与思考力度",
        "Set the API base URL, key, model and thinking effort",
    ),
    (
        "配置 VibeGuard，保存到 {}",
        "Configure VibeGuard, saved to {}",
    ),
    ("配置已保存", "Config saved"),
    ("配置已保存到 {}", "Config saved to {}"),
    ("配置文件", "Config file"),
    ("配置文件 ", "Config file "),
    ("配置文件格式错误：{}", "Malformed config file: {}"),
    (
        "配置权限模式和工作目录",
        "Set the permission mode and working directory",
    ),
    ("重试（拆半）：{}（{} 条）", "Retry (split): {} ({} items)"),
    ("错误", "Error"),
    ("错误：{e:#}", "Error: {e:#}"),
    ("错误：权限不足", "Error: Permission denied"),
    (
        "非密码学随机数，不能用来生成 token、验证码或密钥",
        "A non-cryptographic RNG must not generate tokens, codes or keys",
    ),
    ("非法的对话编号：{id}", "Invalid conversation id: {id}"),
    ("非法的对话编号：{}", "Invalid conversation id: {}"),
    (
        "预览扫描与修复界面（示例数据）",
        "Preview the scan and fix UI (sample data)",
    ),
    ("高危", "High"),
    ("默认", "Default"),
    (
        "默认（启动时的当前目录）",
        "Default (the directory you started in)",
    ),
    (
        "（仓库里另有 {rest} 个文件与本批条目不相关，未列出）\n\n",
        "(the repository has {rest} more files unrelated to this batch, not listed)\n\n",
    ),
    (
        "（尚未创建，使用默认值）",
        "(not created yet, using defaults)",
    ),
    ("（思考 {r}）", "(thinking {r})"),
    ("（无标题）", "(untitled)"),
    ("（未给出证据）", "(no evidence given)"),
    ("（未设置）", "(not set)"),
    ("（缓存命中 {hit}）", "(cache hit {hit})"),
    (
        "（跳过 {files_skipped} 个：非文本、超大或已标注忽略）",
        "(skipped {files_skipped}: not text, too large, or marked ignored)",
    ),
    (
        "；/resume 可恢复上一次对话「{}」（{}）",
        "; /resume brings back the last conversation \"{}\" ({})",
    ),
    (
        "\n> ⚠ 有 {} 批判定没有跑成功，上面的覆盖数字**不完整**。\n> 修好网络/额度之后重跑 `/scan --ai`，否则这些条目等于没看。\n\n",
        "\n> ⚠ {} judgement batches failed, so the coverage numbers above are **incomplete**.\n> Rerun `/scan --ai` once the network or quota is fixed, otherwise those items were never checked.\n\n",
    ),
    (
        "> {} 条被判为「无法判断」：它们需要把服务跑起来发请求才能确认（限流、爆破、越权、响应头这类）。**不要把它们当成「没问题」。**\n\n",
        "> {} items were judged \"undetermined\": confirming them means running the service and sending requests (rate limiting, brute force, broken access control, response headers). **Do not read them as clean.**\n\n",
    ),
    (
        "- 本地规则命中是**确定**的：它看到了具体的代码特征（字符串里的密钥、`shell=True`、`eval` 调用）。\n- 「疑似」是模型读到相关代码但证据不足，需要人工确认。\n- 本报告由 VibeGuard 生成，是审计的**起点**不是结论；每条都要人看过再改。\n",
        "- A local rule hit is **certain**: it saw a concrete code pattern (a key in a string, `shell=True`, an `eval` call).\n- \"Suspected\" means the model read related code but the evidence was thin, so a human has to confirm.\n- VibeGuard generates this report as a **starting point**, not a verdict; read every item before acting.\n",
    ),
    (
        "权限不足：{} 模式只允许查看类命令（rg / find / ls / cat / head / tail / git log 等）。找文件用 `rg --files` 或 `find . -name \"*.ts\"`，搜内容用 `rg -n 模式 路径`；确实要执行其他命令，请让用户 `/config set permission_mode full`",
        "Permission denied: {} mode only allows inspection commands (rg / find / ls / cat / head / tail / git log, …). Find files with `rg --files` or `find . -name \"*.ts\"`, search content with `rg -n pattern path`; to run anything else, ask the user to `/config set permission_mode full`",
    ),
    (
        "这一轮已经调用工具 {max_rounds} 次，先停在这里。接着说「继续」我就往下做，或者 `/config set max_tool_rounds 300` 调高上限",
        "This round already made {max_rounds} tool calls, so I am stopping here. Say \"continue\" and I will carry on, or raise the limit with `/config set max_tool_rounds 300`",
    ),
    (
        "计划模式下不能修改文件：请把修复建议写进报告，由用户切换到逐条确认或全自动模式后再动手。",
        "Plan mode cannot change files: put the fix suggestion in the report and let the user switch to confirm or auto mode before touching anything.",
    ),
    (
        "错误：当前工作模式是 plan，它强制只读，改 permission_mode 不会生效。要执行这类命令，需要用户按 Shift-Tab 切到 confirm 或 auto；只想看代码的话用 rg / find / cat。",
        "Error: the current work mode is plan, which forces read-only, so changing permission_mode has no effect. To run commands like this the user has to press Shift-Tab for confirm or auto; to just read code, use rg / find / cat.",
    ),
    (
        "模型连着 {repeats} 次用同样的参数调用同一个工具，判断为原地打转，先停下来。换个说法，或者 /scan 看看现状",
        "The model called the same tool with the same arguments {repeats} times in a row, which looks like spinning in place, so I am stopping. Rephrase, or run /scan to see where things stand",
    ),
    (
        "当前工作模式是 plan（强制只读），这个权限暂时不生效：按 Shift-Tab 切到 confirm 或 auto",
        "The current work mode is plan (forced read-only), so this permission has no effect for now: press Shift-Tab for confirm or auto",
    ),
    ("界面语言", "Language"),
    ("默认语言", "Default"),
    ("中文界面", "Chinese interface"),
    ("是，信任它", "Yes, trust it"),
    (
        "可以在这里执行命令、修改文件",
        "commands and edits are allowed here",
    ),
    ("否，只读", "No, read-only"),
    (
        "本次会话不写文件、不跑命令",
        "this session cannot write files or run commands",
    ),
    ("强调色", "Accent"),
    ("默认的青", "the default teal"),
    ("樱花粉", "sakura pink"),
    ("海蓝", "ocean blue"),
    ("紫罗兰", "violet"),
    ("琥珀", "amber"),
    ("青柠", "lime"),
    ("无彩", "monochrome"),
    (
        "只读，不能修改文件或执行命令",
        "read-only: no edits, no commands",
    ),
    (
        "可编辑，能修改文件，不能执行命令",
        "editable: edits allowed, commands blocked",
    ),
    (
        "全权限，能修改文件和执行命令（删除除外）",
        "full: edits and commands, except deletes",
    ),
    ("界面主题", "Theme"),
    ("字形", "Glyphs"),
    ("权限模式", "Permission mode"),
    ("工作模式", "Work mode"),
];
