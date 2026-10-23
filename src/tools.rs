//! 文件和命令工具实现，供 AI 通过工具调用执行。

use crate::llm::Tool;
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    ReadOnly,
    Editable,
    Full,
}

impl PermissionMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ReadOnly => "只读",
            Self::Editable => "可编辑",
            Self::Full => "全权限",
        }
    }
}

/// 生成工具定义供模型调用
pub fn tool_definitions() -> Vec<Tool> {
    vec![
        Tool {
            name: "read_file".to_string(),
            description: "读取文件内容".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "相对于工作目录或绝对路径"
                    }
                },
                "required": ["path"]
            }),
        },
        Tool {
            name: "list_dir".to_string(),
            description: "列出目录内容".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "相对于工作目录或绝对路径，默认为 ."
                    }
                },
                "required": []
            }),
        },
        Tool {
            name: "write_file".to_string(),
            description: "写入或创建文件".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "相对于工作目录或绝对路径"
                    },
                    "content": {
                        "type": "string",
                        "description": "文件内容"
                    }
                },
                "required": ["path", "content"]
            }),
        },
        Tool {
            name: "edit_file".to_string(),
            description: "编辑文件：查找并替换字符串".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "相对于工作目录或绝对路径"
                    },
                    "old_str": {
                        "type": "string",
                        "description": "要查找的原始字符串（必须精确匹配一处）"
                    },
                    "new_str": {
                        "type": "string",
                        "description": "替换为此内容"
                    }
                },
                "required": ["path", "old_str", "new_str"]
            }),
        },
        Tool {
            name: "run_command".to_string(),
            description: "执行终端命令（需要全权限，删除命令除外）".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "cmd": {
                        "type": "string",
                        "description": "命令名（如 ls, git 等）"
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "命令参数"
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
        Err(e) => format!("错误：{e:#}"),
    }
}

fn _execute_tool(name: &str, args_json: &str, mode: &PermissionMode) -> Result<Value> {
    match name {
        "read_file" => {
            let args: serde_json::Map<String, Value> =
                serde_json::from_str(args_json).map_err(|e| anyhow!("参数格式错误：{e}"))?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("缺少 path 参数"))?;
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
                bail!("权限不足：不允许写文件")
            }
            let args: serde_json::Map<String, Value> =
                serde_json::from_str(args_json).map_err(|e| anyhow!("参数格式错误：{e}"))?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("缺少 path 参数"))?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("缺少 content 参数"))?;
            write_file(path, content)
        }
        "edit_file" => {
            if matches!(mode, PermissionMode::ReadOnly) {
                bail!("权限不足：不允许编辑文件")
            }
            let args: serde_json::Map<String, Value> =
                serde_json::from_str(args_json).map_err(|e| anyhow!("参数格式错误：{e}"))?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("缺少 path 参数"))?;
            let old = args
                .get("old_str")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("缺少 old_str 参数"))?;
            let new = args
                .get("new_str")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("缺少 new_str 参数"))?;
            edit_file(path, old, new)
        }
        "run_command" => {
            if !matches!(mode, PermissionMode::Full) {
                bail!("权限不足：不允许执行命令")
            }
            let args: serde_json::Map<String, Value> =
                serde_json::from_str(args_json).map_err(|e| anyhow!("参数格式错误：{e}"))?;
            let cmd = args
                .get("cmd")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("缺少 cmd 参数"))?;
            let cmd_args: Vec<&str> = args
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();

            // 检查删除命令
            if is_delete_command(cmd, &cmd_args) {
                bail!("删除命令需要用户确认（当前不支持自动执行）")
            }
            run_command(cmd, &cmd_args)
        }
        _ => bail!("未知工具：{name}"),
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
            "找不到唯一匹配的字符串（找到 {} 处）",
            content.matches(old).count()
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
