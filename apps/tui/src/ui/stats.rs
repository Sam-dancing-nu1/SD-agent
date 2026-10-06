//! 统计渲染：Token 消耗总览 + 周对齐热力图（7 行 × 8 周）。
//! 全量版（hub WORK 态 / /stats 面板）与精简版（hub 初始态底部短栏：
//! 累计 token + 热力图，图1 规格可精简）。

use ratatui::text::{Line, Span};

use crate::stats::Stats;
use crate::ui::theme;

/// 统计面板行（hub 右栏 / /stats 面板共用）：总览 + 热力图（含图例）。
pub fn render_stats(stats: &Stats) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled("Token 消耗总览", theme::title())));
    lines.push(Line::from(""));
    lines.push(total_line(stats));
    lines.push(Line::from(Span::styled(
        format!(
            "  prompt {} · completion {}",
            stats.total_prompt(),
            stats.total_completion()
        ),
        theme::text(),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "  run {} 次 · 轨迹 {} 份",
            stats.total_runs, stats.trace_files
        ),
        theme::text(),
    )));
    lines.push(Line::from(Span::styled(
        if stats.cache_hit_available {
            "  缓存命中：见分项"
        } else {
            "  缓存命中：端点未返回该字段"
        },
        theme::dim(),
    )));
    lines.push(Line::from(""));
    lines.extend(heatmap_lines(stats, true));
    lines
}

/// 精简统计面板行（初始态底部短栏）：累计 + 热力图。
pub fn render_stats_compact(stats: &Stats) -> Vec<Line<'static>> {
    let mut lines = vec![total_line(stats)];
    lines.extend(heatmap_lines(stats, false));
    lines
}

/// 累计消耗行：`累计 10975 tok`。
fn total_line(stats: &Stats) -> Line<'static> {
    Line::from(vec![
        Span::styled("累计 ", theme::muted()),
        Span::styled(
            format!("{}", stats.total_tokens()),
            theme::accent().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(" tok", theme::muted()),
    ])
}

/// 热力图块：标题 + 7 行格子（legend=true 时追加图例行）。
fn heatmap_lines(stats: &Stats, legend: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let hm = stats.heatmap(8, &crate::stats::today_string());
    lines.push(Line::from(Span::styled(
        format!(
            "消耗热力图（近 {} 周 · {}）",
            hm.label_weeks, hm.anchor_note
        ),
        theme::title(),
    )));
    let weekday_labels = ["一", "二", "三", "四", "五", "六", "日"];
    for (row, levels) in hm.levels.iter().enumerate() {
        // 行首星期标签收紧：2 显示列 + 1 空格，格子 2 列/周，对齐口径同 pad_width。
        let mut spans = vec![Span::styled(
            format!("{} ", super::widgets::pad_width(weekday_labels[row], 2)),
            theme::muted(),
        )];
        for &lv in levels {
            spans.push(Span::styled(
                format!("{} ", theme::HEAT_CHARS[lv as usize]),
                theme::heat(lv as usize),
            ));
        }
        lines.push(Line::from(spans));
    }
    if legend {
        lines.push(Line::from(Span::styled(
            format!(
                "  空 ░ 轻 ▒ 中 ▓ 重 █（按日总量归一 · 峰值 {} tok）",
                hm.max
            ),
            theme::dim(),
        )));
    }
    lines
}
