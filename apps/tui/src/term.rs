//! 双终端：hub 提交任务后以系统默认终端弹新终端跑 worker。
//!
//! Windows 假设 Windows Terminal（wt）可用，fallback conhost/cmd start；
//! Linux 按桌面环境查表。全部失败时由调用方降级内嵌执行（不丢任务）。

use std::path::Path;
use std::process::{Command, Stdio};

/// 弹新终端跑 worker（`<exe> --worker --inbox <任务文件>`），不等待返回。
pub fn spawn_worker(exe: &Path, inbox: &Path) -> std::io::Result<()> {
    let exe_str = exe.to_string_lossy().to_string();
    let inbox_str = inbox.to_string_lossy().to_string();
    let args: Vec<String> = vec!["--worker".into(), "--inbox".into(), inbox_str.clone()];

    #[cfg(windows)]
    {
        // 1) Windows Terminal。
        if has_command("wt") {
            let mut cmd = Command::new("wt");
            cmd.args(["-w", "new-tab", "--title", "sd-agent · 会话"])
                .arg(&exe_str)
                .args(&args)
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
            .args(&args)
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
                cmd.args(&pre).arg(&exe_str).args(&args);
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
