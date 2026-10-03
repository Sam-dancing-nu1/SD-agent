//! edit 工具：精确字符串替换（旧串必须唯一命中，除非 replace_all）。

use std::path::Path;

use super::{ToolError, ToolOutput};

/// 参数（deny_unknown_fields：拼错参数名立即回错，与 schema 一致）。
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditArgs {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    pub replace_all: Option<bool>,
}

pub(super) fn execute(
    workspace_root: &Path,
    os: crate::sys::OsFamily,
    args: &EditArgs,
) -> Result<ToolOutput, ToolError> {
    let gp = super::fs_guard::guard_path(workspace_root, &args.path)
        .map_err(|e| ToolError::Io(e.to_string()))?;
    if !super::fs_guard::is_under_root(os, workspace_root, &gp.normalized) {
        return Err(ToolError::Io(format!("path outside workspace: {}", gp.raw)));
    }
    let content = std::fs::read_to_string(&gp.normalized)
        .map_err(|e| ToolError::Io(format!("{}: {e}", gp.raw)))?;
    let occurrences = content.matches(&args.old_string).count();
    if args.old_string.is_empty() {
        // 空 old_string + replace_all 会把分隔符插满全文——直接拒绝。
        return Err(ToolError::Io("old_string must not be empty".into()));
    }
    if occurrences == 0 {
        return Err(ToolError::Io(format!("old_string not found in {}", gp.raw)));
    }
    let replace_all = args.replace_all.unwrap_or(false);
    if occurrences > 1 && !replace_all {
        return Err(ToolError::Io(format!(
            "old_string matches {occurrences} times in {}; refine it or set replace_all",
            gp.raw
        )));
    }
    let new_content = if replace_all {
        content.replace(&args.old_string, &args.new_string)
    } else {
        content.replacen(&args.old_string, &args.new_string, 1)
    };
    std::fs::write(&gp.normalized, new_content.as_bytes())
        .map_err(|e| ToolError::Io(format!("{}: {e}", gp.raw)))?;
    Ok(ToolOutput {
        ok: true,
        exit_code: Some(0),
        text: format!(
            "replaced {} occurrence(s) in {} ({})",
            if replace_all { occurrences } else { 1 },
            gp.raw,
            gp.display_both()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_unique_match() {
        let dir = std::env::temp_dir().join(format!("sd-edit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c.txt"), "aaa bbb aaa").unwrap();
        let err = execute(
            &dir,
            crate::sys::OsFamily::Windows,
            &EditArgs {
                path: "c.txt".into(),
                old_string: "aaa".into(),
                new_string: "x".into(),
                replace_all: Some(false),
            },
        );
        assert!(
            err.is_err(),
            "multiple matches must be rejected without replace_all"
        );
        let out = execute(
            &dir,
            crate::sys::OsFamily::Windows,
            &EditArgs {
                path: "c.txt".into(),
                old_string: "aaa".into(),
                new_string: "x".into(),
                replace_all: Some(true),
            },
        )
        .unwrap();
        assert!(out.ok);
        assert_eq!(
            std::fs::read_to_string(dir.join("c.txt")).unwrap(),
            "x bbb x"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
