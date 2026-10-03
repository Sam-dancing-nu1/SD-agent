//! bash 工具：命令执行（工具名从模型生态惯例，执行器由环境档案 shell_kind
//! 查表裁决——project-structure.md 决策 5）。
//!
//! P0 边界（第六节 1）：命令黑名单前置检查在 policy/rules.rs；本函数
//! cwd 钉死工作区 + 超时 + 全量审计留痕。逃逸面（解释器 -c 类、cd 改目录）
//! 不宣称封闭，真正的封闭靠隔离执行体（硬约束 6）分期到位。
//! 超时强杀按 KillPolicy::KillSingle：子进程树可能残留（第六节 3），留升级位。
//!
//! 输出读取口径（问题①修复）：限量累积但**排空丢弃到 EOF**，不 drop 管道
//! 读端——否则子进程写满管道即 EPIPE（cargo build / git log 一类大输出命令
//! 会非零退出或输出损坏）。残余：排空带 DRAIN_TIMEOUT，孙进程占管不放时
//! 弃剩余输出（输出可能截断，不挂死）。

use std::path::Path;
use std::time::Duration;

use super::{ToolError, ToolOutput};
use crate::config::env::EnvProfile;

/// 参数（deny_unknown_fields：拼错参数名立即回错，与 schema 一致）。
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BashArgs {
    pub command: String,
    pub timeout_secs: Option<u64>,
}

/// 捕获输出上限：截断上限单点常量（问题⑥：与 read 共用
/// `tools::MAX_TOOL_OUTPUT_BYTES` 口径，不再各自散落）。
const MAX_CAPTURE_BYTES: usize = crate::tools::MAX_TOOL_OUTPUT_BYTES;
const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
/// 管道排空超时（防孙进程占管挂死；弃剩余输出不弃正确性）。
const DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub(super) async fn execute(
    workspace_root: &Path,
    env: &EnvProfile,
    args: &BashArgs,
) -> Result<ToolOutput, ToolError> {
    let timeout_secs = args
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);
    let (program, base_args) = env.shell_kind.executor();
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW：GUI 进程防黑窗
    cmd.args(&base_args)
        .arg(&args.command)
        .current_dir(workspace_root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // 编码口径（问题⑤）：不注入任何工具专属环境变量（去工具指向性）——
    // 曾注入 PYTHONIOENCODING 属工具专项补丁，已删。输出编码期望由 env 档案
    // 的 encoding 字段声明（硬约束 9），字节侧统一有损解码容错（decode_lossy）。

    let mut child = cmd
        .spawn()
        .map_err(|e| ToolError::Spawn(format!("{program}: {e}")))?;

    // 管道读取与等待分离：child 不被 move，超时后仍可 kill（KillPolicy::KillSingle）。
    // 读取限量（M2：防大输出炸内存）且**排空丢弃到 EOF**（问题①：不 drop 读端，
    // 防子进程写管道 EPIPE）；排空带超时（M1：防孙进程占管挂死）。
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let out_task = tokio::spawn(async move {
        match stdout_pipe.as_mut() {
            Some(p) => read_capped(p, MAX_CAPTURE_BYTES).await,
            None => (Vec::new(), false),
        }
    });
    let err_task = tokio::spawn(async move {
        match stderr_pipe.as_mut() {
            Some(p) => read_capped(p, MAX_CAPTURE_BYTES).await,
            None => (Vec::new(), false),
        }
    });

    match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await {
        Ok(Ok(status)) => {
            // 排空也带超时：孙进程占管不放时弃剩余输出（残余：输出可能截断，不挂死）。
            let (stdout, out_dropped) = tokio::time::timeout(DRAIN_TIMEOUT, out_task)
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or((Vec::new(), false));
            let (stderr, err_dropped) = tokio::time::timeout(DRAIN_TIMEOUT, err_task)
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or((Vec::new(), false));
            let mut text = decode_lossy(&stdout);
            let stderr = decode_lossy(&stderr);
            if !stderr.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str("--- stderr ---\n");
                text.push_str(&stderr);
            }
            // 截断标记：任一管道丢弃过输出、或合并文本超上限都标注（问题①）。
            if out_dropped || err_dropped || text.len() > MAX_CAPTURE_BYTES {
                text = format!(
                    "{}\n…[truncated]",
                    crate::sys::truncate_char_boundary(&text, MAX_CAPTURE_BYTES)
                );
            }
            if text.is_empty() {
                text.push_str("(no output)");
            }
            Ok(ToolOutput {
                ok: status.success(),
                exit_code: status.code(),
                text,
            })
        }
        Ok(Err(e)) => Err(ToolError::Spawn(format!("wait failed: {e}"))),
        Err(_) => {
            // 超时：KillPolicy::KillSingle——杀直接子进程，进程树残留显式声明。
            let _ = child.kill().await;
            out_task.abort();
            err_task.abort();
            Err(ToolError::TimedOut { secs: timeout_secs })
        }
    }
}

/// 排空式限量读取（问题①核心）：累积到 cap 为止，超出部分**继续读但丢弃**
/// 直到 EOF——绝不提前 drop 读端（否则子进程写满管道即 EPIPE）。
/// 返回（捕获字节 ≤ cap， 是否发生丢弃）。
async fn read_capped<R>(reader: &mut R, cap: usize) -> (Vec<u8>, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8 * 1024];
    let mut dropped = false;
    loop {
        match reader.read(&mut chunk).await {
            // EOF：读端持有到最后，子进程写端全量投递成功。
            Ok(0) => break,
            Ok(n) => {
                let room = cap.saturating_sub(buf.len());
                let keep = room.min(n);
                buf.extend_from_slice(&chunk[..keep]);
                if keep < n {
                    dropped = true;
                }
            }
            // 读错误按 EOF 收尾（不 panic，显式容错）。
            Err(_) => break,
        }
    }
    (buf, dropped)
}

/// 字节 → 字符串：优先 UTF-8，失败按有损解码（不 panic，显式容错）。
fn decode_lossy(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => String::from_utf8_lossy(bytes).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_and_captures() {
        let dir = std::env::temp_dir().join(format!("sd-bash-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let env = EnvProfile::detect();
        let out = execute(
            &dir,
            &env,
            &BashArgs {
                command: "echo hello-from-bash-probe".into(),
                timeout_secs: Some(30),
            },
        )
        .await
        .unwrap();
        assert!(out.ok);
        assert!(out.text.contains("hello-from-bash-probe"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_capped_drains_to_eof_no_epipe() {
        // 问题①回归钉死：读满上限后必须继续排空丢弃到 EOF——写端 64 KiB
        // 全量投递成功（旧实现 drop 读端会让 write_all 拿到 BrokenPipe）。
        let (tx, mut rx) = tokio::io::duplex(1024);
        let writer = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let data = vec![b'x'; 64 * 1024];
            let mut tx = tx;
            let res = tx.write_all(&data).await;
            let _ = tx.shutdown().await;
            res
        });
        let (buf, dropped) = read_capped(&mut rx, 8 * 1024).await;
        assert_eq!(buf.len(), 8 * 1024);
        assert!(dropped);
        let write_result = writer.await.expect("writer task joins");
        assert!(
            write_result.is_ok(),
            "writer must not hit EPIPE: {write_result:?}"
        );
    }

    #[tokio::test]
    async fn read_capped_small_input_undropped() {
        let mut src: &[u8] = b"hello";
        let (buf, dropped) = read_capped(&mut src, 16).await;
        assert_eq!(buf, b"hello");
        assert!(!dropped);
        let mut src2: &[u8] = b"abcdef";
        let (buf2, dropped2) = read_capped(&mut src2, 3).await;
        assert_eq!(buf2, b"abc");
        assert!(dropped2);
    }

    /// 64 KiB 输出命令（按 ShellKind 方言；总量 64 × 1024 字节）。
    fn big_output_command(kind: crate::config::env::ShellKind) -> String {
        use crate::config::env::ShellKind;
        let line = "x".repeat(1024);
        match kind {
            ShellKind::Cmd => format!("for /L %i in (1,1,64) do @echo {line}"),
            ShellKind::PowerShell => format!("1..64 | ForEach-Object {{ '{line}' }}"),
            ShellKind::Bash | ShellKind::Sh => format!("yes {line} | head -n 64"),
        }
    }

    #[tokio::test]
    async fn big_output_exits_zero_with_truncate_marker() {
        // 问题①验收：输出 64 KiB 的命令 exit code 仍为 0 且文本带截断标记。
        let dir = std::env::temp_dir().join(format!("sd-bash-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let env = EnvProfile::detect();
        let command = big_output_command(env.shell_kind);
        let out = execute(
            &dir,
            &env,
            &BashArgs {
                command,
                timeout_secs: Some(60),
            },
        )
        .await
        .unwrap();
        assert!(out.ok, "exit={:?} text={}", out.exit_code, out.text);
        assert_eq!(out.exit_code, Some(0));
        assert!(
            out.text.contains("…[truncated]"),
            "head={}",
            &out.text[..out.text.len().min(200)]
        );
        // 截断后体量受上限约束。
        assert!(out.text.len() <= MAX_CAPTURE_BYTES + 32);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
