//! 对话流渲染：feed 条目 → 可滚动文本流（含流式光标、灰字干预、工具卡）。
//!
//! 渲染只读 worker 状态；折行在本层按显示宽度预先完成（宽度口径见
//! char_cols），保证任意终端宽度/字体下文本不越界；滚动按行偏移取尾部窗口。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};

use crate::app::worker::{is_hidden_system_line, FeedItem, ToolStatus};
use crate::ui::theme;

/// feed → 尾部窗口 Text（scroll_bottom=距底部偏移，0=贴底自动跟随）。
///
/// 行集已按 width 显示列折行（显示行）；尾部窗口按行取，窗口外不渲染。
pub fn render_feed_window(
    feed: &[FeedItem],
    scroll_bottom: u16,
    max_lines: usize,
    width: usize,
) -> Text<'static> {
    let all = collect_lines(feed, width);
    let max_lines = max_lines.max(1);
    let total = all.len();
    let start = total.saturating_sub(max_lines + scroll_bottom as usize);
    let end = (start + max_lines).min(total);
    Text::from(all[start..end].to_vec())
}

/// feed → 全量折行后显示行。
///
/// 生命周期系统行（`[系统] run_start` 等）在此过滤：内部事件只留痕轨迹
/// 文件，不给用户看（进料侧 worker::on_event 已拦一层，此处兜底）。
fn collect_lines(feed: &[FeedItem], width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for item in feed {
        match item {
            FeedItem::User(t) => {
                push_wrapped(&mut lines, "你 › ", theme::accent(), t, theme::text());
                lines.push(Line::from(""));
            }
            FeedItem::Assistant {
                text, streaming, ..
            } => {
                let mut t = text.clone();
                if *streaming {
                    t.push('▍');
                }
                let style = if *streaming {
                    theme::text().add_modifier(Modifier::SLOW_BLINK)
                } else {
                    theme::text()
                };
                push_wrapped(&mut lines, "", Style::default(), &t, style);
                lines.push(Line::from(""));
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
                    push_wrapped(&mut lines, "思考 › ", theme::dim(), &t, theme::dim());
                } else {
                    let head: String = text.chars().take(60).collect();
                    lines.push(Line::from(Span::styled(
                        format!("思考 › {head}…（已折叠）"),
                        theme::dim().add_modifier(Modifier::ITALIC),
                    )));
                }
            }
            FeedItem::Tool {
                name, args, status, ..
            } => {
                let (badge, badge_style) = match status {
                    ToolStatus::Streaming => ("…参数流入中", theme::info()),
                    ToolStatus::Pending => ("⏸ 等待审批", theme::accent()),
                    ToolStatus::Running => ("▶ 执行中", theme::info()),
                    ToolStatus::Done(_d) => ("✓ 完成", theme::ok()),
                    ToolStatus::Failed(_f) => ("✗ 未执行/失败", theme::err()),
                };
                lines.push(Line::from(vec![
                    Span::styled("  ⚙ ", theme::muted()),
                    Span::styled(name.clone(), theme::info()),
                    Span::styled("  ", Style::default()),
                    Span::styled(badge.to_string(), badge_style),
                ]));
                // 参数摘要（单行截断）+ 结果摘要。
                let arg_line: String = args.chars().take(120).collect();
                lines.push(Line::from(Span::styled(
                    format!("    {arg_line}"),
                    theme::muted(),
                )));
                let done = match status {
                    ToolStatus::Done(d) | ToolStatus::Failed(d) => Some(d.clone()),
                    _ => None,
                };
                if let Some(d) = done {
                    if !d.is_empty() {
                        let d: String = d.chars().take(160).collect();
                        lines.push(Line::from(Span::styled(format!("    ↳ {d}"), theme::dim())));
                    }
                }
            }
            FeedItem::Intervention(t) => {
                if is_hidden_system_line(t) {
                    continue;
                }
                push_wrapped(&mut lines, "", Style::default(), t, theme::intervention());
            }
            FeedItem::Doctor(report) => {
                lines.push(Line::from(Span::styled("── 体检报告 ──", theme::title())));
                for item in &report.items {
                    let mark = if item.ok { "✓" } else { "✗" };
                    let style = if item.ok { theme::ok() } else { theme::err() };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {mark} "), style),
                        Span::styled(item.title.clone(), theme::text()),
                        Span::styled(format!(" — {}", item.detail), theme::muted()),
                    ]));
                    if !item.ok && !item.fix.is_empty() {
                        lines.push(Line::from(Span::styled(
                            format!("    修法：{}", item.fix),
                            theme::accent(),
                        )));
                    }
                }
            }
            FeedItem::Error { message, hint } => {
                push_wrapped(&mut lines, "错误 › ", theme::err(), message, theme::err());
                if !hint.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("  ↳ {hint}"),
                        theme::accent(),
                    )));
                }
                lines.push(Line::from(""));
            }
            FeedItem::Note(t) => {
                push_wrapped(&mut lines, "", Style::default(), t, theme::dim());
            }
        }
    }
    lines
        .into_iter()
        .flat_map(|l| wrap_line(l, width))
        .collect()
}

/// 滚动条参数（与 render_feed_window 同源口径）：(总行数, 窗口首行)。
pub fn feed_window_metrics(
    feed: &[FeedItem],
    scroll_bottom: u16,
    max_lines: usize,
    width: usize,
) -> (usize, usize) {
    let total = collect_lines(feed, width).len();
    let max_lines = max_lines.max(1);
    let start = total.saturating_sub(max_lines + scroll_bottom as usize);
    (total, start)
}

/// 前缀 + 正文的段落推送（正文整体同一样式；折行由 wrap_line 统一处理）。
fn push_wrapped(
    lines: &mut Vec<Line<'static>>,
    prefix: &str,
    prefix_style: Style,
    body: &str,
    body_style: Style,
) {
    if !prefix.is_empty() {
        lines.push(Line::from(Span::styled(prefix.to_string(), prefix_style)));
    }
    for para in body.split('\n') {
        lines.push(Line::from(Span::styled(para.to_string(), body_style)));
    }
}

/// 折行宽度口径：CJK 等宽字符 2 列（与 ui::is_wide / widgets::disp_width
/// 同源）；ASCII 1 列；其余非 ASCII（… · › ' " — 等歧义宽度字符，部分
/// 终端字体按 2 列渲染）保守按 2 列计——任何字体下折行结果都不越界。
fn char_cols(c: char) -> usize {
    if super::is_wide(c) {
        2
    } else if c.is_ascii() {
        1
    } else {
        2
    }
}

/// 行显示宽度（折行同口径；测试断言"不越界"用）。
#[cfg(test)]
pub fn line_cols(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .flat_map(|s| s.content.chars())
        .map(char_cols)
        .sum()
}

/// 一行按 width 显示列贪心折行：空白处优先断行，无空白超长段硬断；
/// 样式随字符保留（相邻同样式合并回 Span）。
fn wrap_line(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut chars: Vec<(char, Style)> = Vec::new();
    for span in line.spans {
        for c in span.content.chars() {
            chars.push((c, span.style));
        }
    }
    if chars.is_empty() {
        return vec![Line::from("")];
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let mut used = 0;
        let mut j = i;
        while j < chars.len() {
            let cw = char_cols(chars[j].0);
            if used + cw > width {
                break;
            }
            used += cw;
            j += 1;
        }
        if j == i {
            j = i + 1; // 单字符宽于行宽：硬放一行，防死循环
        }
        if j >= chars.len() {
            out.push(styled_line(&chars[i..]));
            break;
        }
        // 空白断点优先（行尾空白归前行）；无空白则硬断。
        match (i..j).rev().find(|&k| chars[k].0.is_whitespace()) {
            Some(bp) => {
                out.push(styled_line(&chars[i..=bp]));
                i = bp + 1;
            }
            None => {
                out.push(styled_line(&chars[i..j]));
                i = j;
            }
        }
    }
    out
}

/// (字符, 样式) 序列 → Line（相邻同样式合并成 Span）。
fn styled_line(chars: &[(char, Style)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for &(c, st) in chars {
        match spans.last_mut() {
            Some(s) if s.style == st => s.content.to_mut().push(c),
            _ => spans.push(Span::styled(c.to_string(), st)),
        }
    }
    Line::from(spans)
}
