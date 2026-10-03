//! 环境档案（硬约束 9：环境差异显式写入档案，禁止依赖模型自行适配）。
//!
//! 字段覆盖两路盲审 #5/#9 的平台差异面：os_family / shell_kind / 编码 /
//! 行尾 / 路径约定 / 代理提示 / 进程终止策略。工具名 bash 的实际执行器
//! 由本档案 shell_kind 查表裁决（project-structure.md 决策 5）。

use crate::sys::{KillPolicy, OsFamily};

/// shell/执行器种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    /// Windows cmd.exe（工具名 bash 的默认裁决执行器）。
    Cmd,
    /// Windows PowerShell。
    PowerShell,
    /// POSIX bash。
    Bash,
    /// POSIX sh。
    Sh,
}

impl ShellKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShellKind::Cmd => "cmd",
            ShellKind::PowerShell => "powershell",
            ShellKind::Bash => "bash",
            ShellKind::Sh => "sh",
        }
    }

    /// 查表裁决：工具名 bash → 实际执行器程序与前导参数。
    ///（工具名从模型生态惯例保 bash，见 project-structure.md 决策 5【待磨合】。）
    pub fn executor(&self) -> (&'static str, Vec<&'static str>) {
        match self {
            ShellKind::Cmd => ("cmd", vec!["/C"]),
            ShellKind::PowerShell => ("powershell", vec!["-NoProfile", "-Command"]),
            ShellKind::Bash => ("bash", vec!["-c"]),
            ShellKind::Sh => ("sh", vec!["-c"]),
        }
    }
}

/// 环境档案（三类档案之一，config/mod.rs 门面持有）。
#[derive(Debug, Clone)]
pub struct EnvProfile {
    pub os_family: OsFamily,
    pub shell_kind: ShellKind,
    /// 输出编码口径：期望 UTF-8（不依赖 Windows 代码页）。
    /// 诚实口径：cmd.exe 内建命令实际输出 OEM(GBK) 字节，bash.rs 以有损解码
    /// 容忍（可能乱码）；如需精确中文输出走 PowerShell 或外部工具显式 UTF-8。
    pub encoding: &'static str,
    /// 文本行尾约定。
    pub line_ending: &'static str,
    /// 路径约定说明（分隔符 / 盘符 / 大小写敏感性）。
    pub path_convention: &'static str,
    /// 网络代理环境差异提示（只记录"存在差异"的事实位，不记录代理地址）。
    pub proxy_note: &'static str,
    pub kill_policy: KillPolicy,
}

impl EnvProfile {
    /// 探测当前进程环境（探测失败的字段给保守默认值并显式标注）。
    pub fn detect() -> Self {
        let os_family = OsFamily::current();
        let shell_kind = detect_shell(os_family);
        EnvProfile {
            os_family,
            shell_kind,
            encoding: "utf-8",
            line_ending: if os_family == OsFamily::Windows {
                "crlf"
            } else {
                "lf"
            },
            path_convention: if os_family == OsFamily::Windows {
                "drive-letter, backslash, case-insensitive"
            } else {
                "unix, slash, case-sensitive"
            },
            proxy_note: "network egress may require proxy; see host env (not recorded here)",
            kill_policy: KillPolicy::current(),
        }
    }

    /// 人类可读摘要（doctor 用；不含任何凭据）。
    pub fn summary(&self) -> String {
        format!(
            "os={} shell={} encoding={} line_ending={} path[{}] kill={}",
            self.os_family.as_str(),
            self.shell_kind.as_str(),
            self.encoding,
            self.line_ending,
            self.path_convention,
            self.kill_policy.as_str()
        )
    }
}

fn detect_shell(os: OsFamily) -> ShellKind {
    match os {
        // cmd 是 Windows 全量部署的最低公分母；执行器可被 SD_AGENT_SHELL 覆盖
        //（P0 只认 cmd/powershell 两个值，其他一律回退 cmd 并显式记录）。
        OsFamily::Windows => match std::env::var("SD_AGENT_SHELL") {
            Ok(v) if v.eq_ignore_ascii_case("powershell") => ShellKind::PowerShell,
            _ => ShellKind::Cmd,
        },
        OsFamily::Macos => ShellKind::Bash,
        OsFamily::Linux => {
            if std::path::Path::new("/bin/bash").exists() {
                ShellKind::Bash
            } else {
                ShellKind::Sh
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_has_summary() {
        let env = EnvProfile::detect();
        let s = env.summary();
        assert!(s.contains("os=") && s.contains("shell="));
    }
}
