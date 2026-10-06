//! 通用浮层组件：居中弹窗矩形、帮助浮层、斜杠命令浮层、审批弹窗。

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::app::worker::WorkerState;
use crate::keymap;
use crate::slash;
use crate::ui::theme;

/// 居中弹窗矩形（按百分比占父区域）。
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vert[1])[1]
}

/// 帮助浮层（键位表，与 keymap::HELP 同源）。
pub fn draw_help(frame: &mut ratatui::Frame, area: Rect) {
    let rect = centered_rect(64, 70, area);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .title(" 键位（? 关闭） ")
        .title_style(theme::title())
        .borders(Borders::ALL)
        .border_style(theme::border_focus())
        .style(theme::text().bg(theme::POPUP_BG));
    let mut lines = vec![Line::from("")];
    for (k, d) in keymap::HELP {
        lines.push(Line::from(vec![
            Span::styled(format!("  {k:<14}"), theme::accent()),
            Span::styled(d.to_string(), theme::text()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  命令：/help /doctor /model /settings /effort /new",
        theme::muted(),
    )));
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, rect);
}

/// 斜杠命令浮层（输入以 / 开头时弹出，按前缀过滤）。
pub fn draw_slash(frame: &mut ratatui::Frame, area: Rect, typed: &str) {
    let matches = slash::filter(typed);
    if matches.is_empty() {
        return;
    }
    let height = (matches.len() as u16 + 2).min(14);
    let rect = Rect {
        x: area.x + 2,
        y: area.bottom().saturating_sub(height + 8),
        width: area.width.saturating_sub(4).min(56),
        height,
    };
    frame.render_widget(Clear, rect);
    let items: Vec<ListItem> = matches
        .iter()
        .map(|c| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<14}", c.name), theme::accent()),
                Span::styled(c.desc, theme::muted()),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border_focus())
                .style(theme::text().bg(theme::POPUP_BG)),
        )
        .highlight_style(theme::selected())
        .highlight_symbol("▶ ");
    let mut state = ListState::default().with_selected(Some(0));
    frame.render_stateful_widget(list, rect, &mut state);
}

/// 审批弹窗（run 线程阻塞等待裁决）。
pub fn draw_approval(frame: &mut ratatui::Frame, area: Rect, w: &WorkerState) {
    let Some(pending) = &w.approval else {
        return;
    };
    let rect = centered_rect(72, 56, area);
    frame.render_widget(Clear, rect);
    let req = &pending.request;
    let block = Block::default()
        .title(" ⚠ 工具审批")
        .title_style(theme::accent())
        .borders(Borders::ALL)
        .border_style(theme::border_focus())
        .style(theme::text().bg(theme::POPUP_BG));
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  工具：", theme::muted()),
            Span::styled(req.tool.clone(), theme::info()),
        ]),
        Line::from(vec![
            Span::styled("  摘要：", theme::muted()),
            Span::styled(req.summary.clone(), theme::text()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Y 放行 · A 本次会话全放行 · N 拒绝（Esc 同）",
            theme::accent(),
        )),
        Line::from(""),
    ];
    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, rect);
}

/// worker 顶部状态提示条（非浮层）。
pub fn worker_note<'a>(w: &WorkerState) -> Line<'a> {
    let running = match w.running {
        Some(_) => "● 运行中",
        None => "○ 空闲",
    };
    Line::from(Span::styled(
        format!("{running} · Enter 发送 · Esc 取消 · ? 键位 · / 命令"),
        theme::dim(),
    ))
}
