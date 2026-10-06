//! 对话流渲染：feed 条目 → 可滚动文本流（含流式光标、灰字干预、工具卡）。
//!
//! 渲染只读 worker 状态；折行交给渲染组件的自动换行，滚动按行偏移。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};

use crate::app::worker::{FeedItem, ToolStatus};
use crate::ui::theme;

/// feed → 尾部窗口 Text（scroll_bottom=距底部偏移，0=贴底自动跟随）。
///
/// 滚动语义与 WorkerState::scroll 一致：按逻辑行取尾部窗口，窗口外不渲染；
/// 折行由渲染组件负责（窗口按逻辑行近似，wrap 后显示行可能略少）。
pub fn render_feed_window(
    feed: &[FeedItem],
    scroll_bottom: u16,
    max_lines: usize,
) -> Text<'static> {
    let all = collect_lines(feed);
    let max_lines = max_lines.max(1);
    let total = all.len();
    let start = total.saturating_sub(max_lines + scroll_bottom as usize);
    let end = (start + max_lines).min(total);
    Text::from(all[start..end].to_vec())
}

/// feed → 全量逻辑行。
fn collect_lines(feed: &[FeedItem]) -> Vec<Line<'static>> {
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
}

/// 前缀 + 正文的段落推送（正文整体同一样式；折行交渲染组件）。
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
