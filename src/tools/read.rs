//! read 工具：读工作区内 UTF-8 文本（执行函数 pub(super)，经 tools::run_tool 收口）。

use std::path::Path;

use super::{ToolError, ToolOutput};

/// 参数（模型 JSON → 强类型）。deny_unknown_fields：模型拼错参数名立即回错，
/// 不静默忽略（schema 声明 additionalProperties:false，行为必须一致）。
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadArgs {
    pub path: String,
    pub offset_lines: Option<usize>,
    pub limit_lines: Option<usize>,
}

/// 输出上限：超过截断并在文本尾部标注（防炸上下文；阈值口径见
/// config::DisciplineTables::output_digest_threshold_bytes 的登记）。
/// 截断按字符边界（crate::sys::truncate_char_boundary，中文不 panic）。
const MAX_OUTPUT_BYTES: usize = 16 * 1024;

pub(super) fn execute(
    workspace_root: &Path,
    os: crate::sys::OsFamily,
    args: &ReadArgs,
) -> Result<ToolOutput, ToolError> {
    let gp = super::fs_guard::guard_path(workspace_root, &args.path)
        .map_err(|e| ToolError::Io(e.to_string()))?;
    if !super::fs_guard::is_under_root(os, workspace_root, &gp.normalized) {
        return Err(ToolError::Io(format!("path outside workspace: {}", gp.raw)));
    }
    let content = std::fs::read_to_string(&gp.normalized)
        .map_err(|e| ToolError::Io(format!("{}: {e}", gp.raw)))?;
    let lines: Vec<&str> = content.lines().collect();
    let offset = args.offset_lines.unwrap_or(0).min(lines.len());
    let limit = args.limit_lines.unwrap_or(usize::MAX);
    let window: Vec<String> = lines
        .iter()
        .skip(offset)
        .take(limit)
        .enumerate()
        .map(|(i, l)| format!("{}\t{l}", offset + i + 1))
        .collect();
    let mut text = window.join("\n");
    if text.len() > MAX_OUTPUT_BYTES {
        text = format!(
            "{}\n…[truncated]",
            crate::sys::truncate_char_boundary(&text, MAX_OUTPUT_BYTES)
        );
    }
    Ok(ToolOutput {
        ok: true,
        exit_code: Some(0),
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_with_line_numbers() {
        let dir = std::env::temp_dir().join(format!("sd-read-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello\nworld\n").unwrap();
        let out = execute(
            &dir,
            crate::sys::OsFamily::Windows,
            &ReadArgs {
                path: "a.txt".into(),
                offset_lines: None,
                limit_lines: None,
            },
        )
        .unwrap();
        assert!(out.ok);
        assert!(out.text.contains("1\thello"));
        assert!(out.text.contains("2\tworld"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_escape() {
        let dir = std::env::temp_dir().join(format!("sd-read-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = execute(
            &dir,
            crate::sys::OsFamily::Windows,
            &ReadArgs {
                path: "../x.txt".into(),
                offset_lines: None,
                limit_lines: None,
            },
        );
        assert!(err.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
