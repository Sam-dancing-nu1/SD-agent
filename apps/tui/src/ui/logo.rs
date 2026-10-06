//! Logo 渲染：大 Logo（hub 初始态）/ 小 Logo（顶栏）。
//!
//! 渲染统一走 `render_into(buf, cols, rows)`：SVG 网格渲染（logo_raster
//! 零依赖光栅化，半字符 ▀ 上下双色），光栅化失败时降级 ASCII 占位。

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::theme;

/// SVG 原文（作者交付的 logo，构建期即源码内嵌）。
const LOGO_SVG: &str = include_str!("../../assets/logo.svg");

/// Logo 字形渲染入口：把 Logo 画进 `buf`（按 `cols` 列宽居中）。
///
/// 半字符网格：每格 '▀'，fg=上半像素色，bg=下半像素色；行高按 rows 给定
/// （调用方按可用区域给）。光栅化异常时降级 ASCII 占位（渲染层不崩）。
pub fn render_into(buf: &mut Vec<Line<'static>>, cols: usize, rows: usize) {
    // 保形（用户实测教训：60 列×7 行把 1:1 SVG 横拉 4 倍成金条）：
    // SVG viewBox 1:1，半字符格 1 宽 2 高，正方形要求 cols == rows*2。
    // 取"外部空间放得下的最大正方形"，高度不够宁可缩宽，绝不拉伸。
    let cols_max = cols.clamp(8, 200);
    let rows_max = rows.clamp(3, 60);
    let cols = cols_max.min(rows_max * 2);
    let rows = cols / 2;
    let grid = super::logo_raster::render_logo(LOGO_SVG, cols, rows);
    if grid.cols == 0 || grid.rows == 0 {
        render_ascii_fallback(buf, cols);
        return;
    }
    for r in 0..grid.rows {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(grid.cols);
        for c in 0..grid.cols {
            let cell = &grid.cells[r * grid.cols + c];
            let mut style =
                Style::default().fg(ratatui::style::Color::Rgb(cell.fg.0, cell.fg.1, cell.fg.2));
            if let Some((r, g, b)) = cell.bg {
                style = style.bg(ratatui::style::Color::Rgb(r, g, b));
            }
            spans.push(Span::styled(cell.ch.to_string(), style));
        }
        buf.push(Line::from(spans));
    }
}

/// ASCII 降级（光栅化失败时）。
fn render_ascii_fallback(buf: &mut Vec<Line<'static>>, cols: usize) {
    let art = [
        " ███████╗██████╗",
        " ██╔════╝██╔══██╗",
        " ███████╗██║  ██║",
        " ╚════██║██║  ██║",
        " ███████║██████╔╝",
        " ╚══════╝╚═════╝ ",
    ];
    for l in art {
        buf.push(Line::from(Span::styled(center(l, cols), theme::accent())));
    }
}

/// 小 Logo（hub 运行态顶栏左侧）。
pub fn small(version: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("SD", Style::default().fg(theme::ACCENT)),
        Span::styled(format!(" · {version}"), theme::muted()),
    ])
}

/// 按显示宽度把 s 居中到 cols 列（CJK 按 2 列近似；超宽左对齐截断）。
fn center(s: &str, cols: usize) -> String {
    let width: usize = s
        .chars()
        .map(|c| if super::is_wide(c) { 2 } else { 1 })
        .sum();
    if width >= cols {
        return super::widgets::fit_width(s, cols);
    }
    let pad = (cols - width) / 2;
    format!("{}{}", " ".repeat(pad), s)
}
