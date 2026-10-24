//! 文件和命令工具实现，供 AI 通过工具调用执行。

use crate::i18n::{pick, tr};
use crate::llm::Tool;
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    /// 默认只读：装好就能扫，但什么都改不了
    #[default]
    ReadOnly,
    Editable,
    Full,
}

impl PermissionMode {
    pub const ALL: [Self; 3] = [Self::ReadOnly, Self::Editable, Self::Full];

    pub fn label(&self) -> &'static str {
        match self {
            Self::ReadOnly => tr("只读"),
            Self::Editable => tr("可编辑"),
            Self::Full => tr("全权限"),
        }
    }

    /// 写进配置文件的写法，和界面显示的语言无关
    pub fn code(self) -> &'static str {
        match self {
            Self::ReadOnly => "readonly",
            Self::Editable => "editable",
            Self::Full => "full",
        }
    }

    /// 菜单里那一行说明。三个模式的区别就在「能不能改文件、能不能跑命令」，
    /// 所以每条都把这三点说全，别让人靠名字猜。
    pub fn desc(self) -> &'static str {
        match self {
            Self::ReadOnly => tr("只读，不能修改文件或执行命令"),
            Self::Editable => tr("可编辑，能修改文件，不能执行命令"),
            Self::Full => tr("全权限，能修改文件和执行命令（删除除外）"),
        }
    }
}

/// 生成工具定义供模型调用
pub fn tool_definitions() -> Vec<Tool> {
    vec![
        Tool {
            name: "read_file".to_string(),
            description: tr("读取文件内容").to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": tr("相对于工作目录或绝对路径")
                    }
                },
                "required": ["path"]
            }),
        },
        Tool {
            name: "list_dir".to_string(),
            description: tr("列出目录内容").to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": tr("相对于工作目录或绝对路径，默认为 .")
                    }
                },
                "required": []
            }),
        },
        Tool {
            name: "write_file".to_string(),
            description: tr("写入或创建文件").to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": tr("相对于工作目录或绝对路径")
                    },
                    "content": {
                        "type": "string",
                        "description": tr("文件内容")
                    }
                },
                "required": ["path", "content"]
            }),
        },
        Tool {
            name: "edit_file".to_string(),
            description: tr("编辑文件：查找并替换字符串").to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": tr("相对于工作目录或绝对路径")
                    },
                    "old_str": {
                        "type": "string",
                        "description": tr("要查找的原始字符串（必须精确匹配一处）")
                    },
                    "new_str": {
                        "type": "string",
                        "description": tr("替换为此内容")
                    }
                },
                "required": ["path", "old_str", "new_str"]
            }),
        },
        Tool {
            name: "run_command".to_string(),
            description: tr("执行终端命令。只读 / 可编辑模式下仅允许查看类命令                 （rg、grep、find、fd、ls、tree、cat、head、tail、wc、diff、jq、                 git log/diff/show/status/blame 等），不能跑 python、sed、awk；                 full 模式下可执行任意命令（删除命令除外）")
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cmd": {
                        "type": "string",
                        "description": tr("命令名（如 ls, git 等）")
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": tr("命令参数")
                    }
                },
                "required": ["cmd"]
            }),
        },
    ]
}

/// 执行工具，接收 JSON 参数，返回结果字符串
pub fn execute_tool(name: &str, args_json: &str, mode: &PermissionMode) -> String {
    match _execute_tool(name, args_json, mode) {
        Ok(result) => result.to_string(),
        Err(e) => pick(format!("错误：{e:#}"), format!("Error: {e:#}")),
    }
}

fn _execute_tool(name: &str, args_json: &str, mode: &PermissionMode) -> Result<Value> {
    match name {
        "read_file" => {
            let args: serde_json::Map<String, Value> =
                serde_json::from_str(args_json).map_err(|e| {
                    anyhow!(
                        "{}",
                        pick(format!("参数格式错误：{e}"), format!("Bad arguments: {e}"))
                    )
                })?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("{}", tr("缺少 path 参数")))?;
            read_file(path)
        }
        "list_dir" => {
            let args: serde_json::Map<String, Value> =
                serde_json::from_str(args_json).unwrap_or_default();
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            list_dir(path)
        }
        "write_file" => {
            if matches!(mode, PermissionMode::ReadOnly) {
                bail!("{}", tr("权限不足：不允许写文件"))
            }
            let args: serde_json::Map<String, Value> =
                serde_json::from_str(args_json).map_err(|e| {
                    anyhow!(
                        "{}",
                        pick(format!("参数格式错误：{e}"), format!("Bad arguments: {e}"))
                    )
                })?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("{}", tr("缺少 path 参数")))?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("{}", tr("缺少 content 参数")))?;
            write_file(path, content)
        }
        "edit_file" => {
            if matches!(mode, PermissionMode::ReadOnly) {
                bail!("{}", tr("权限不足：不允许编辑文件"))
            }
            let args: serde_json::Map<String, Value> =
                serde_json::from_str(args_json).map_err(|e| {
                    anyhow!(
                        "{}",
                        pick(format!("参数格式错误：{e}"), format!("Bad arguments: {e}"))
                    )
                })?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("{}", tr("缺少 path 参数")))?;
            let old = args
                .get("old_str")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("{}", tr("缺少 old_str 参数")))?;
            let new = args
                .get("new_str")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("{}", tr("缺少 new_str 参数")))?;
            edit_file(path, old, new)
        }
        "run_command" => {
            let args: serde_json::Map<String, Value> =
                serde_json::from_str(args_json).map_err(|e| {
                    anyhow!(
                        "{}",
                        pick(format!("参数格式错误：{e}"), format!("Bad arguments: {e}"))
                    )
                })?;
            let cmd = args
                .get("cmd")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("{}", tr("缺少 cmd 参数")))?;
            let cmd_args: Vec<&str> = args
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();

            // 只读 / 可编辑模式下放行「只看不改」的命令。
            // 没有 rg，模型面对一个陌生仓库只能一个文件一个文件读——
            // 几百个文件的项目，几十轮都读不完，然后撞上限。
            if !matches!(mode, PermissionMode::Full) && !is_read_only_command(cmd, &cmd_args) {
                bail!(
                    "{}",
                    pick(
                        format!(
                            "权限不足：{} 模式只允许查看类命令（rg / find / ls / cat / head / tail / \
                     git log 等）。找文件用 `rg --files` 或 `find . -name \"*.ts\"`，\
                     搜内容用 `rg -n 模式 路径`；确实要执行其他命令，\
                     请让用户 `/config set permission_mode full`",
                            mode.label()
                        ),
                        format!(
                            "Permission denied: {} mode only allows inspection commands (rg / find / ls / cat / head / tail / git log, …). Find files with `rg --files` or `find . -name \"*.ts\"`, search content with `rg -n pattern path`; to run anything else, ask the user to `/config set permission_mode full`",
                            mode.label()
                        )
                    )
                )
            }

            // 检查删除命令
            if is_delete_command(cmd, &cmd_args) {
                bail!("{}", tr("删除命令需要用户确认（当前不支持自动执行）"))
            }
            run_command(cmd, &cmd_args)
        }
        _ => bail!(
            "{}",
            pick(format!("未知工具：{name}"), format!("Unknown tool: {name}"))
        ),
    }
}

fn read_file(path: &str) -> Result<Value> {
    let content = std::fs::read_to_string(path)?;
    Ok(json!({ "content": content }))
}

fn list_dir(path: &str) -> Result<Value> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        let metadata = entry.metadata()?;
        let item = json!({
            "name": name.to_string_lossy(),
            "is_dir": metadata.is_dir(),
            "size": metadata.len(),
        });
        entries.push(item);
    }
    entries.sort_by(|a, b| {
        let a_is_dir = a["is_dir"].as_bool().unwrap_or(false);
        let b_is_dir = b["is_dir"].as_bool().unwrap_or(false);
        if a_is_dir != b_is_dir {
            b_is_dir.cmp(&a_is_dir)
        } else {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        }
    });
    Ok(json!({ "entries": entries }))
}

fn write_file(path: &str, content: &str) -> Result<Value> {
    std::fs::write(path, content)?;
    Ok(json!({ "status": "ok" }))
}

fn edit_file(path: &str, old: &str, new: &str) -> Result<Value> {
    let mut content = std::fs::read_to_string(path)?;
    if content.matches(old).count() != 1 {
        bail!(
            "{}",
            pick(
                format!(
                    "找不到唯一匹配的字符串（找到 {} 处）",
                    content.matches(old).count()
                ),
                format!(
                    "No unique match for that string ({} matches)",
                    content.matches(old).count()
                )
            )
        )
    }
    content = content.replacen(old, new, 1);
    std::fs::write(path, &content)?;
    Ok(json!({ "status": "ok" }))
}

fn run_command(cmd: &str, args: &[&str]) -> Result<Value> {
    let output = std::process::Command::new(cmd).args(args).output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    Ok(json!({
        "exit_code": output.status.code(),
        "stdout": stdout.to_string(),
        "stderr": stderr.to_string(),
    }))
}

/// 只读 / 可编辑模式下允许执行的「查看类」命令。
/// 找文件（find / fd）、看内容（cat / head / tail）、搜内容（rg / grep）、
/// 看历史（git log / diff）都在这里——缺一个，模型就会绕路去读整个文件。
pub const READ_ONLY_COMMANDS: &[&str] = &[
    "rg",
    "grep",
    "egrep",
    "fgrep",
    "ag",
    "ack",
    "find",
    "fd",
    "ls",
    "tree",
    "file",
    "stat",
    "du",
    "df",
    "wc",
    "pwd",
    "date",
    "uname",
    "whoami",
    "which",
    "realpath",
    "basename",
    "dirname",
    "cat",
    "head",
    "tail",
    "nl",
    "cut",
    "diff",
    "cmp",
    "jq",
    "md5",
    "shasum",
    "sha256sum",
];

/// 只读模式下允许执行的「查看类」命令。
///
/// 用白名单而不是黑名单：这个模式的意义是「绝不改变磁盘」，
/// 而「这条命令不会写」太难判断——`sort -o`、`find -delete`、
/// `git diff --output` 都能悄悄落盘，漏一个就破了保证。
fn is_read_only_command(cmd: &str, args: &[&str]) -> bool {
    let base = Path::new(cmd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(cmd);

    // 这些参数能把「只读命令」变成写操作
    const WRITES: &[&str] = &[
        "-delete",
        "-exec",
        "-execdir",
        "-ok",
        "-okdir",
        "-fprint",
        "-fprintf",
        "--output",
        "--in-place",
    ];
    if args
        .iter()
        .any(|a| WRITES.contains(a) || a.starts_with("--output="))
    {
        return false;
    }

    // git 只放行只读子命令：push / commit / checkout / clean 一律不放
    if base == "git" {
        return args.first().is_some_and(|a| {
            matches!(
                *a,
                "log"
                    | "show"
                    | "diff"
                    | "status"
                    | "blame"
                    | "grep"
                    | "ls-files"
                    | "rev-parse"
                    | "shortlog"
                    | "describe"
            )
        });
    }
    READ_ONLY_COMMANDS.contains(&base)
}

fn is_delete_command(cmd: &str, args: &[&str]) -> bool {
    let cmd_base = Path::new(cmd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(cmd);

    match cmd_base {
        "rm" | "rmdir" => true,
        "git" => args.first().is_some_and(|&a| a == "clean" || a == "reset"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_mode_can_look_but_not_touch() {
        // 扫描陌生仓库没有 rg 就只能一个文件一个文件读，几十轮都读不完
        let ok = execute_tool(
            "run_command",
            r#"{"cmd":"ls","args":["."]}"#,
            &PermissionMode::ReadOnly,
        );
        assert!(!ok.contains("权限不足"), "{ok}");

        let ok = execute_tool(
            "run_command",
            r#"{"cmd":"rg","args":["--files","-g","*.rs"]}"#,
            &PermissionMode::ReadOnly,
        );
        assert!(!ok.contains("权限不足"), "{ok}");

        let git = execute_tool(
            "run_command",
            r#"{"cmd":"git","args":["log","--oneline","-3"]}"#,
            &PermissionMode::ReadOnly,
        );
        assert!(!git.contains("权限不足"), "{git}");

        // find 是最基础的查找命令：漏掉它，模型就只能去读整个目录
        let find = execute_tool(
            "run_command",
            r#"{"cmd":"find","args":[".","-maxdepth","1","-name","Cargo.toml"]}"#,
            &PermissionMode::ReadOnly,
        );
        assert!(!find.contains("权限不足"), "{find}");
    }

    #[test]
    fn a_refused_command_says_what_to_use_instead() {
        let out = execute_tool(
            "run_command",
            r#"{"cmd":"sed","args":["-i","s/a/b/","x"]}"#,
            &PermissionMode::ReadOnly,
        );
        assert!(out.contains("权限不足"));
        assert!(out.contains("rg --files"), "要给出替代写法：{out}");
        assert!(
            out.contains("permission_mode full"),
            "也要说明怎么放开：{out}"
        );
    }

    #[test]
    fn the_read_only_list_covers_the_obvious_ways_to_look_around() {
        for cmd in ["find", "fd", "rg", "ls", "cat", "head", "tail", "wc", "git"] {
            assert!(
                READ_ONLY_COMMANDS.contains(&cmd) || cmd == "git",
                "{cmd} 应当能查"
            );
        }
    }

    #[test]
    fn read_only_mode_refuses_anything_that_could_write() {
        for args in [
            r#"{"cmd":"python3","args":["-c","print(1)"]}"#,
            r#"{"cmd":"git","args":["push"]}"#,
            r#"{"cmd":"find","args":[".","-delete"]}"#,
            r#"{"cmd":"git","args":["diff","--output=/tmp/x"]}"#,
            r#"{"cmd":"sh","args":["-c","ls"]}"#,
        ] {
            let out = execute_tool("run_command", args, &PermissionMode::ReadOnly);
            assert!(out.contains("权限不足"), "应当拒绝：{args} → {out}");
        }
    }

    #[test]
    fn editable_mode_gets_the_same_read_only_allowance() {
        let out = execute_tool(
            "run_command",
            r#"{"cmd":"ls","args":["."]}"#,
            &PermissionMode::Editable,
        );
        assert!(!out.contains("权限不足"), "{out}");
        let out = execute_tool(
            "run_command",
            r#"{"cmd":"rm","args":["-rf","/tmp/nope"]}"#,
            &PermissionMode::Editable,
        );
        assert!(out.contains("权限不足"), "{out}");
    }

    #[test]
    fn read_only_blocks_write() {
        let result = execute_tool(
            "write_file",
            r#"{"path":"test.txt","content":"hi"}"#,
            &PermissionMode::ReadOnly,
        );
        assert!(result.contains("权限不足"));
    }

    #[test]
    fn recognizes_delete_commands() {
        assert!(is_delete_command("rm", &[]));
        assert!(is_delete_command("git", &["clean"]));
        assert!(!is_delete_command("ls", &[]));
    }
}
