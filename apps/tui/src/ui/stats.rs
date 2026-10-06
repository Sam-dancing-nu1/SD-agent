//! 统计渲染：Token 消耗总览 + 周对齐热力图（7 行 × 8 周）。

use ratatui::text::{Line, Span};

use crate::stats::Stats;
use crate::ui::theme;

/// 统计面板行（hub 右栏 / /stats 面板共用）。
pub fn render_stats(stats: &Stats) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled("Token 消耗总览", theme::title())));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("累计 ", theme::muted()),
        Span::styled(
            format!("{}", stats.total_tokens()),
            theme::accent().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(" tok", theme::muted()),
    ]));
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

    // 热力图：7 行（一..日）× 8 周。
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
        let mut spans = vec![Span::styled(
            format!(" {} ", weekday_labels[row]),
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
    lines.push(Line::from(Span::styled(
        format!("  轻 ▒ 中 ▓ 重 █（按日总量归一 · 峰值 {} tok）", hm.max),
        theme::dim(),
    )));
    lines
}
