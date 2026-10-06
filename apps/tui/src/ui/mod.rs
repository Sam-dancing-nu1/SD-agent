//! 渲染门面：一帧组装（hub / worker 布局 + 浮层 + 状态栏）。
//!
//! 渲染只读 App 状态；样式取 theme、键位提示取 keymap（同源）。

mod chat;
mod hub;
pub mod logo;
mod stats;
pub mod theme;
mod widgets;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{App, Focus, Mode};

/// 绘制一帧。
pub fn draw(app: &App, frame: &mut ratatui::Frame) {
    let area = frame.area();
    // 主区 + 底部状态栏。
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    match &app.mode {
        Mode::Hub(h) => {
            let mode = if h.is_work() {
                "主页 · WORK"
            } else {
                "主页"
            };
            let status = if h.status.is_empty() {
                "输入任务回车开始"
            } else {
                &h.status
            };
            hub::draw(rows[0], h, app.focus, app.version, frame);
            frame.render_widget(status_bar(app.version, mode, status), rows[1]);
        }
        Mode::Worker(w) => {
            draw_worker(rows[0], app, w, frame);
            let status = if w.running.is_some() {
                "任务执行中"
            } else {
                "空闲 · 可输入"
            };
            frame.render_widget(status_bar(app.version, "会话 · worker", status), rows[1]);
        }
    }

    // 浮层（后画盖在上层）。
    if app.help_open {
        widgets::draw_help(frame, area);
    }
    if app.slash_open {
        widgets::draw_slash(frame, area, app.input_text());
    }
    if let Mode::Worker(w) = &app.mode {
        if w.approval.is_some() {
            widgets::draw_approval(frame, area, w);
        }
    }
}

/// worker 布局：状态条 + 对话流 + 输入框。
fn draw_worker(
    area: Rect,
    app: &App,
    w: &crate::app::worker::WorkerState,
    frame: &mut ratatui::Frame,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 状态提示条
            Constraint::Min(5),    // 对话流
            Constraint::Length(4), // 输入框
        ])
        .split(area);

    frame.render_widget(widgets::worker_note(w), rows[0]);

    // 对话流（尾部窗口：scroll=距底部偏移，0=贴底跟随）。
    let window_h = rows[1].height.saturating_sub(2) as usize;
    let text = chat::render_feed_window(&w.feed, w.scroll, window_h);
    let chat_block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(" 对话 ")
        .title_style(theme::title());
    let p = Paragraph::new(text)
        .block(chat_block)
        .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(p, rows[1]);

    // 输入框。
    let focused = app.focus == Focus::Input;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if focused {
            theme::border_focus()
        } else {
            theme::border()
        })
        .title(" 输入（Enter 发送 · Alt+Enter 换行） ")
        .title_style(theme::title());
    let inner = block.inner(rows[2]);
    frame.render_widget(block, rows[2]);
    let input = Paragraph::new(w.editor.text.clone())
        .style(theme::text())
        .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(input, inner);
    if focused {
        let col = cursor_display_col(&w.editor.text, w.editor.cursor());
        let cx = inner.x + col.min(inner.width.saturating_sub(1));
        let cy = inner.y;
        frame.set_cursor_position((cx, cy));
    }

    // 空态引导（无内容时给一句起点，不是空白屏）。
    if w.feed.is_empty() {
        let hint = Paragraph::new(Line::from("  输入第一条任务开始 · / 看命令 · ? 看键位"))
            .style(theme::dim());
        frame.render_widget(hint, rows[1]);
    }
}

/// 光标显示列（CJK 宽字符按 2 列近似；多行/折行不精确定位——已知限制，
/// 与 ui/hub.rs 共用同一口径）。
pub fn cursor_display_col(text: &str, cursor: usize) -> u16 {
    let cursor = cursor.min(text.len());
    text[..cursor]
        .chars()
        .map(|c| if is_wide(c) { 2u16 } else { 1 })
        .sum()
}

/// 宽字符近似判定（CJK/全角区；不引 unicode-width 依赖）。
fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF | 0xFE30..=0xFE6F | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
}

/// 底部状态栏（一行）。
fn status_bar(version: &str, mode: &str, status: &str) -> Paragraph<'static> {
    hub::status_line(version, mode, status).into_paragraph()
}

/// Line → Paragraph 便捷转换（状态栏）。
trait IntoParagraph<'a> {
    fn into_paragraph(self) -> Paragraph<'a>;
}

impl<'a> IntoParagraph<'a> for Line<'a> {
    fn into_paragraph(self) -> Paragraph<'a> {
        Paragraph::new(self)
    }
}
