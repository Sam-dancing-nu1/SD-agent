//! 渲染门面：一帧组装（hub / worker 布局 + 浮层 + 状态栏）。
//!
//! 渲染只读 App 状态；样式取 theme、键位提示取 keymap（同源）。

mod chat;
#[cfg(test)]
mod frame_tests;
mod hub;
pub mod layout;
pub mod logo;
mod logo_raster;
mod panels;
mod stats;
pub mod theme;
mod widgets;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{App, Focus, Mode};

use self::layout::HitRects;

/// 绘制一帧，返回本帧可交互组件区域（鼠标命中检测用）。
pub fn draw(app: &App, frame: &mut ratatui::Frame) -> HitRects {
    let area = frame.area();
    // 主区 + 底部状态栏。
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    let mut hits;
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
            frame.render_widget(status_bar(app.version, mode, status), rows[1]);
            hits = hub::draw(rows[0], h, app.focus, app.version, frame);
        }
        Mode::Worker(w) => {
            let status = if w.running.is_some() {
                "任务执行中"
            } else {
                "空闲 · 可输入"
            };
            frame.render_widget(status_bar(app.version, "会话 · worker", status), rows[1]);
            hits = draw_worker(rows[0], app, w, frame);
        }
    }

    // 浮层（后画盖在上层）。
    if app.help_open {
        widgets::draw_help(frame, area);
    }
    if app.slash_open {
        if let Some(rect) =
            widgets::draw_slash(frame, area, app.input_text(), app.slash_sel, hits.input)
        {
            hits.slash = Some(rect);
        }
    }
    if let Mode::Worker(w) = &app.mode {
        if w.approval.is_some() {
            widgets::draw_approval(frame, area, w);
        }
    }
    // 模型选择浮层（读 HubState.model_open；键位 ↑↓/Enter/Esc 由交互层）。
    if let Mode::Hub(h) = &app.mode {
        if h.model_open {
            widgets::draw_model_popup(frame, area, &h.profiles, h.profile_sel);
        }
    }
    // 配置表单弹窗（最上层；读 FormState）。
    if let Mode::Hub(h) = &app.mode {
        if let Some(form) = &h.form {
            hits.form = Some(widgets::draw_form(frame, area, form));
        }
    }
    hits
}

/// worker 布局：状态条 + 对话流 + 输入框。
fn draw_worker(
    area: Rect,
    app: &App,
    w: &crate::app::worker::WorkerState,
    frame: &mut ratatui::Frame,
) -> HitRects {
    let mut hits = HitRects::default();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 状态提示条
            Constraint::Min(5),    // 对话流
            Constraint::Length(4), // 输入框
        ])
        .split(area);

    frame.render_widget(widgets::worker_note(w), rows[0]);

    // 对话流（尾部窗口：scroll=距底部偏移，0=贴底跟随）+ 侧边滚动条。
    let window_h = rows[1].height.saturating_sub(2) as usize;
    let inner_w = rows[1].width.saturating_sub(2) as usize;
    let text = chat::render_feed_window(&w.feed, w.scroll, window_h, inner_w);
    let chat_block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(" 对话 ")
        .title_style(theme::title());
    let p = Paragraph::new(text)
        .block(chat_block)
        .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(p, rows[1]);
    let (total, start) = chat::feed_window_metrics(&w.feed, w.scroll, window_h, inner_w);
    widgets::draw_scrollbar(frame, rows[1], total, window_h.min(total), start);
    hits.chat = Some(rows[1]);

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

    hits.input = Some(rows[2]);

    // 空态引导（无内容时给一句起点，不是空白屏）。
    if w.feed.is_empty() {
        let hint = Paragraph::new(Line::from("  输入第一条任务开始 · / 看命令 · ? 看键位"))
            .style(theme::dim());
        frame.render_widget(hint, rows[1]);
    }
    hits
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

#[cfg(test)]
mod frame_dump {
    //! 初始态 / WORK 态 ASCII 帧 dump（`cargo test -p sd-tui frame_dump -- --nocapture`
    //! 查看；布局回归时肉眼比对，断言只锁关键内容存在）。

    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::app::{App, Mode};

    const W: u16 = 120;
    const H: u16 = 40;

    fn dump(app: &mut App) -> String {
        let backend = TestBackend::new(W, H);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| {
                let _hits = super::draw(app, f);
            })
            .expect("draw");
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..H {
            for x in 0..W {
                if let Some(cell) = buf.cell((x, y)) {
                    out.push_str(cell.symbol());
                }
            }
            out.push('\n');
        }
        out
    }

    /// 存在性断言（口径同 selfcheck::expect：CJK 宽字符 padding 先去空格）。
    fn has(buf: &str, needle: &str) -> bool {
        buf.contains(needle) || buf.replace(' ', "").contains(&needle.replace(' ', ""))
    }

    #[test]
    fn initial_and_work_frames() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        // 进程独占根目录：防旧会话残留把初始态误判成 WORK 态。
        let root = std::env::temp_dir().join(format!("sd-tui-frame-dump-{}", std::process::id()));
        let mut app = App::new_hub(root, "0.1.0-alpha.1", rt);
        let initial = dump(&mut app);
        println!("── 初始态（{W}x{H}）──\n{initial}");
        assert!(has(&initial, "任务"), "初始态应有任务输入框");
        assert!(has(&initial, "输入任务"), "初始态输入框应有占位提示");
        assert!(has(&initial, "▾"), "初始态应有模型信息行（切换形态）");
        assert!(has(&initial, "M 模型"), "初始态应有快捷键提示行");
        assert!(has(&initial, "Tip"), "初始态应有 Tip 提示");
        assert!(has(&initial, "历史会话"), "初始态底部应有历史面板");
        assert!(has(&initial, "Token 消耗"), "初始态底部应有 token 面板");
        assert!(has(&initial, "消耗热力图"), "初始态 token 面板应有热力图");

        // 动画首帧（ToWork 刚触发，progress≈0）：仍是初始态控件行。
        if let Mode::Hub(h) = &mut app.mode {
            h.launched = true;
            h.anim.start(crate::app::anim::AnimKind::ToWork);
        }
        let mid = dump(&mut app);
        assert!(has(&mid, "▾"), "动画首帧仍应是初始态模型信息行");

        // WORK 态定格（无动画，launched=true）。
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        let root = std::env::temp_dir().join(format!("sd-tui-frame-dump-{}", std::process::id()));
        let mut app = App::new_hub(root, "0.1.0-alpha.1", rt);
        if let Mode::Hub(h) = &mut app.mode {
            h.launched = true;
        }
        let work = dump(&mut app);
        println!("── WORK 态（{W}x{H}）──\n{work}");
        assert!(has(&work, "Token 消耗"), "WORK 态应有统计面板");
        assert!(has(&work, "Token 消耗总览"), "WORK 态统计应为全量总览");
    }

    /// 斜杠浮层选中高亮 + HitRects 登记 + 初始态底部两栏/角落版本号几何断言。
    #[test]
    fn slash_overlay_and_panel_hits() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        let root =
            std::env::temp_dir().join(format!("sd-tui-frame-dump-slash-{}", std::process::id()));
        let mut app = App::new_hub(root, "0.1.0-alpha.1", rt);
        app.slash_open = true;
        app.slash_sel = 2;
        let backend = TestBackend::new(W, H);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut hits = crate::ui::layout::HitRects::default();
        terminal
            .draw(|f| {
                hits = super::draw(&app, f);
            })
            .expect("draw");
        let buf = terminal.backend().buffer();

        // 斜杠浮层登记 + 选中行 ▶ 高亮符号落在 slash_sel 行。
        let rect = hits.slash.expect("斜杠浮层应登记 rect");
        let mut marker_y: Option<u16> = None;
        for y in 0..H {
            for x in 0..W {
                if buf.cell((x, y)).map(|c| c.symbol()) == Some("▶") {
                    marker_y = Some(y);
                }
            }
        }
        let my = marker_y.expect("选中行应有 ▶ 高亮符号");
        assert_eq!(my, rect.y + 1 + app.slash_sel as u16, "▶ 应落在浮层选中行");

        // 关浮层再画一帧：断言被浮层盖住的初始态角落/面板登记。
        app.slash_open = false;
        let mut hits = crate::ui::layout::HitRects::default();
        terminal
            .draw(|f| {
                hits = super::draw(&app, f);
            })
            .expect("draw");
        let buf = terminal.backend().buffer();

        // 初始态底部两栏登记 + 右下角版本号（token 面板内侧末行右端）。
        let tp = hits.token_panel.expect("初始态应登记 token 面板");
        assert!(hits.history_panel.is_some(), "初始态应登记历史面板");
        let vy = tp.bottom() - 2;
        let row_text: String = (tp.x..tp.right())
            .filter_map(|x| buf.cell((x, vy)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(
            row_text.contains("0.1.0-alpha.1"),
            "token 面板右下角应有版本号: {row_text:?}"
        );
        // 模型信息行登记：整行=模型浮层入口，分段档=思考强度滑条。
        assert!(hits.model_bar.is_some(), "初始态应登记模型信息行");
        assert!(hits.effort_slider.is_some(), "初始态应登记思考强度分段档");
    }

    #[test]
    fn worker_chat_scrollbar() {
        // 对话流超窗高时应画出侧边滚动条（自绘 '█' 滑块；feed 本身无该字符）。
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        let mut app = App::new_worker(
            std::env::temp_dir().join("sd-tui-frame-dump-worker"),
            "0.1.0-alpha.1",
            rt,
            None,
        );
        if let Mode::Worker(w) = &mut app.mode {
            for i in 0..60 {
                w.push(crate::app::worker::FeedItem::User(format!("消息 {i}")));
            }
        }
        let buf = dump(&mut app);
        assert!(buf.contains('█'), "超窗高的对话流应有滚动条滑块");
    }
}
