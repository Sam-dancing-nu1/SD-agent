//! EventSink：提交事件语义 + JSONL 实现（决策 4 / 审查 #20）。
//!
//! 接口只承诺"提交"，不承诺同步落盘——同步/异步由网关层决定（P0 JSONL
//! 实现为整行写入 + flush，单写者约定，见 project-structure.md 第六节 4/5：
//! 崩溃最多损失最后一行，读端容忍并丢弃末尾半行）。
//! P1 换 SQLite 实现时契约不动（扩展位 event/sink_sqlite.rs）。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::contract::{Event, EventPayload};

/// 提交事件语义（四个接口之一）。
pub trait EventSink: Send + Sync {
    /// 提交一个事件。失败是真故障（磁盘/句柄问题），调用方必须处置。
    fn emit(&self, event: Event) -> Result<(), SinkError>;
}

/// JSONL 追加实现（只追加、整行写入、每次 flush）。
pub struct JsonlSink {
    file: Mutex<File>,
    pub path: PathBuf,
}

impl JsonlSink {
    /// 打开（创建）轨迹文件。路径：<workspace>/.sd-agent/traces/<trace_id>.jsonl
    ///
    /// 撞名防护（审查 P2）：trace_id 由调用方拼（如 run-{now_ms}-{pid}），同毫秒
    /// 并发可能撞名。若目标文件**已存在且非空**，拒绝打开并返回 SinkError::Io
    /// （错误文案 "trace file already exists"），调用方换 trace_id 重试——绝不
    /// append 混写：两个 run 的事件混进同一文件会把各自从 0 起的 seq 打乱，
    /// 直接构成 trace_integrity 口径下的"真损坏"。空文件视为崩溃残留的空壳，
    /// 允许接管（不消耗任何事件）。
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, SinkError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SinkError::Io {
                path: parent.display().to_string(),
                source: e,
            })?;
        }
        // 撞名防护：已存在且非空 → 拒绝（报错优于静默混写）。
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.is_file() && meta.len() > 0 {
                return Err(SinkError::Io {
                    path: path.display().to_string(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "trace file already exists (non-empty), refusing to append; \
                         caller should retry with a different trace_id",
                    ),
                });
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| SinkError::Io {
                path: path.display().to_string(),
                source: e,
            })?;
        Ok(Self {
            file: Mutex::new(file),
            path,
        })
    }
}

impl EventSink for JsonlSink {
    fn emit(&self, event: Event) -> Result<(), SinkError> {
        let line = event.to_json_line();
        let mut file = self.file.lock().map_err(|_| SinkError::Poisoned)?;
        // 整行写入：一次 write_all 保证行内不交错；flush 保证崩溃口径（第六节 5）。
        file.write_all(line.as_bytes()).map_err(|e| SinkError::Io {
            path: self.path.display().to_string(),
            source: e,
        })?;
        file.write_all(b"\n").map_err(|e| SinkError::Io {
            path: self.path.display().to_string(),
            source: e,
        })?;
        file.flush().map_err(|e| SinkError::Io {
            path: self.path.display().to_string(),
            source: e,
        })
    }
}

/// 丢弃实现（单测/探针用，无需落盘的场景）。
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: Event) -> Result<(), SinkError> {
        Ok(())
    }
}

/// 轨迹记录器：seq 分配 + 时间戳 + kind 推导的唯一入口。
///（seq 由本结构原子递增，sink 只负责提交——职责分离，单写者约定。）
pub struct TraceRecorder {
    pub trace_id: String,
    seq: AtomicU64,
    sink: Arc<dyn EventSink>,
}

impl TraceRecorder {
    pub fn new(trace_id: impl Into<String>, sink: Arc<dyn EventSink>) -> Self {
        Self {
            trace_id: trace_id.into(),
            seq: AtomicU64::new(0),
            sink,
        }
    }

    /// 便捷构造：轨迹落 <workspace>/.sd-agent/traces/<trace_id>.jsonl。
    pub fn jsonl(workspace: &Path, trace_id: impl Into<String>) -> Result<Self, SinkError> {
        let trace_id = trace_id.into();
        let path = workspace
            .join(".sd-agent")
            .join("traces")
            .join(format!("{trace_id}.jsonl"));
        let sink = JsonlSink::open(path)?;
        Ok(Self::new(trace_id, Arc::new(sink)))
    }

    /// 记录一个 payload（自动分配 seq 与时间戳，kind 由 payload 推导）。
    pub fn record(&self, payload: EventPayload) -> Result<Event, SinkError> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let event = Event::new(self.trace_id.clone(), seq, now_unix_ms(), payload);
        self.sink.emit(event.clone())?;
        Ok(event)
    }

    pub fn next_seq(&self) -> u64 {
        self.seq.load(Ordering::SeqCst)
    }
}

/// 系统时钟毫秒（唯一时间戳来源，集中在此便于审计对齐）。
pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug)]
pub enum SinkError {
    Io {
        path: String,
        source: std::io::Error,
    },
    Poisoned,
}

impl std::fmt::Display for SinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SinkError::Io { path, source } => write!(f, "sink io error on {path}: {source}"),
            SinkError::Poisoned => write!(f, "sink lock poisoned"),
        }
    }
}

impl std::error::Error for SinkError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::contract::{EventPayload, RunStarted};

    #[test]
    fn jsonl_appends_and_seqs_increment() {
        let dir = std::env::temp_dir().join(format!("sd-sink-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // 防上次崩溃残留触发撞名拒绝
        let rec = TraceRecorder::jsonl(&dir, "t1").unwrap();
        for i in 0..3 {
            rec.record(EventPayload::RunStarted(RunStarted {
                task: format!("task-{i}"),
                max_rounds: 20,
            }))
            .unwrap();
        }
        let content = std::fs::read_to_string(&dir.join(".sd-agent/traces/t1.jsonl")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);
        let last: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(last["seq"], 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 撞名防护：同 trace_id 二次打开已存在非空轨迹 → 拒绝并明示报错（不混写）。
    #[test]
    fn open_rejects_existing_non_empty_trace_file() {
        let dir = std::env::temp_dir().join(format!("sd-sink-collide-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = TraceRecorder::jsonl(&dir, "dup").unwrap();
        rec.record(EventPayload::RunStarted(RunStarted {
            task: "first-run".into(),
            max_rounds: 1,
        }))
        .unwrap();
        // 同毫秒并发的另一个 run 撞名 → 必须报错，绝不 append 混写。
        let err = match TraceRecorder::jsonl(&dir, "dup") {
            Err(e) => e,
            Ok(_) => panic!("expected SinkError for colliding trace_id"),
        };
        let msg = err.to_string();
        assert!(msg.contains("already exists"), "unexpected error: {msg}");
        // 原文件未被混写：仍只有 1 行。
        let content = std::fs::read_to_string(&dir.join(".sd-agent/traces/dup.jsonl")).unwrap();
        assert_eq!(content.lines().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 空文件是崩溃残留的空壳，允许接管（撞名防护只拦非空文件）。
    #[test]
    fn open_takes_over_empty_file() {
        let dir = std::env::temp_dir().join(format!("sd-sink-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(".sd-agent/traces/empty.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"").unwrap();
        let sink = JsonlSink::open(&path).expect("empty file should be taken over");
        drop(sink);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
