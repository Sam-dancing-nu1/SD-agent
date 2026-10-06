//! Logo 渲染：大 Logo（hub 初始态）/ 小 Logo（顶栏）。
//!
//! 当前为 ASCII art 占位；作者的 Logo SVG 到货后走构建期转换（SVG → ANSI
//! art）替换 `big()` 的来源文件，渲染接口不变（[待磨合]）。

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::theme;

/// 大 Logo（hub 初始态居中展示）：SD 字形 + 版本 + 定位语。
pub fn big(version: &str) -> Vec<Line<'static>> {
    let art = [
        " ███████╗██████╗",
        " ██╔════╝██╔══██╗",
        " ███████╗██║  ██║",
        " ╚════██║██║  ██║",
        " ███████║██████╔╝",
        " ╚══════╝╚═════╝ ",
    ];
    let mut lines: Vec<Line<'static>> = art
        .iter()
        .map(|l| Line::from(Span::styled(l.to_string(), theme::accent())))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("sd-agent {version} · 私人助理运行时底座"),
        theme::muted(),
    )));
    lines.push(Line::from(Span::styled(
        "输入任务回车开始 · ? 看键位 · / 看命令",
        theme::dim(),
    )));
    lines
}

/// 小 Logo（hub 运行态顶栏左侧）。
pub fn small(version: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("SD", Style::default().fg(theme::ACCENT)),
        Span::styled(format!(" · {version}"), theme::muted()),
    ])
}
