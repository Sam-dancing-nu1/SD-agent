//! 跨平台单点（project-structure.md 决策 5/6）：
//! 路径归一化 / 进程终止策略 / 执行器选择，cfg 集中在此，其余模块不写 cfg。

use std::path::{Component, Path, PathBuf};

/// 操作系统族。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsFamily {
    Windows,
    Linux,
    Macos,
}

impl OsFamily {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            OsFamily::Windows
        } else if cfg!(target_os = "macos") {
            OsFamily::Macos
        } else {
            OsFamily::Linux
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            OsFamily::Windows => "windows",
            OsFamily::Linux => "linux",
            OsFamily::Macos => "macos",
        }
    }
}

/// 进程终止策略（残余风险：Windows 强杀默认不清理子进程树，见
/// project-structure.md 第六节 3；进程树清理留升级位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillPolicy {
    /// 直接 kill 单进程；子进程树可能残留（P0 口径，显式声明）。
    KillSingle,
}

impl KillPolicy {
    pub fn current() -> Self {
        KillPolicy::KillSingle
    }

    pub fn as_str(&self) -> &'static str {
        "kill_single"
    }
}

/// 去掉 Windows 扩展长度前缀（\\?\），避免 canonicalize 产物与其他应用
/// 不兼容（[S3] Microsoft Naming a File）。只做前缀剥离，不解析符号链接。
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest.to_string());
    }
    path.to_path_buf()
}

/// 路径形态归一：去 verbatim 前缀 + 统一为 `/` 分隔的显示形态
///（仅用于事件记录与日志展示，不用于打开文件）。
///
/// 渲染规则（问题④修复）：Prefix（盘符 `F:` / UNC `\\server\share`）与
/// RootDir 之间不重复插分隔符——盘符后只出一个 `/`（`F:/sd_agent`，不是
/// 旧实现的 `F://sd_agent` 一类畸形）；UNC 前缀自带根（`//server/share/x`）；
/// 盘符相对形态 `C:foo` 不加 `/`（盘符冒号后紧跟组件）。
pub fn normalize_display(path: &Path) -> String {
    let mut out = String::new();
    for comp in strip_verbatim(path).components() {
        match comp {
            Component::Prefix(p) => {
                // 盘符/UNC 前缀内部反斜杠统一为 `/`；结尾不补分隔符
                //（RootDir 或首个组件统一补，避免 F:// 一类重复斜杠）。
                out.push_str(&p.as_os_str().to_string_lossy().replace('\\', "/"));
            }
            Component::RootDir => {
                if !out.ends_with('/') {
                    out.push('/');
                }
            }
            Component::Normal(part) => {
                if !out.is_empty() && !out.ends_with('/') && !out.ends_with(':') {
                    out.push('/');
                }
                out.push_str(&part.to_string_lossy());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.is_empty() && !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str("..");
            }
        }
    }
    out
}

/// 词法规范化：解析 `.` 与 `..` 组件，拒绝逃逸出根（不做 IO、不解析符号链接，
/// 规避 TOCTOU 的同时保持确定性；TOCTOU 残余风险见 project-structure.md 第六节 2）。
/// 返回规范化后的组件向量；逃逸返回 None。
pub fn lexical_normalize(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut comps: Vec<std::ffi::OsString> = Vec::new();
    let rel = Path::new(relative);
    if rel.is_absolute() {
        return None; // 模型输入只允许工作区相对路径
    }
    for comp in rel.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if comps.pop().is_none() {
                    return None; // 逃逸出根
                }
            }
            Component::Normal(part) => comps.push(part.to_os_string()),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    let mut out = strip_verbatim(root);
    for c in comps {
        out.push(c);
    }
    Some(out)
}

/// 平台路径比较策略：Windows/NTFS 大小写不敏感（[S8]），按 ASCII 忽略大小写
/// 做组件级比较；POSIX 区分大小写。
pub fn path_component_eq(os: OsFamily, a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    match os {
        OsFamily::Windows => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
        _ => a == b,
    }
}

/// Windows GUI 进程内 spawn 子进程防黑窗：CREATE_NO_WINDOW 标志。
/// 桌面壳（windows_subsystem=windows）里任何 Command 都会闪 conhost 窗口，
/// 所有 Harness 侧子进程（doctor 探针/verify 的 git/工具执行）统一从这里设。
#[cfg(windows)]
pub fn no_console_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
pub fn no_console_window(_cmd: &mut std::process::Command) {}

/// char-boundary 安全截断（floor 到边界，绝不 panic；多字节字符安全）。
/// 通用字符串工具收在 sys 单点：tools 与 verify 均可引用，不产生跨层依赖。
pub fn truncate_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn truncate_multibyte_safe() {
        // 中文多字节边界截断不 panic。
        let s = "汉汉汉汉";
        let cut = truncate_char_boundary(s, 5);
        assert!(cut.len() <= 5);
        assert!(s.starts_with(cut));
    }

    #[test]
    fn lexical_blocks_escape() {
        let root = Path::new("F:/sd_agent");
        assert!(lexical_normalize(root, "../outside").is_none());
        assert!(lexical_normalize(root, "src/../src/lib.rs").is_some());
        assert!(lexical_normalize(root, "C:/Windows").is_none());
    }

    #[test]
    fn normalize_display_relative_and_root() {
        assert_eq!(normalize_display(Path::new("a/b")), "a/b");
        assert_eq!(normalize_display(Path::new("/a/b")), "/a/b");
    }

    #[test]
    #[cfg(windows)]
    fn normalize_display_exact_forms() {
        // 精确断言（问题④回归钉死）：盘符 + RootDir 不重复斜杠。
        assert_eq!(normalize_display(Path::new(r"C:\a\b")), "C:/a/b");
        assert_eq!(normalize_display(Path::new(r"F:\sd_agent")), "F:/sd_agent");
        // 斜杠输入与反斜杠输入渲染一致。
        assert_eq!(normalize_display(Path::new("F:/sd_agent")), "F:/sd_agent");
        assert_eq!(normalize_display(Path::new(r"C:\")), "C:/");
        // UNC 前缀自带根。
        assert_eq!(
            normalize_display(Path::new(r"\\server\share\x")),
            "//server/share/x"
        );
    }
}
