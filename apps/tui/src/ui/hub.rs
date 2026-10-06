//! hub 主页布局：初始态（大 Logo + 空白对话框）/ WORK 态（顶栏 + 历史 + 统计）。
//!
//! 两态判定在 app::hub::HubState::is_work（有历史或已发起任务即 WORK）。

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::app::Focus;
use crate::app::hub::HubState;
use crate::ui::theme;

/// 渲染 hub（占满给定区域，状态栏由 ui::status 绘制）。
pub fn draw(area: Rect, hub: &HubState, focus: Focus, version: &str, frame: &mut ratatui::Frame) {
    if hub.is_work() {
        draw_work(area, hub, focus, version, frame);
    } else {
        draw_initial(area, hub, focus, version, frame);
    }
}

/// 初始态：大 Logo 居中 + 下方空白对话框。
fn draw_initial(
    area: Rect,
    hub: &HubState,
    focus: Focus,
    version: &str,
    frame: &mut ratatui::Frame,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),    // Logo 区（尽量大）
            Constraint::Length(5), // 对话框
            Constraint::Length(1), // 状态提示
        ])
        .split(area);

    // Logo 居中。
    let logo_lines = super::logo::big(version);
    let logo = Paragraph::new(logo_lines).alignment(Alignment::Center);
    frame.render_widget(logo, rows[0]);

    draw_input(rows[1], hub, focus, frame, "输入任务，回车开新会话");

    let hint = Paragraph::new(Line::from(Span::styled(
        "Enter 开始 · ? 键位 · / 命令 · q 退出",
        theme::dim(),
    )))
    .alignment(Alignment::Center);
    frame.render_widget(hint, rows[2]);
}

/// WORK 态：顶栏（小 Logo + 输入框）+ 下方左 1/3 历史、右 2/3 统计。
fn draw_work(area: Rect, hub: &HubState, focus: Focus, version: &str, frame: &mut ratatui::Frame) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(6)])
        .split(area);

    // 顶栏：Logo 左上、输入框在其右侧。
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(20)])
        .split(rows[0]);

    let logo = super::logo::small(version);
    let logo_block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border());
    let logo_p = Paragraph::new(logo).block(logo_block);
    frame.render_widget(logo_p, top[0]);

    draw_input(top[1], hub, focus, frame, "新任务，回车开新会话");

    // 下方左右分栏 1:2。
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(33), Constraint::Percentage(67)])
        .split(rows[1]);

    // 左：历史会话列表（可滚动）。
    let items: Vec<ListItem> = hub
        .sessions
        .iter()
        .map(|s| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("● {}", s.title), theme::text()),
                Span::styled(format!("  ({} 条)", s.message_count), theme::dim()),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .title(" 历史会话 ")
                .title_style(theme::title())
                .borders(Borders::ALL)
                .border_style(if focus == Focus::List {
                    theme::border_focus()
                } else {
                    theme::border()
                }),
        )
        .highlight_style(theme::selected())
        .highlight_symbol("▶ ");
    let mut state = ListState::default().with_selected(Some(hub.selected));
    frame.render_stateful_widget(list, cols[0], &mut state);

    // 右：Token 统计总览 + 热力图。
    let stat_lines = super::stats::render_stats(&hub.stats);
    let stats_p = Paragraph::new(stat_lines).block(
        Block::default()
            .title(" Token 消耗 ")
            .title_style(theme::title())
            .borders(Borders::ALL)
            .border_style(theme::border()),
    );
    frame.render_widget(stats_p, cols[1]);
}

/// 输入框（两种布局共用）。
fn draw_input(
    area: Rect,
    hub: &HubState,
    focus: Focus,
    frame: &mut ratatui::Frame,
    placeholder: &str,
) {
    let focused = focus == Focus::Input;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if focused {
            theme::border_focus()
        } else {
            theme::border()
        })
        .title(" 任务 ")
        .title_style(theme::title());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if hub.editor.text.is_empty() {
        let ph = Paragraph::new(Line::from(Span::styled(placeholder, theme::dim())));
        frame.render_widget(ph, inner);
    } else {
        let p = Paragraph::new(hub.editor.text.clone())
            .style(theme::text())
            .wrap(ratatui::widgets::Wrap { trim: false });
        frame.render_widget(p, inner);
        if focused {
            // 光标按显示宽度定位（CJK=2 列近似；多行折行不追踪——已知限制）。
            let col = super::cursor_display_col(&hub.editor.text, hub.editor.cursor());
            let cx = inner.x + col.min(inner.width.saturating_sub(1));
            frame.set_cursor_position((cx, inner.y));
        }
    }
}

/// 供 ui::mod 复用的状态栏行（全 owned，免生命周期拼接）。
pub fn status_line(version: &str, mode: &str, status: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(" SD ", theme::accent().add_modifier(Modifier::BOLD)),
        Span::styled(version.to_string(), theme::muted()),
        Span::styled(" │ ", theme::dim()),
        Span::styled(mode.to_string(), theme::info()),
        Span::styled(" │ ", theme::dim()),
        Span::styled(status.to_string(), theme::dim()),
    ])
}
