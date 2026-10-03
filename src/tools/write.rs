//! write 工具：写工作区内 UTF-8 文本（创建或覆盖；覆盖属裁决项，
//! 是否放行由 policy::dispatch 决定，本函数只执行）。

use std::path::Path;

use super::{ToolError, ToolOutput};

/// 参数（deny_unknown_fields：拼错参数名立即回错，与 schema 一致）。
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteArgs {
    pub path: String,
    pub content: String,
}

pub(super) fn execute(
    workspace_root: &Path,
    os: crate::sys::OsFamily,
    args: &WriteArgs,
) -> Result<ToolOutput, ToolError> {
    let gp = super::fs_guard::guard_path(workspace_root, &args.path)
        .map_err(|e| ToolError::Io(e.to_string()))?;
    if !super::fs_guard::is_under_root(os, workspace_root, &gp.normalized) {
        return Err(ToolError::Io(format!("path outside workspace: {}", gp.raw)));
    }
    if let Some(parent) = gp.normalized.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ToolError::Io(format!("mkdir {}: {e}", parent.display())))?;
    }
    std::fs::write(&gp.normalized, args.content.as_bytes())
        .map_err(|e| ToolError::Io(format!("{}: {e}", gp.raw)))?;
    Ok(ToolOutput {
        ok: true,
        exit_code: Some(0),
        text: format!(
            "wrote {} bytes to {} ({})",
            args.content.len(),
            gp.raw,
            gp.display_both()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_inside_workspace() {
        let dir = std::env::temp_dir().join(format!("sd-write-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = execute(
            &dir,
            crate::sys::OsFamily::Windows,
            &WriteArgs {
                path: "sub/b.txt".into(),
                content: "content".into(),
            },
        )
        .unwrap();
        assert!(out.ok);
        assert_eq!(
            std::fs::read_to_string(dir.join("sub/b.txt")).unwrap(),
            "content"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
