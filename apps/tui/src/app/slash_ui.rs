//! 斜杠命令浮层交互：选中状态机（↑↓/PgUp/PgDn/Enter/Esc）与鼠标行定位。
//!
//! 契约（冻结）：App.slash_sel（选中索引，交互层写、渲染层读）与
//! HitRects.slash（浮层整体 rect，渲染路每帧登记）。几何口径（与渲染侧
//! 一致）：浮层 rect 内容首行 = rect.y+1（顶边框一行），行序 =
//! crate::slash::filter(typed) 顺序。hub 与 worker 两模式共用本状态机。

use ratatui::layout::Rect;

use super::App;
use crate::slash;

/// Enter 的执行目标：选中命令 vs 整行文本。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecTarget {
    /// 执行过滤列表中选中的命令（命令名，不含参数）。
    Selected(&'static str),
    /// 过滤不到命令：整行走斜杠路由（如 /effort high 带参命令）。
    WholeLine,
}

/// 选中索引 clamp：空列表归 0，否则夹到末项（过滤数随输入动态变化）。
pub fn clamp_sel(sel: usize, n: usize) -> usize {
    if n == 0 {
        0
    } else {
        sel.min(n - 1)
    }
}

/// ↑↓/PgUp/PgDn/滚轮移动选中（up=向列表头部）。边界不环绕（clamp）。
pub fn step_sel(sel: usize, n: usize, up: bool) -> usize {
    if n == 0 {
        return 0;
    }
    if up {
        sel.saturating_sub(1)
    } else {
        sel.saturating_add(1)
    }
    .min(n - 1)
}

/// Enter 执行目标（浮层开着时调用）：有匹配 = 执行选中项（sel 越界 clamp）。
pub fn exec_target(typed: &str, sel: usize) -> ExecTarget {
    let m = slash::filter(typed);
    match m.get(clamp_sel(sel, m.len())) {
        Some(c) => ExecTarget::Selected(c.name),
        None => ExecTarget::WholeLine,
    }
}

/// 浮层内 (y) → 行索引（内容首行 = rect.y+1；行序 = filter 顺序；
/// 点在边框/超出可见行/超出过滤数 = None）。`n` = 过滤后命令数。
pub fn row_at(rect: Rect, y: u16, n: usize) -> Option<usize> {
    if y <= rect.y || y >= rect.y.saturating_add(rect.height).saturating_sub(1) {
        return None;
    }
    let inner_h = rect.height.saturating_sub(2) as usize;
    let row = (y - rect.y - 1) as usize;
    (row < n.min(inner_h)).then_some(row)
}

impl App {
    /// 每次状态变更后同步浮层态：开合跟随输入前缀、选中 clamp
    /// （输入删光归 0；抑制态见 slash_dismiss）。
    pub(crate) fn sync_slash(&mut self) {
        let t = self.input_text();
        if !t.starts_with('/') {
            // 前缀消失：解除抑制、复位。
            self.slash_dismissed = false;
            self.slash_open = false;
            self.slash_sel = 0;
            return;
        }
        let n = slash::filter(t).len();
        self.slash_open = !self.slash_dismissed && t.len() < 24 && n > 0;
        self.slash_sel = clamp_sel(self.slash_sel, n);
    }

    /// ↑↓/PgUp/PgDn/滚轮：移动选中（clamp 到过滤后命令数）。
    pub(crate) fn slash_move(&mut self, up: bool) {
        let n = slash::filter(self.input_text()).len();
        self.slash_sel = step_sel(self.slash_sel, n, up);
    }

    /// Esc：关浮层并清空输入（选中归 0）。
    pub(crate) fn slash_cancel(&mut self) {
        if let Some(ed) = self.editor_opt_pub() {
            ed.clear();
        }
        self.slash_open = false;
        self.slash_sel = 0;
        self.slash_dismissed = false;
    }

    /// 浮层外点击：只关浮层（输入保留；抑制自动重开直到 '/' 前缀消失）。
    pub(crate) fn slash_dismiss(&mut self) {
        self.slash_open = false;
        self.slash_dismissed = true;
    }

    /// 浮层内点击：按 y 定位行，选中并立即执行（与 Enter 同路）。
    /// 点在边框/空白行：吞掉点击不动作。返回是否消费本次点击。
    pub(crate) fn slash_click(&mut self, x: u16, y: u16) -> bool {
        let Some(rect) = self.hit.slash else {
            self.slash_dismiss();
            return true;
        };
        if !rect.contains((x, y).into()) {
            self.slash_dismiss();
            return true;
        }
        let n = slash::filter(self.input_text()).len();
        if let Some(row) = row_at(rect, y, n) {
            self.slash_sel = row;
            self.submit_input();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Mode};
    use crate::keymap::Action;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect::new(x, y, w, h)
    }

    fn test_app(name: &str) -> App {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio");
        let root = std::env::temp_dir().join(format!("sd-tui-test-slash-{name}"));
        App::new_hub(root, "0.0.0-test", rt)
    }

    fn click(x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn clamp_and_step_follow_filter_len() {
        // 空列表（输入删光/无匹配）恒 0。
        assert_eq!(clamp_sel(0, 0), 0);
        assert_eq!(clamp_sel(5, 0), 0);
        // 过滤数变小 → clamp 末项。
        assert_eq!(clamp_sel(5, 3), 2);
        assert_eq!(clamp_sel(1, 3), 1);
        // ↑↓ 边界不环绕。
        assert_eq!(step_sel(0, 3, true), 0);
        assert_eq!(step_sel(0, 3, false), 1);
        assert_eq!(step_sel(2, 3, false), 2);
        assert_eq!(step_sel(2, 3, true), 1);
        assert_eq!(step_sel(9, 0, false), 0);
    }

    #[test]
    fn enter_target_is_selected_command_not_whole_line() {
        // 命令表顺序：/help /doctor /model …。
        assert_eq!(exec_target("/", 1), ExecTarget::Selected("/doctor"));
        assert_eq!(exec_target("/do", 0), ExecTarget::Selected("/doctor"));
        assert_eq!(exec_target("/do", 5), ExecTarget::Selected("/doctor")); // sel 越界 clamp
        assert_eq!(exec_target("/v", 0), ExecTarget::Selected("/version"));
        // 无匹配（带参命令/未知前缀）：整行走斜杠路由。
        assert_eq!(exec_target("/effort high", 0), ExecTarget::WholeLine);
        assert_eq!(exec_target("/zzz", 0), ExecTarget::WholeLine);
    }

    #[test]
    fn mouse_row_maps_content_first_row() {
        // 浮层 (10,5,30,6)：边框 y=5/10，内容行 y=6..9 → 行 0..3。
        let r = rect(10, 5, 30, 6);
        assert_eq!(row_at(r, 5, 12), None); // 顶边框
        assert_eq!(row_at(r, 10, 12), None); // 底边框
        assert_eq!(row_at(r, 6, 12), Some(0));
        assert_eq!(row_at(r, 8, 12), Some(2));
        assert_eq!(row_at(r, 9, 12), Some(3));
        // 超出过滤行数/空列表的行不命中。
        assert_eq!(row_at(r, 8, 2), None);
        assert_eq!(row_at(r, 6, 0), None);
    }

    #[test]
    fn slash_state_machine_clamps_and_resets() {
        let mut app = test_app("state");
        // 键入 "/" → 浮层开、选中 0。
        app.dispatch(Action::Insert('/'));
        assert!(app.slash_open);
        assert_eq!(app.slash_sel, 0);
        // 过滤到 1 项（/do → /doctor）：↑↓ 都 clamp 到 0。
        for ch in "do".chars() {
            app.dispatch(Action::Insert(ch));
        }
        app.dispatch(Action::SlashNext);
        app.dispatch(Action::SlashPrev);
        assert_eq!(app.slash_sel, 0);
        // 回到 "/"（12 项）：↓ 连按 clamp 到末项，↑ 回退。
        app.dispatch(Action::Backspace);
        app.dispatch(Action::Backspace);
        for _ in 0..20 {
            app.dispatch(Action::SlashNext);
        }
        assert_eq!(app.slash_sel, 11);
        app.dispatch(Action::SlashPrev);
        assert_eq!(app.slash_sel, 10);
        // 删光：浮层关、选中归 0（输入删光归 0）。
        app.dispatch(Action::Backspace);
        assert!(!app.slash_open);
        assert_eq!(app.slash_sel, 0);
    }

    #[test]
    fn enter_executes_slash_sel_item() {
        let mut app = test_app("enter");
        app.dispatch(Action::Insert('/'));
        app.dispatch(Action::SlashNext); // sel=1 (/doctor)
        app.dispatch(Action::SlashNext); // sel=2 (/model)
        app.dispatch(Action::SlashConfirm);
        // 执行选中项而非整行：输入已提交清空、浮层关、/model 落到状态栏。
        assert!(!app.slash_open);
        assert_eq!(app.input_text(), "");
        assert!(
            matches!(&app.mode, Mode::Hub(h) if h.status.contains("/model")),
            "应执行选中项 /model"
        );
    }

    #[test]
    fn esc_closes_and_clears_input() {
        let mut app = test_app("esc");
        app.dispatch(Action::Insert('/'));
        app.dispatch(Action::Insert('d'));
        app.dispatch(Action::SlashCancel);
        assert!(!app.slash_open);
        assert_eq!(app.input_text(), "");
        assert_eq!(app.slash_sel, 0);
    }

    #[test]
    fn mouse_click_row_selects_and_executes() {
        let mut app = test_app("mouse-row");
        app.dispatch(Action::Insert('/'));
        app.hit.slash = Some(rect(10, 5, 30, 6));
        // 点 y=8 → 内容行 2 → 命令表第 3 项 /model 立即执行。
        app.handle_mouse(click(12, 8));
        assert!(!app.slash_open);
        assert_eq!(app.input_text(), "");
        assert!(matches!(&app.mode, Mode::Hub(h) if h.status.contains("/model")));
    }

    #[test]
    fn mouse_click_outside_dismisses_without_reopen() {
        let mut app = test_app("mouse-outside");
        app.dispatch(Action::Insert('/'));
        app.dispatch(Action::Insert('d'));
        app.hit.slash = Some(rect(10, 5, 30, 6));
        // 浮层外点击 = 关浮层（输入保留）。
        app.handle_mouse(click(0, 0));
        assert!(!app.slash_open);
        assert_eq!(app.input_text(), "/d");
        // 继续输入不自动重开（抑制态），前缀删光后解除。
        app.dispatch(Action::Insert('o'));
        assert!(!app.slash_open);
        app.dispatch(Action::Backspace);
        app.dispatch(Action::Backspace);
        app.dispatch(Action::Backspace);
        app.dispatch(Action::Insert('/'));
        assert!(app.slash_open);
    }

    #[test]
    fn mouse_scroll_in_overlay_moves_selection() {
        let mut app = test_app("mouse-scroll");
        app.dispatch(Action::Insert('/'));
        app.hit.slash = Some(rect(10, 5, 30, 6));
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 12,
            row: 8,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.slash_sel, 1);
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 12,
            row: 8,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.slash_sel, 0);
    }
}
