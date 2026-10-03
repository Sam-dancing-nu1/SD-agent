//! 四工具 + 目录 schema（project-structure.md 决策 7）。
//!
//! 执行面收口（决策 3 机制形态磨合点，实现期实测落定）：
//! Rust 可见性只能限定到**祖先**模块（E0742），`pub(in crate::policy)`
//! 字面形态物理不可行（policy 不是 tools 的祖先）。收口等价形态：
//! 各工具 execute 为 `pub(super)`（对 tools/mod.rs 可见、对 policy 不可见），
//! 面向 policy 的唯一接口 = `run_tool()`（pub(crate)）。
//! 收口强度的诚实口径：execute 层编译期闭合（policy 不可见）；run_tool 层
//! 为单接口 + 审查约定（pub(crate) 全 crate 可达，闭合性靠"只允许
//! policy::dispatch 调用"的调用纪律与 grep 校验，非编译期绝对闭合）。
//! verify / doctor 的确定性执行是 Harness 侧独立窄通道，不经本模块。
//!
//! 工具目录序列化字节稳定（固定字段序 + 断言两次序列化字节相同），
//! 防毁前缀缓存（靶子 18 / 审查 #23）。

pub mod bash;
pub mod edit;
pub mod fs_guard;
pub mod read;
pub mod write;

use serde::Serialize;

/// 工具输出截断上限（单点常量，问题⑥：散落三处的截断常量收敛到此处统一
/// 引用；read / bash 共用口径 16 KiB）。事件摘要是另一概念，走
/// `config::DisciplineTables::output_digest_threshold_bytes`（policy::digest_of）。
pub(crate) const MAX_TOOL_OUTPUT_BYTES: usize = 16 * 1024;

/// 工具目录条目（序列化字段序固定：name → description → parameters）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema（object）。
    pub parameters: serde_json::Value,
}

/// 统一工具输出（policy 层消费；执行函数的返回形态）。
#[derive(Debug)]
pub(crate) struct ToolOutput {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub text: String,
}

/// 统一工具执行错误（执行失败 ≠ 工具拒绝；拒绝走 dispatch 前置裁决）。
#[derive(Debug)]
pub(crate) enum ToolError {
    Io(String),
    TimedOut { secs: u64 },
    Spawn(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::Io(m) => write!(f, "io error: {m}"),
            ToolError::TimedOut { secs } => write!(f, "timed out after {secs}s"),
            ToolError::Spawn(m) => write!(f, "spawn failed: {m}"),
        }
    }
}

/// 面向 policy 的唯一执行接口（调用纪律：只允许 policy::dispatch；
/// 可见性收口机制见文件头"执行面收口"）。参数解析到各工具强类型 Args
/// 后调受限可见性 execute，policy 不可绕过。
pub(crate) async fn run_tool(
    workspace_root: &std::path::Path,
    env: &crate::config::env::EnvProfile,
    name: &str,
    args: &serde_json::Value,
) -> Result<ToolOutput, ToolError> {
    match name {
        "read" => {
            let a: read::ReadArgs =
                serde_json::from_value(args.clone()).map_err(|e| ToolError::Io(e.to_string()))?;
            read::execute(workspace_root, env.os_family, &a)
        }
        "write" => {
            let a: write::WriteArgs =
                serde_json::from_value(args.clone()).map_err(|e| ToolError::Io(e.to_string()))?;
            write::execute(workspace_root, env.os_family, &a)
        }
        "edit" => {
            let a: edit::EditArgs =
                serde_json::from_value(args.clone()).map_err(|e| ToolError::Io(e.to_string()))?;
            edit::execute(workspace_root, env.os_family, &a)
        }
        "bash" => {
            let a: bash::BashArgs =
                serde_json::from_value(args.clone()).map_err(|e| ToolError::Io(e.to_string()))?;
            bash::execute(workspace_root, env, &a).await
        }
        other => Err(ToolError::Io(format!("unknown tool: {other}"))),
    }
}

/// 工具目录（四件，白名单与 config::DisciplineTables 对齐）。
pub fn catalog() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read",
            description: "Read a UTF-8 text file inside the workspace. Paths are workspace-relative.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "workspace-relative file path"},
                    "offset_lines": {"type": "integer", "minimum": 0},
                    "limit_lines": {"type": "integer", "minimum": 1}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "write",
            description: "Write a UTF-8 text file inside the workspace (creates or overwrites).",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "edit",
            description: "Replace an exact string in a workspace file. old_string must appear exactly once unless replace_all is true.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "old_string": {"type": "string"},
                    "new_string": {"type": "string"},
                    "replace_all": {"type": "boolean", "default": false}
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "bash",
            description: "Run a shell command with the workspace as cwd (executor decided by the env profile). Dangerous commands are denied by rule table.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "timeout_secs": {"type": "integer", "minimum": 1, "maximum": 600, "default": 120}
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
    ]
}

/// 目录的稳定序列化字节（固定字段序由 struct 声明序保证；
/// JSON object 内部键序由 serde_json 默认 BTreeMap 排序保证）。
pub fn catalog_json_bytes() -> Vec<u8> {
    serde_json::to_vec(&catalog()).expect("catalog serializable")
}

/// 字节稳定断言：两次序列化必须逐字节相同（test/doctor 调用）。
pub fn assert_bytes_stable() -> bool {
    catalog_json_bytes() == catalog_json_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_byte_stable() {
        assert!(assert_bytes_stable());
    }

    #[test]
    fn catalog_has_four_tools() {
        let c = catalog();
        assert_eq!(c.len(), 4);
        let names: Vec<&str> = c.iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["read", "write", "edit", "bash"]);
    }
}
