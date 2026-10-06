//! 统计聚合：轨迹事件（JSONL）→ Token 消耗账本 → 热力图数据。
//!
//! 数据源是唯一事实源（.sd-agent/traces/*.jsonl 的 usage 事件），不维护
//! 平行状态；扫描为容错读（半行/坏行跳过，worker 被杀留半截轨迹不炸）。
//! 热力图维度（主脑定）：按周对齐 7 行 × 最近 N 周（周一起），格值=当日
//! prompt+completion 总 token，档位 0-4 按分位归一。

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// 单日消耗。
#[derive(Debug, Default, Clone)]
pub struct DayStat {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// 当日 run 数（run_started 计数）。
    pub runs: u64,
}

/// 聚合结果。
#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub days: BTreeMap<String, DayStat>,
    /// 总 run 数。
    pub total_runs: u64,
    /// 扫描到的轨迹文件数。
    pub trace_files: usize,
    /// 缓存命中：端点 usage 未返回该字段（如实显示，不编数）。
    pub cache_hit_available: bool,
}

impl Stats {
    /// 扫描轨迹目录聚合（目录不存在 = 空账本）。
    pub fn scan(root: &Path) -> Self {
        let mut st = Stats::default();
        let dir = root.join(".sd-agent").join("traces");
        let Ok(entries) = fs::read_dir(&dir) else {
            return st;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            st.trace_files += 1;
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue; // 半行/坏行容忍（worker 被杀场景）
                };
                let kind = v.get("kind").and_then(|k| k.as_str()).unwrap_or("");
                let ts = v.get("ts_unix_ms").and_then(|t| t.as_u64()).unwrap_or(0);
                let date = ts_to_date(ts);
                let day = st.days.entry(date).or_default();
                match kind {
                    "run_started" => {
                        day.runs += 1;
                        st.total_runs += 1;
                    }
                    "model_turn_finished" => {
                        let p = v
                            .get("payload")
                            .and_then(|p| p.get("usage_prompt_tokens"))
                            .and_then(|x| x.as_u64());
                        let c = v
                            .get("payload")
                            .and_then(|p| p.get("usage_completion_tokens"))
                            .and_then(|x| x.as_u64());
                        if let Some(p) = p {
                            day.prompt_tokens += p;
                        }
                        if let Some(c) = c {
                            day.completion_tokens += c;
                        }
                    }
                    _ => {}
                }
            }
        }
        st
    }

    /// 总消耗（prompt + completion）。
    pub fn total_tokens(&self) -> u64 {
        self.days
            .values()
            .map(|d| d.prompt_tokens + d.completion_tokens)
            .sum()
    }

    pub fn total_prompt(&self) -> u64 {
        self.days.values().map(|d| d.prompt_tokens).sum()
    }

    pub fn total_completion(&self) -> u64 {
        self.days.values().map(|d| d.completion_tokens).sum()
    }

    /// 热力图矩阵：7 行（周一起）× weeks 列（最近 N 周），值为当日总 token。
    /// 今天在最后一列的对应行。
    pub fn heatmap(&self, weeks: usize, today: &str) -> Heatmap {
        let today = parse_date(today);
        let (iso_y, iso_w, weekday) = iso_week(today);
        // 最后一列的周一日期。
        let last_monday = today - (weekday - 1) as i64;
        let first_monday = last_monday - 7 * (weeks as i64 - 1);

        let mut values: Vec<Vec<u64>> = vec![vec![0; weeks]; 7];
        for (date, stat) in &self.days {
            let d = parse_date(date);
            let offset = d - first_monday;
            if offset < 0 {
                continue;
            }
            let col = (offset / 7) as usize;
            let row = (offset % 7) as usize;
            if col < weeks && row < 7 {
                values[row][col] = stat.prompt_tokens + stat.completion_tokens;
            }
        }
        let max = values.iter().flatten().copied().max().unwrap_or(0);
        let levels = values
            .iter()
            .map(|row| row.iter().map(|&v| heat_level(v, max)).collect::<Vec<u8>>())
            .collect();
        Heatmap {
            levels,
            max,
            label_weeks: weeks,
            anchor_note: format!("对齐至 {iso_y}-W{iso_w:02}"),
        }
    }
}

/// 热力图渲染数据。
#[derive(Debug, Clone)]
pub struct Heatmap {
    /// 档位 0..=4（7 行 × N 列）。
    pub levels: Vec<Vec<u8>>,
    /// 当日总量峰值（渲染图例用）。
    pub max: u64,
    pub label_weeks: usize,
    pub anchor_note: String,
}

/// 档位归一：0 空，1-4 按 25/50/75/100% 分位。
fn heat_level(v: u64, max: u64) -> u8 {
    if v == 0 || max == 0 {
        return 0;
    }
    let ratio = v as f64 / max as f64;
    if ratio <= 0.25 {
        1
    } else if ratio <= 0.5 {
        2
    } else if ratio <= 0.75 {
        3
    } else {
        4
    }
}

// ── 日期工具（1970 纪元日序运算，不引第三方时间库） ─────────────

/// unix 毫秒 → YYYY-MM-DD（UTC+8 固定偏移，本机即东八区；含时区迁移代价的
/// 简化，标注：统计按东八区日界）。
fn ts_to_date(ts_ms: u64) -> String {
    let secs = (ts_ms / 1000) as i64 + 8 * 3600;
    let days = secs.div_euclid(86_400);
    date_to_string(days)
}

/// 纪元日序 → YYYY-MM-DD（1970-01-01 = 0）。
fn date_to_string(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// YYYY-MM-DD → 纪元日序。
fn parse_date(s: &str) -> i64 {
    let mut it = s.split('-');
    let y: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(1970);
    let m: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(1);
    let d: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(1);
    days_from_civil(y, m, d)
}

/// Howard Hinnant 的日历算法（days_from_civil / civil_from_days）。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// ISO 周（年, 周序, 周内日 1=周一..7=周日）。
fn iso_week(days: i64) -> (i64, u32, u32) {
    let (y, m, d) = civil_from_days(days);
    let weekday = (((days + 3) % 7) + 1) as u32; // 1970-01-01=周四 → 1=周一..7=周日
    let jan4 = days_from_civil(y, 1, 4);
    let jan4_weekday = (((jan4 + 3) % 7) + 1) as u32;
    let week1_monday = jan4 - (jan4_weekday - 1) as i64;
    let week = ((days - week1_monday) / 7 + 1) as u32;
    let year = if days < week1_monday {
        y - 1
    } else if week > 52 && m == 1 && d <= 7 {
        y + 1
    } else {
        y
    };
    (year, week, weekday)
}

/// 今日（东八区）YYYY-MM-DD。
pub fn today_string() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    ts_to_date(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_roundtrip() {
        assert_eq!(date_to_string(0), "1970-01-01");
        assert_eq!(parse_date("1970-01-01"), 0);
        assert_eq!(date_to_string(parse_date("2026-10-05")), "2026-10-05");
    }

    #[test]
    fn weekday_aligns_monday() {
        // 2026-10-05 是周一。
        let d = parse_date("2026-10-05");
        let (_, _, wd) = iso_week(d);
        assert_eq!(wd, 1);
    }

    #[test]
    fn heat_levels() {
        assert_eq!(heat_level(0, 100), 0);
        assert_eq!(heat_level(25, 100), 1);
        assert_eq!(heat_level(50, 100), 2);
        assert_eq!(heat_level(75, 100), 3);
        assert_eq!(heat_level(100, 100), 4);
    }

    #[test]
    fn heatmap_shape() {
        let st = Stats::default();
        let hm = st.heatmap(8, "2026-10-05");
        assert_eq!(hm.levels.len(), 7);
        assert!(hm.levels.iter().all(|r| r.len() == 8));
    }

    #[test]
    fn scan_tolerates_bad_lines() {
        let dir = std::env::temp_dir().join(format!("sd-stats-test-{}", std::process::id()));
        let traces = dir.join(".sd-agent").join("traces");
        fs::create_dir_all(&traces).unwrap();
        fs::write(
            traces.join("t.jsonl"),
            "{\"kind\":\"run_started\",\"ts_unix_ms\":1791210000000}\n{\"kind\":\"model_turn_finished\",\"ts_unix_ms\":1791210000000,\"payload\":{\"usage_prompt_tokens\":10,\"usage_completion_tokens\":5}}\n{half line",
        )
        .unwrap();
        let st = Stats::scan(&dir);
        assert_eq!(st.total_runs, 1);
        assert_eq!(st.total_prompt(), 10);
        assert_eq!(st.total_completion(), 5);
        let _ = fs::remove_dir_all(&dir);
    }
}
