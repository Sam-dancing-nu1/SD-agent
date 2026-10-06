//! 双终端：hub 提交任务/打开会话后以系统默认终端弹新终端跑 worker。
//!
//! Windows 假设 Windows Terminal（wt）可用，fallback conhost/cmd start；
//! Linux 按桌面环境查表。全部失败时由调用方状态栏报错提示（任务文件/
//! 会话库保留，可重试）——进程模型铁律：hub 绝不原地变身 worker。

use std::path::Path;
use std::process::{Command, Stdio};

/// 弹新终端跑 worker 新任务（`<exe> --worker --inbox <任务文件>`），不等待返回。
pub fn spawn_worker(exe: &Path, inbox: &Path) -> std::io::Result<()> {
    let args: Vec<String> = vec![
        "--worker".into(),
        "--inbox".into(),
        inbox.to_string_lossy().to_string(),
    ];
    spawn_in_terminal(exe, &args)
}

/// 弹新终端打开历史会话（`<exe> --worker --session <id>`），不等待返回。
/// 一个 worker 窗口单 WORK；hub 原地不动只刷新历史。
pub fn spawn_worker_session(exe: &Path, session_id: &str) -> std::io::Result<()> {
    let args: Vec<String> = vec![
        "--worker".into(),
        "--session".into(),
        session_id.to_string(),
    ];
    spawn_in_terminal(exe, &args)
}

/// 系统默认终端弹窗执行 `<exe> <args>`（wt → conhost；Linux 桌面环境查表）。
fn spawn_in_terminal(exe: &Path, args: &[String]) -> std::io::Result<()> {
    let exe_str = exe.to_string_lossy().to_string();

    #[cfg(windows)]
    {
        // 1) Windows Terminal。
        if has_command("wt") {
            let mut cmd = Command::new("wt");
            cmd.args(["-w", "new-tab", "--title", "sd-agent · 会话"])
                .arg(&exe_str)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if cmd.spawn().is_ok() {
                return Ok(());
            }
        }
        // 2) conhost/cmd start（窗口标题可读）。
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", "sd-agent · 会话", &exe_str])
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        return cmd.spawn().map(|_| ());
    }

    #[cfg(not(windows))]
    {
        for (term, pre) in [
            ("x-terminal-emulator", vec!["-e"]),
            ("gnome-terminal", vec!["--"]),
            ("konsole", vec!["-e"]),
            ("xterm", vec!["-e"]),
        ] {
            if has_command(term) {
                let mut cmd = Command::new(term);
                cmd.args(&pre).arg(&exe_str).args(args);
                if cmd.spawn().is_ok() {
                    return Ok(());
                }
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "未找到可用终端（x-terminal-emulator/gnome-terminal/konsole/xterm）",
        ))
    }
}

/// 命令是否在 PATH 上。
fn has_command(name: &str) -> bool {
    let checker = if cfg!(windows) { "where" } else { "which" };
    Command::new(checker)
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
