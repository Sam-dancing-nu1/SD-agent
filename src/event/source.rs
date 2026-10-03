//! EventSource：读侧 replay（审查 #7/#11——订阅/回放/断点恢复的接口位）。
//!
//! P0 实现 JSONL 文件回放：从指定 seq 起读，容忍末尾半行与坏行（丢弃+计数，
//! project-structure.md 第六节 5 的准确口径），对高版本事件按"跳过+计数"
//! 处置（契约只增不改名，未来字段兼容靠 serde 忽略未知字段）。

use std::path::PathBuf;

use super::contract::{CONTRACT_VERSION, Event};

/// 读侧回放接口（四个接口之一）。
pub trait EventSource: Send + Sync {
    /// 从 from_seq（含）回放事件。
    fn replay_from(&self, from_seq: u64) -> Result<ReplayOutcome, SourceError>;
}

/// 回放产物：事件 + 处置警告（坏行/高版本行的计数与位置）。
#[derive(Debug, Default)]
pub struct ReplayOutcome {
    pub events: Vec<Event>,
    /// 被丢弃的行（摘要，含行号与原因）。
    pub warnings: Vec<String>,
}

/// JSONL 文件实现。
pub struct JsonlSource {
    pub path: PathBuf,
}

impl JsonlSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl EventSource for JsonlSource {
    fn replay_from(&self, from_seq: u64) -> Result<ReplayOutcome, SourceError> {
        let content = std::fs::read_to_string(&self.path).map_err(|e| SourceError::Io {
            path: self.path.display().to_string(),
            source: e,
        })?;
        let mut outcome = ReplayOutcome::default();
        let total_lines = content.lines().count();
        for (idx, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Event>(line) {
                Ok(ev) => {
                    // 高版本行口径（自查确认，行为不变）：v > CONTRACT_VERSION 的行
                    // 跳过 + 计数警告，回放不中断——契约只增不改名，未来版本可能带
                    // 本端不理解的语义，宁可跳过留痕也不误读；同文件其余行照常回放。
                    if ev.v > CONTRACT_VERSION {
                        outcome.warnings.push(format!(
                            "line {}: skipped event v{} (> {CONTRACT_VERSION})",
                            idx + 1,
                            ev.v
                        ));
                        continue;
                    }
                    if ev.seq >= from_seq {
                        outcome.events.push(ev);
                    }
                }
                Err(e) => {
                    // 末尾半行 = 崩溃安全口径，静默容忍；中间坏行记警告。
                    let is_last = idx + 1 == total_lines;
                    let reason = if is_last {
                        "truncated last line tolerated"
                    } else {
                        "bad line dropped"
                    };
                    outcome
                        .warnings
                        .push(format!("line {}: {reason}: {e}", idx + 1));
                }
            }
        }
        Ok(outcome)
    }
}

#[derive(Debug)]
pub enum SourceError {
    Io {
        path: String,
        source: std::io::Error,
    },
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Io { path, source } => write!(f, "source io error on {path}: {source}"),
        }
    }
}

impl std::error::Error for SourceError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerates_truncated_last_line() {
        let dir = std::env::temp_dir().join(format!("sd-src-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let good = r#"{"v":1,"trace_id":"t","seq":0,"ts_unix_ms":1,"kind":"hook","payload":{"hook":"run_start","note":"n"}}"#;
        std::fs::write(&path, format!("{good}\n{{\"v\":1,\"trac")).unwrap();
        let src = JsonlSource::new(&path);
        let out = src.replay_from(0).unwrap();
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].contains("truncated"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_seq_filters() {
        let dir = std::env::temp_dir().join(format!("sd-src-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let mut content = String::new();
        for seq in 0..4 {
            content.push_str(&format!(
                r#"{{"v":1,"trace_id":"t","seq":{seq},"ts_unix_ms":1,"kind":"hook","payload":{{"hook":"h","note":"n"}}}}"#
            ));
            content.push('\n');
        }
        std::fs::write(&path, content).unwrap();
        let out = JsonlSource::new(&path).replay_from(2).unwrap();
        assert_eq!(out.events.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 高版本行（v > CONTRACT_VERSION）：跳过 + 计数警告，其余行照常回放（口径自查）。
    #[test]
    fn high_version_lines_skipped_and_counted() {
        let dir = std::env::temp_dir().join(format!("sd-src-test3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let mut content = String::new();
        for v in [1u32, CONTRACT_VERSION + 1, 1] {
            content.push_str(&format!(
                r#"{{"v":{v},"trace_id":"t","seq":0,"ts_unix_ms":1,"kind":"hook","payload":{{"hook":"h","note":"n"}}}}"#
            ));
            content.push('\n');
        }
        std::fs::write(&path, content).unwrap();
        let out = JsonlSource::new(&path).replay_from(0).unwrap();
        assert_eq!(out.events.len(), 2, "高版本行应被跳过");
        assert!(
            out.events.iter().all(|e| e.v <= CONTRACT_VERSION),
            "回放事件不应含高版本行"
        );
        assert_eq!(out.warnings.len(), 1);
        assert!(
            out.warnings[0].contains("skipped") && out.warnings[0].contains("v2"),
            "应有跳过计数警告: {:?}",
            out.warnings
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
