//! 拖选文本提取 + OSC 52 剪贴板写入（零依赖）。
//!
//! 选区是屏幕格坐标（App.selection），本模块负责：
//! 1) 把选区坐标切回文本（行内字符范围，CJK 宽字符按 2 列近似）；
//! 2) 生成 OSC 52 转义序列（\x1b]52;c;<base64>\x07）并写 stdout——
//!    借终端自身能力进系统剪贴板，不引第三方剪贴板库。
//!
//! 纯文本行与 ui::chat / ui::hub 的渲染行同口径（前缀+正文/标题行），
//! 已知限制：折行显示的行按逻辑行近似（与渲染层注释一致）。

use std::io::Write;

use sd_agent::session::SessionMeta;

use super::worker::FeedItem;

/// 屏幕选区（起止含端点，行主序归一后的 (x,y) 对）。
pub type Sel = ((u16, u16), (u16, u16));

/// 归一选区：保证起点在行主序前（行先于列）。
pub fn normalize_sel(a: (u16, u16), b: (u16, u16)) -> Sel {
    if (a.1, a.0) <= (b.1, b.0) {
        (a, b)
    } else {
        (b, a)
    }
}

/// base64（标准字母表 + padding）。自写实现，避免额外依赖。
pub fn base64(data: &[u8]) -> String {
    const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TBL[(n >> 18) as usize & 63] as char);
        out.push(TBL[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TBL[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TBL[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// OSC 52 序列（BEL 收尾；部分终端只认 ST，BEL 为最广兼容口径）。
pub fn osc52_sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// 写剪贴板：序列直接打到 stdout（终端识别后进系统剪贴板）。
pub fn write_clipboard(text: &str) {
    let mut out = std::io::stdout();
    let _ = write!(out, "{}", osc52_sequence(text));
    let _ = out.flush();
}

/// 宽字符近似判定（与 ui/mod.rs 同口径；ui 侧私有，此处复刻免跨界依赖）。
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF | 0xFE30..=0xFE6F | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
}

/// 按显示列切片 [c0, c1]（含端点；宽字符与区间相交即收）。
pub fn slice_display(line: &str, c0: u16, c1: u16) -> String {
    let mut out = String::new();
    let mut col = 0u16;
    for c in line.chars() {
        let w = if is_wide(c) { 2u16 } else { 1 };
        let end = col + w; // 半开区间 [col, end)
        if end > c0 && col <= c1 {
            out.push(c);
        }
        col = end;
        if col > c1 {
            break;
        }
    }
    out
}

/// 尾部窗口（与 ui::chat::render_feed_window 同口径）：
/// scroll=距底部偏移，0=贴底；取逻辑行 [total-h-scroll, total-h-scroll+h)。
pub fn visible_window(all: &[String], scroll: u16, height: usize) -> Vec<String> {
    let height = height.max(1);
    let total = all.len();
    let start = total.saturating_sub(height + scroll as usize);
    let end = (start + height).min(total);
    all[start..end].to_vec()
}

/// 选区 → 文本。lines[k] 显示在屏幕行 inner_y+k，列起点 inner_x。
/// 中间行整行取，首/尾行按列裁剪；行间以 \n 连接。
pub fn extract_selection(lines: &[String], inner_x: u16, inner_y: u16, sel: Sel) -> String {
    let ((x1, y1), (x2, y2)) = sel;
    let mut out = Vec::new();
    for (k, line) in lines.iter().enumerate() {
        let row = inner_y + k as u16;
        if row < y1 || row > y2 {
            continue;
        }
        let (c0, c1) = if y1 == y2 {
            (x1.saturating_sub(inner_x), x2.saturating_sub(inner_x))
        } else if row == y1 {
            (x1.saturating_sub(inner_x), u16::MAX)
        } else if row == y2 {
            (0, x2.saturating_sub(inner_x))
        } else {
            (0, u16::MAX)
        };
        out.push(slice_display(line, c0, c1));
    }
    out.join("\n")
}

/// feed → 全量纯文本逻辑行（与 ui::chat::collect_lines 的行序/前缀同口径，
/// 去样式；流式尾巴 ▍ 一并保留以对齐显示行）。
pub fn feed_plain_lines(feed: &[FeedItem]) -> Vec<String> {
    use super::worker::ToolStatus;
    let mut lines: Vec<String> = Vec::new();
    for item in feed {
        match item {
            FeedItem::User(t) => {
                push_plain(&mut lines, "你 › ", t);
            }
            FeedItem::Assistant {
                text, streaming, ..
            } => {
                let mut t = text.clone();
                if *streaming {
                    t.push('▍');
                }
                push_plain(&mut lines, "", &t);
            }
            FeedItem::Reasoning {
                text,
                streaming,
                open,
            } => {
                if *open {
                    let mut t = text.clone();
                    if *streaming {
                        t.push('▍');
                    }
                    push_plain(&mut lines, "思考 › ", &t);
                } else {
                    let head: String = text.chars().take(60).collect();
                    lines.push(format!("思考 › {head}…（已折叠）"));
                }
            }
            FeedItem::Tool {
                name, args, status, ..
            } => {
                let badge = match status {
                    ToolStatus::Streaming => "…参数流入中",
                    ToolStatus::Pending => "⏸ 等待审批",
                    ToolStatus::Running => "▶ 执行中",
                    ToolStatus::Done(_) => "✓ 完成",
                    ToolStatus::Failed(_) => "✗ 未执行/失败",
                };
                lines.push(format!("  ⚙ {name}  {badge}"));
                let arg_line: String = args.chars().take(120).collect();
                lines.push(format!("    {arg_line}"));
                let done = match status {
                    ToolStatus::Done(d) | ToolStatus::Failed(d) => Some(d),
                    _ => None,
                };
                if let Some(d) = done {
                    if !d.is_empty() {
                        let d: String = d.chars().take(160).collect();
                        lines.push(format!("    ↳ {d}"));
                    }
                }
            }
            FeedItem::Intervention(t) | FeedItem::Note(t) => {
                push_plain(&mut lines, "", t);
            }
            FeedItem::Doctor(report) => {
                lines.push("── 体检报告 ──".into());
                for it in &report.items {
                    let mark = if it.ok { "✓" } else { "✗" };
                    lines.push(format!("  {mark} {} — {}", it.title, it.detail));
                    if !it.ok && !it.fix.is_empty() {
                        lines.push(format!("    修法：{}", it.fix));
                    }
                }
            }
            FeedItem::Error { message, hint } => {
                push_plain(&mut lines, "错误 › ", message);
                if !hint.is_empty() {
                    lines.push(format!("  ↳ {hint}"));
                }
                lines.push(String::new());
            }
        }
    }
    lines
}

/// 历史列表 → 纯文本行（与 ui/hub.rs 列表行同口径）。
pub fn history_plain_lines(sessions: &[SessionMeta]) -> Vec<String> {
    sessions
        .iter()
        .map(|s| format!("● {}  ({} 条)", s.title, s.message_count))
        .collect()
}

/// 前缀 + 正文的段落推送（按 \n 拆行，与渲染层 push_wrapped 同口径）。
fn push_plain(lines: &mut Vec<String>, prefix: &str, body: &str) {
    if !prefix.is_empty() {
        lines.push(prefix.to_string());
    }
    for para in body.split('\n') {
        lines.push(para.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64("你".as_bytes()), "5L2g");
    }

    #[test]
    fn osc52_wraps_base64() {
        assert_eq!(osc52_sequence("ab"), "\x1b]52;c;YWI=\x07");
    }

    #[test]
    fn slice_display_cjk_and_range() {
        let s = "a你b";
        // a=列0，你=列1..3，b=列3。
        assert_eq!(slice_display(s, 0, 0), "a");
        assert_eq!(slice_display(s, 1, 2), "你");
        assert_eq!(slice_display(s, 3, 3), "b");
        assert_eq!(slice_display(s, 0, 3), "a你b");
        assert_eq!(slice_display(s, 4, 9), "");
    }

    #[test]
    fn visible_window_tail_semantics() {
        let all: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        assert_eq!(visible_window(&all, 0, 3), vec!["7", "8", "9"]);
        assert_eq!(visible_window(&all, 2, 3), vec!["5", "6", "7"]);
        assert_eq!(visible_window(&all, 20, 3), vec!["0", "1", "2"]);
    }

    #[test]
    fn extract_selection_multiline() {
        let lines = vec!["hello".to_string(), "world".to_string(), "end".to_string()];
        // 起点 (1,6)（行0列1）→ 终点 (2,7)（行1列2，含端点）；inner_y=6。
        let sel = normalize_sel((2, 7), (1, 6));
        assert_eq!(sel, ((1, 6), (2, 7)));
        let text = extract_selection(&lines, 0, 6, sel);
        assert_eq!(text, "ello\nwor");
    }

    #[test]
    fn extract_single_line_range() {
        let lines = vec!["hello".to_string()];
        let sel = normalize_sel((2, 3), (1, 3));
        assert_eq!(extract_selection(&lines, 0, 3, sel), "el");
    }

    #[test]
    fn normalize_reorders_drag_reverse() {
        let s = normalize_sel((5, 9), (1, 2));
        assert_eq!(s, ((1, 2), (5, 9)));
    }
}
