//! hub 内容两栏：左 1/3 历史会话（滚动条）+ 右 2/3 Token 统计（热力图）。
//!
//! 初始态（图1 底部两栏）与 WORK 态共用同一组件与标题样式（边框/标题风格
//! 统一）；compact=true 时 token 面板走精简内容（累计 + 热力图）。
//! 每帧登记 HitRects 的 history/stats（鼠标命中）与 history_panel/token_panel
//! （面板登记，契约 ui/layout.rs）。

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::app::hub::HubState;
use crate::app::Focus;
use crate::ui::layout::HitRects;
use crate::ui::theme;
use crate::ui::widgets;

/// 两栏分割：左 1/3 历史 / 右 2/3 token 记录（两态一致）。
const HISTORY_PCT: u16 = 33;

/// 渲染内容两栏并登记可交互区。
pub fn draw(
    rect: Rect,
    hub: &HubState,
    focus: Focus,
    compact: bool,
    frame: &mut ratatui::Frame,
    hits: &mut HitRects,
) {
    if rect.width < 10 || rect.height < 3 {
        return;
    }
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(HISTORY_PCT),
            Constraint::Percentage(100 - HISTORY_PCT),
        ])
        .split(rect);

    draw_history(cols[0], hub, focus, frame);
    hits.history = Some(cols[0]);
    hits.history_panel = Some(cols[0]);

    draw_token(cols[1], hub, compact, frame);
    hits.stats = Some(cols[1]);
    hits.token_panel = Some(cols[1]);
}

/// 左栏：历史会话列表（可滚动 + 侧边滚动条；空列表给占位提示）。
fn draw_history(rect: Rect, hub: &HubState, focus: Focus, frame: &mut ratatui::Frame) {
    let block = Block::default()
        .title(" 历史会话 ")
        .title_style(theme::title())
        .borders(Borders::ALL)
        .border_style(if focus == Focus::List {
            theme::border_focus()
        } else {
            theme::border()
        });
    let items: Vec<ListItem> = if hub.sessions.is_empty() {
        vec![ListItem::new(Line::from(ratatui::text::Span::styled(
            "（暂无会话）",
            theme::dim(),
        )))]
    } else {
        hub.sessions
            .iter()
            .map(|s| {
                ListItem::new(Line::from(vec![
                    ratatui::text::Span::styled(format!("● {}", s.title), theme::text()),
                    ratatui::text::Span::styled(
                        format!("  ({} 条)", s.message_count),
                        theme::dim(),
                    ),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selected())
        .highlight_symbol("▶ ");
    let sel = if hub.sessions.is_empty() {
        None
    } else {
        Some(hub.selected.min(hub.sessions.len() - 1))
    };
    let mut state = ListState::default().with_selected(sel);
    frame.render_stateful_widget(list, rect, &mut state);
    let visible_rows = rect.height.saturating_sub(2) as usize;
    widgets::draw_scrollbar(
        frame,
        rect,
        hub.sessions.len(),
        visible_rows,
        state.offset(),
    );
}

/// 右栏：Token 讟录（WORK 态全量总览 / 初始态精简）。
fn draw_token(rect: Rect, hub: &HubState, compact: bool, frame: &mut ratatui::Frame) {
    let lines = if compact {
        super::stats::render_stats_compact(&hub.stats)
    } else {
        super::stats::render_stats(&hub.stats)
    };
    let block = Block::default()
        .title(" Token 消耗 ")
        .title_style(theme::title())
        .borders(Borders::ALL)
        .border_style(theme::border());
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}
