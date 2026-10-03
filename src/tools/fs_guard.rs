//! 路径基线（project-structure.md 决策 6 / 审查 #8）：
//! 双字段记录（原始输入 + 规范化形态，绝不只存 canonicalize 形态——
//! Windows verbatim 路径问题 [S3][S8]）/ 组件级比较 / 平台大小写策略。
//!
//! 边界声明：词法校验尽力而为，校验与打开之间存在 TOCTOU 竞态
//!（project-structure.md 第六节 2），P0 不宣称安全封闭。

use std::path::{Path, PathBuf};

use crate::sys::{OsFamily, lexical_normalize};

/// 双字段路径记录：raw = 模型原始输入；normalized = 词法规范化形态（打开用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedPath {
    pub raw: String,
    pub normalized: PathBuf,
}

impl GuardedPath {
    /// 事件/日志展示形态（不用于打开文件）。
    pub fn display_both(&self) -> String {
        format!(
            "raw={} normalized={}",
            self.raw,
            crate::sys::normalize_display(&self.normalized)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsGuardError {
    /// 逃逸出工作区（`..` 越界、绝对路径、盘符前缀）。
    Escape { raw: String },
    /// 输入不合法（空串、NUL、非 UTF-8 语义等）。
    Invalid { raw: String, reason: String },
}

impl std::fmt::Display for FsGuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsGuardError::Escape { raw } => write!(f, "path escapes workspace: {raw}"),
            FsGuardError::Invalid { raw, reason } => write!(f, "invalid path {raw}: {reason}"),
        }
    }
}

impl std::error::Error for FsGuardError {}

/// 路径守卫：所有文件类工具的唯一路径入口。
/// 校验：非空 → 无 NUL → 相对路径 → 词法规范化不逃逸 → **canonical 真实位置核对**
///（符号链接/junction 穿透防线）→ 双字段记录。
///
/// 残余风险声明（project-structure.md 第六节 2 的延伸）：canonicalize 校验与
/// 打开之间仍存在 TOCTOU 竞态；P0 做尽力校验，不宣称安全封闭。
pub fn guard_path(workspace_root: &Path, raw: &str) -> Result<GuardedPath, FsGuardError> {
    if raw.is_empty() {
        return Err(FsGuardError::Invalid {
            raw: raw.to_string(),
            reason: "empty".into(),
        });
    }
    if raw.contains('\0') {
        return Err(FsGuardError::Invalid {
            raw: raw.to_string(),
            reason: "contains NUL".into(),
        });
    }
    let normalized = lexical_normalize(workspace_root, raw).ok_or(FsGuardError::Escape {
        raw: raw.to_string(),
    })?;
    ensure_real_location_under_root(workspace_root, &normalized, raw)?;
    Ok(GuardedPath {
        raw: raw.to_string(),
        normalized,
    })
}

/// canonical 真实位置核对（符号链接/junction 穿透防线）：
/// 对"路径上最近存在的祖先"做 canonicalize（解析一切 reparse point），
/// 其 canonical 形态必须位于 canonical(root) 之下。词法校验挡 `..`/绝对路径，
/// 本函数挡"词法在内、真实位置在外"的链接穿透。
/// 不存在的尾部组件（write 新建文件）按其最近存在祖先判定——新建的是普通文件，
/// 不存在链接语义。
fn ensure_real_location_under_root(
    root: &Path,
    normalized: &Path,
    raw: &str,
) -> Result<(), FsGuardError> {
    let escape = FsGuardError::Escape {
        raw: raw.to_string(),
    };
    // 找最近存在的祖先（normalized 本身或向上逐级）。
    let mut anchor = normalized.to_path_buf();
    loop {
        if anchor.as_os_str().is_empty() {
            return Ok(()); // 空路径退化（词法校验已通过）
        }
        if anchor.exists() {
            break;
        }
        match anchor.parent() {
            Some(p) if p != anchor => anchor = p.to_path_buf(),
            _ => return Ok(()), // 整条链不存在且已到根：词法校验兜底
        }
    }
    let root_canon = std::fs::canonicalize(root).map_err(|_| escape.clone())?;
    let anchor_canon = std::fs::canonicalize(&anchor).map_err(|_| escape.clone())?;
    if is_under_root(
        OsFamily::current(),
        &crate::sys::strip_verbatim(&root_canon),
        &crate::sys::strip_verbatim(&anchor_canon),
    ) {
        Ok(())
    } else {
        Err(escape)
    }
}

/// 组件级比较（大小写策略按平台）：判定 normalized 是否确实在 root 之下。
/// guard_path 的词法保证之上再做一次物理组件核对（双保险，防根形态差异）。
pub fn is_under_root(os: OsFamily, root: &Path, candidate: &Path) -> bool {
    let root = crate::sys::strip_verbatim(root);
    let candidate = crate::sys::strip_verbatim(candidate);
    let mut rc = root.components().peekable();
    let mut cc = candidate.components().peekable();
    loop {
        match (rc.next(), cc.next()) {
            (None, _) => return true, // root 走完：candidate 在其下
            (Some(_), None) => return false,
            (Some(r), Some(c)) => {
                if !crate::sys::path_component_eq(os, r.as_os_str(), c.as_os_str()) {
                    return false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn guard_keeps_both_fields() {
        let root = Path::new("F:/sd_agent");
        let gp = guard_path(root, "./src/../src/lib.rs").unwrap();
        assert_eq!(gp.raw, "./src/../src/lib.rs");
        assert!(gp.normalized.ends_with("lib.rs"));
    }

    #[test]
    fn guard_rejects_escape_and_absolute() {
        let root = Path::new("F:/sd_agent");
        assert!(guard_path(root, "../x").is_err());
        assert!(guard_path(root, "C:/x").is_err());
        assert!(guard_path(root, "").is_err());
    }

    #[test]
    fn under_root_case_policy() {
        let root = Path::new("F:/sd_agent");
        assert!(is_under_root(
            OsFamily::Windows,
            root,
            Path::new("f:/SD_AGENT/src")
        ));
        assert!(!is_under_root(
            OsFamily::Linux,
            root,
            Path::new("f:/SD_AGENT/src")
        ));
    }

    #[test]
    fn canonical_check_rejects_real_location_outside() {
        // 词法合法但真实位置在 root 外（链接穿透后的等价形态）必须被拒。
        let root = std::env::temp_dir().join(format!("sd-guard-root-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let outside = std::env::temp_dir();
        let err = ensure_real_location_under_root(&root, &outside, "via-link");
        assert!(err.is_err(), "real location outside root must be rejected");
        let inside = root.join("sub");
        assert!(ensure_real_location_under_root(&root, &inside, "ok").is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }
}
