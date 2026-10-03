//! 验证器（硬约束 7：口头完工无效，完成判定只认可复查证据）。
//!
//! 证据双件（审查 #18）：
//!   1. 事件 kind=verify_result（进轨迹，可回放）；
//!   2. 落盘 .sd-agent/evidence/verify-<ts>.txt（含每个探针输出与退出码）。
//! 确定性执行是 Harness 侧独立窄通道（git 等），显式声明、不经模型路径
//!（project-structure.md 决策 3）。
//!
//! P0 探针三件：git diff --stat / git status --porcelain / 轨迹完整性自检。
//!
//! trace_integrity 口径修正（分层处置，防"半截轨迹→永久 FAIL"自伤）：
//!   - 硬问题（判 FAIL）：损坏行（非末尾坏行）、seq 断档、无任何轨迹；
//!   - 降级警告（不影响验收）：半截轨迹（缺 run_finished/run_failed 终态锚点，
//!     疑似退出/关窗把在飞 run 杀掉）、空轨迹文件、末尾半行（崩溃安全口径）；
//!   - 完整轨迹（有终态收尾）照旧严格校验 seq 连续 + run_started 锚点。

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::event::{EventPayload, LifecycleHook, SinkError, TraceRecorder, emit_hook};

/// 一个探针的执行产物（输出 + 退出码，禁口头完工）。
#[derive(Debug, Clone)]
pub struct VerifyProbe {
    pub name: String,
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub output: String,
}

#[derive(Debug)]
pub struct EvidenceReport {
    pub probes: Vec<VerifyProbe>,
    pub evidence_file: PathBuf,
}

#[derive(Debug)]
pub enum VerifyError {
    Sink(SinkError),
    Io(String),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::Sink(e) => write!(f, "sink error: {e}"),
            VerifyError::Io(m) => write!(f, "io error: {m}"),
        }
    }
}

impl std::error::Error for VerifyError {}

impl From<SinkError> for VerifyError {
    fn from(e: SinkError) -> Self {
        VerifyError::Sink(e)
    }
}

/// 收集证据：跑探针 → 逐条 emit verify_result 事件 → 落盘证据文件。
pub fn collect_evidence(
    root: &Path,
    recorder: &TraceRecorder,
) -> Result<EvidenceReport, VerifyError> {
    emit_hook(recorder, LifecycleHook::VerifyBefore, "evidence collection")?;

    let mut probes = Vec::new();
    probes.push(run_git(root, "git_diff_stat", &["diff", "--stat"]));
    probes.push(run_git(root, "git_status", &["status", "--porcelain"]));
    probes.push(trace_integrity(root));

    // 落盘证据（含退出码）。
    let ts = crate::event::now_unix_ms();
    let evidence_dir = root.join(".sd-agent").join("evidence");
    std::fs::create_dir_all(&evidence_dir).map_err(|e| VerifyError::Io(e.to_string()))?;
    let evidence_file = evidence_dir.join(format!("verify-{ts}.txt"));

    let mut report_text = String::new();
    report_text.push_str(&format!(
        "sd-agent evidence report\nts_unix_ms={ts}\ntrace_id={}\n\n",
        recorder.trace_id
    ));
    for p in &probes {
        report_text.push_str(&format!(
            "== probe: {} ==\nok={} exit_code={:?}\n--- output ---\n{}\n\n",
            p.name, p.ok, p.exit_code, p.output
        ));
    }
    std::fs::write(&evidence_file, report_text.as_bytes())
        .map_err(|e| VerifyError::Io(e.to_string()))?;

    // 事件侧证据（逐探针 + 汇总）。文件名取不到时退回构造名，生产路径禁 unwrap。
    let evidence_name = evidence_file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("verify-{ts}.txt"));
    let evidence_rel = format!(".sd-agent/evidence/{evidence_name}");
    for p in &probes {
        recorder.record(EventPayload::VerifyResult(crate::event::VerifyResult {
            name: p.name.clone(),
            ok: p.ok,
            exit_code: p.exit_code,
            evidence_path: Some(evidence_rel.clone()),
            detail: summarize(&p.output),
        }))?;
    }
    let all_ok = probes.iter().all(|p| p.ok);
    recorder.record(EventPayload::VerifyResult(crate::event::VerifyResult {
        name: "evidence_summary".into(),
        ok: all_ok,
        exit_code: None,
        evidence_path: Some(evidence_rel),
        detail: format!("{} probes, all_ok={}", probes.len(), all_ok),
    }))?;

    emit_hook(recorder, LifecycleHook::VerifyAfter, "evidence collected")?;
    Ok(EvidenceReport {
        probes,
        evidence_file,
    })
}

/// Harness 侧独立窄通道：git 确定性执行（不经模型路径）。
fn run_git(root: &Path, name: &str, args: &[&str]) -> VerifyProbe {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(root);
    crate::sys::no_console_window(&mut cmd); // GUI 进程防黑窗
    match cmd.output() {
        Ok(out) => {
            let mut text = String::from_utf8_lossy(&out.stdout).to_string();
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.trim().is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str("--- stderr ---\n");
                text.push_str(&err);
            }
            if text.trim().is_empty() {
                text.push_str("(empty output)");
            }
            VerifyProbe {
                name: name.to_string(),
                ok: out.status.success(),
                exit_code: out.status.code(),
                output: text,
            }
        }
        Err(e) => VerifyProbe {
            name: name.to_string(),
            ok: false,
            exit_code: None,
            output: format!("git spawn failed: {e}"),
        },
    }
}

/// 轨迹完整性自检（分层处置口径）：
///
/// - **完整轨迹**（有 run_finished/run_failed 终态收尾）：严格校验——seq 连续、
///   run_started 锚点在、末尾半行按崩溃口径容忍（source.rs / project-structure.md
///   第六节 5）；
/// - **半截轨迹**（缺终态锚点，退出/关窗把在飞 run 杀成半截）：降级为警告
///   （"N 条未完成轨迹（疑似中途退出），不影响验收"），**不再判 FAIL**——否则一个
///   被杀的半截轨迹会让此后每次验证永久 FAIL，与在飞截断缺陷叠加成不可恢复态；
///   但其 seq 断档/坏行仍算问题（真损坏）；
/// - **空文件**：警告不 FAIL（崩溃残留的空壳）；
/// - **ok 判定只看硬问题**：损坏行（非末尾坏行）、seq 断档、无任何轨迹
///   （含 traces 目录不可读——无法核验即无证据）。
///
/// 口径边界：本自检覆盖至 verify_result 发射前的事件；防篡改（签名/外部基线）
/// 不在 P0 口径。
fn trace_integrity(root: &Path) -> VerifyProbe {
    let traces_dir = root.join(".sd-agent").join("traces");
    let mut checked = 0usize;
    let mut problems = Vec::new(); // 真损坏 → 判 FAIL
    let mut warnings = Vec::new(); // 降级项 → 不影响验收
    let mut incomplete: Vec<String> = Vec::new(); // 半截轨迹（缺终态锚点）
    match std::fs::read_dir(&traces_dir) {
        Ok(entries) => {
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map(|x| x == "jsonl").unwrap_or(false))
                .collect();
            files.sort();
            if files.is_empty() {
                problems.push("no trace files found (无任何轨迹)".to_string());
            }
            for f in files {
                let content = match std::fs::read_to_string(&f) {
                    Ok(c) => c,
                    Err(e) => {
                        problems.push(format!("{}: read failed: {e}", f.display()));
                        continue;
                    }
                };
                let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
                if lines.is_empty() {
                    // 空文件：崩溃残留的空壳，警告不 FAIL。
                    warnings.push(format!(
                        "{}: empty trace file (空轨迹文件，不影响验收)",
                        f.display()
                    ));
                    continue;
                }
                let mut expect_seq = 0u64;
                let mut has_start = false;
                let mut has_end = false;
                for (idx, line) in lines.iter().enumerate() {
                    match serde_json::from_str::<crate::event::Event>(line) {
                        Ok(ev) => {
                            if ev.seq != expect_seq {
                                // seq 断档 = 真损坏（无论轨迹完整与否）。
                                problems.push(format!(
                                    "{}: line {} seq {} != expected {}",
                                    f.display(),
                                    idx + 1,
                                    ev.seq,
                                    expect_seq
                                ));
                            }
                            expect_seq = ev.seq + 1;
                            match ev.kind {
                                crate::event::EventKind::RunStarted => has_start = true,
                                crate::event::EventKind::RunFinished
                                | crate::event::EventKind::RunFailed => has_end = true,
                                _ => {}
                            }
                            checked += 1;
                        }
                        Err(e) => {
                            // 末尾半行 = 崩溃安全口径，容忍；中间坏行是真损坏。
                            let is_last = idx + 1 == lines.len();
                            if is_last {
                                warnings.push(format!(
                                    "{}: truncated last line tolerated: {e}",
                                    f.display()
                                ));
                            } else {
                                problems.push(format!("{}: line {}: {}", f.display(), idx + 1, e));
                            }
                        }
                    }
                }
                if has_end {
                    // 完整轨迹：锚点照旧严格校验（防"截尾后 seq 仍连续"假绿）。
                    if !has_start {
                        problems.push(format!(
                            "{}: complete trace missing run_started anchor",
                            f.display()
                        ));
                    }
                } else {
                    // 半截轨迹：缺终态锚点降级为警告，不判 FAIL。
                    incomplete.push(f.display().to_string());
                }
            }
        }
        Err(e) => problems.push(format!("traces dir unreadable (无核验依据): {e}")),
    }
    let ok = problems.is_empty();
    let mut output = if ok {
        format!(
            "trace integrity ok: {checked} events, seq contiguous, anchors present (self-check covers events up to verify_result emission)"
        )
    } else {
        format!("trace integrity FAILED (真损坏): {}", problems.join("; "))
    };
    if !incomplete.is_empty() {
        output.push_str(&format!(
            "；警告：{} 条未完成轨迹（疑似中途退出），不影响验收：{}",
            incomplete.len(),
            incomplete.join(", ")
        ));
    }
    for w in &warnings {
        output.push_str(&format!("；警告：{w}"));
    }
    VerifyProbe {
        name: "trace_integrity".to_string(),
        ok,
        exit_code: Some(if ok { 0 } else { 1 }),
        output,
    }
}

fn summarize(output: &str) -> String {
    const MAX: usize = 200;
    let trimmed = output.trim();
    // char-boundary 安全截断（多字节字符不 panic）。
    let cut = crate::sys::truncate_char_boundary(trimmed, MAX);
    if cut.len() < trimmed.len() {
        format!("{cut}…")
    } else {
        cut.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Hook, JsonlSink, TraceRecorder};
    use std::sync::Arc;

    /// 原始 JSONL 行夹具（绕过 TraceRecorder 的 seq 分配，构造断档/坏行场景）。
    fn raw_line(seq: u64, kind: &str, payload: &str) -> String {
        format!(
            r#"{{"v":1,"trace_id":"t","seq":{seq},"ts_unix_ms":1,"kind":"{kind}","payload":{payload}}}"#
        )
    }

    fn seed_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sd-verify-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // 防上次崩溃残留干扰（撞名拒绝等）
        std::fs::create_dir_all(dir.join(".sd-agent/traces")).unwrap();
        dir
    }

    #[test]
    fn collect_evidence_writes_file_and_events() {
        let dir = seed_dir("evidence");
        let sink = Arc::new(JsonlSink::open(dir.join(".sd-agent/traces/t.jsonl")).unwrap());
        let rec = TraceRecorder::new("t", sink);
        // 完整轨迹（首尾锚点齐）：按完整口径严格校验，应无警告无 FAIL。
        rec.record(EventPayload::Hook(Hook {
            hook: "seed".into(),
            note: "seed event".into(),
        }))
        .unwrap();
        rec.record(EventPayload::RunStarted(crate::event::RunStarted {
            task: "seed".into(),
            max_rounds: 1,
        }))
        .unwrap();
        rec.record(EventPayload::RunFinished(crate::event::RunFinished {
            rounds: 1,
            status: "completed".into(),
        }))
        .unwrap();
        let report = collect_evidence(&dir, &rec).unwrap();
        assert!(report.evidence_file.exists());
        assert!(report.probes.len() >= 3);
        let text = std::fs::read_to_string(&report.evidence_file).unwrap();
        assert!(text.contains("exit_code="));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 半截轨迹（缺终态锚点，退出/关窗杀掉在飞 run）：降级警告，不判 FAIL。
    #[test]
    fn incomplete_trace_warns_but_passes() {
        let dir = seed_dir("incomplete");
        let sink = Arc::new(JsonlSink::open(dir.join(".sd-agent/traces/killed.jsonl")).unwrap());
        let rec = TraceRecorder::new("killed", sink);
        rec.record(EventPayload::RunStarted(crate::event::RunStarted {
            task: "will-be-killed".into(),
            max_rounds: 5,
        }))
        .unwrap();
        // 没有 RunFinished/RunFailed 收尾 → 半截。
        let probe = trace_integrity(&dir);
        assert!(probe.ok, "半截轨迹不应判 FAIL: {}", probe.output);
        assert_eq!(probe.exit_code, Some(0));
        assert!(
            probe.output.contains("未完成") && probe.output.contains("不影响验收"),
            "应给出降级警告文案: {}",
            probe.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 空轨迹文件：警告不 FAIL。
    #[test]
    fn empty_trace_file_warns_but_passes() {
        let dir = seed_dir("empty");
        std::fs::write(dir.join(".sd-agent/traces/empty.jsonl"), b"").unwrap();
        let probe = trace_integrity(&dir);
        assert!(probe.ok, "空文件不应判 FAIL: {}", probe.output);
        assert!(
            probe.output.contains("警告"),
            "应有警告行: {}",
            probe.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// seq 断档 = 真损坏，即便轨迹是半截的也判 FAIL。
    #[test]
    fn seq_gap_fails_even_on_incomplete_trace() {
        let dir = seed_dir("seqgap");
        let content = format!(
            "{}\n{}\n",
            raw_line(0, "run_started", r#"{"task":"t","max_rounds":1}"#),
            raw_line(2, "hook", r#"{"hook":"h","note":"n"}"#) // seq 1 缺失
        );
        std::fs::write(dir.join(".sd-agent/traces/gap.jsonl"), content).unwrap();
        let probe = trace_integrity(&dir);
        assert!(!probe.ok, "seq 断档必须判 FAIL: {}", probe.output);
        assert!(
            probe.output.contains("seq"),
            "输出应指明 seq 断档: {}",
            probe.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 中间坏行 = 真损坏（末尾半行才容忍）。
    #[test]
    fn corrupted_middle_line_fails() {
        let dir = seed_dir("badline");
        let content = format!(
            "{}\n{}\n{}\n",
            raw_line(0, "run_started", r#"{"task":"t","max_rounds":1}"#),
            "{not-json-at-all",
            raw_line(1, "run_finished", r#"{"rounds":1,"status":"completed"}"#)
        );
        std::fs::write(dir.join(".sd-agent/traces/bad.jsonl"), content).unwrap();
        let probe = trace_integrity(&dir);
        assert!(!probe.ok, "中间坏行必须判 FAIL: {}", probe.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 完整轨迹（有终态收尾）照旧严格校验锚点：缺 run_started 判 FAIL。
    #[test]
    fn complete_trace_missing_start_anchor_fails() {
        let dir = seed_dir("nostart");
        let content = format!(
            "{}\n",
            raw_line(0, "run_finished", r#"{"rounds":1,"status":"completed"}"#)
        );
        std::fs::write(dir.join(".sd-agent/traces/nostart.jsonl"), content).unwrap();
        let probe = trace_integrity(&dir);
        assert!(
            !probe.ok,
            "完整轨迹缺 run_started 锚点应判 FAIL: {}",
            probe.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 半截 + 完整并存：完整轨迹的硬问题仍然 FAIL，半截只出警告。
    #[test]
    fn mixed_complete_and_incomplete_traces() {
        let dir = seed_dir("mixed");
        // 完整轨迹（锚点齐）。
        let good = format!(
            "{}\n{}\n{}\n",
            raw_line(0, "run_started", r#"{"task":"a","max_rounds":1}"#),
            raw_line(1, "hook", r#"{"hook":"h","note":"n"}"#),
            raw_line(2, "run_finished", r#"{"rounds":1,"status":"completed"}"#)
        );
        std::fs::write(dir.join(".sd-agent/traces/a-complete.jsonl"), good).unwrap();
        // 半截轨迹（无终态锚点）。
        let partial = format!(
            "{}\n",
            raw_line(0, "run_started", r#"{"task":"b","max_rounds":1}"#)
        );
        std::fs::write(dir.join(".sd-agent/traces/b-killed.jsonl"), partial).unwrap();
        let probe = trace_integrity(&dir);
        assert!(probe.ok, "半截轨迹不应拖垮整体: {}", probe.output);
        assert!(
            probe.output.contains("1 条未完成轨迹"),
            "应报告未完成轨迹数量: {}",
            probe.output
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
