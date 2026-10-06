//! 鼠标交互全量适配：单击/双击/滚轮/拖动/拖选（OSC 52 复制）。
//!
//! 命中判定消费 ui::layout::HitRects（每帧 ui::draw 写入 App.hit）。
//! 几何口径（渲染侧需一致，契约缺口见交付报告）：
//! - 列表 y→项：块矩形含 1 格边框，内容首行 = rect.y+1；
//!   scroll = 距底部偏移（与 WorkerState::scroll 同口径），尾部窗口取行。
//! - 滑条 x→档位：端点对齐（rect.x=0 档，rect.right()-1=末档），四舍五入。
//! - 表单 y→槽位：内容首行 = rect.y+1，FORM_FIELDS 顺序各一行，
//!   第 5 行留白，第 6 行按钮（左半=保存、右半=取消）。
//! - 模型浮层无独立 HitRects 字段（契约缺口）：打开期间点浮层外=关闭并
//!   吞掉点击，滚轮=移动高亮，键盘 ↑↓/Enter/Esc 全覆盖。
//! - 斜杠浮层消费 HitRects.slash（契约字段）：rect 内容首行 = rect.y+1，
//!   行序 = slash::filter(typed) 顺序；点行=选中并立即执行，点外=关浮层
//!   （不吞底层点击语义以外的动作：本次点击整体吞掉防误触），滚轮=切选中。

use std::time::{Duration, Instant};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::form::{FORM_FIELDS, FormField};
use super::{App, Focus, Mode};
use crate::keymap::Action;
use crate::ui::layout::{HitRects, HitTarget};

/// 双击判定窗（同格 500ms 内二次按下）。
const DOUBLE_CLICK_MS: u64 = 500;

/// 滚轮步长（与 PgUp/PgDn 的 scroll_by 同步长）。
pub const SCROLL_STEP: i32 = 3;

/// 拖动模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragKind {
    /// 滑条按住拖动（实时改档）。
    Slider,
    /// 文本拖选（记录归属区，松开复制）。
    Select(HitTarget),
}

/// 鼠标交互状态（按下锚点 / 拖动模式 / 双击检测）。
#[derive(Debug, Default)]
pub struct MouseState {
    drag: Option<DragKind>,
    anchor: Option<(u16, u16)>,
    moved: bool,
    last_down: Option<(u16, u16, Instant)>,
}

/// 表单内命中槽位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormSlot {
    Field(FormField),
    Save,
    Cancel,
    None,
}

/// 双击判定（纯函数，可测）。
fn is_double(last: Option<(u16, u16, Instant)>, x: u16, y: u16) -> bool {
    matches!(last, Some((lx, ly, t)) if (lx, ly) == (x, y)
        && t.elapsed() < Duration::from_millis(DOUBLE_CLICK_MS))
}

/// 滚动偏移更新（距底部偏移口径；滚轮上=看更早=偏移增大）。纯函数，可测。
pub fn scroll_offset(cur: u16, delta: i32, max: u16) -> u16 {
    (cur as i32 + delta).clamp(0, max as i32) as u16
}

/// 列表 y → 项索引（见文件头几何口径；返回 None=点在边框/空白/窗外）。
pub fn list_index_at(rect: Rect, y: u16, scroll: u16, len: usize) -> Option<usize> {
    if len == 0 || y <= rect.y || y >= rect.y.saturating_add(rect.height).saturating_sub(1) {
        return None;
    }
    let inner_h = rect.height.saturating_sub(2) as usize;
    let row = (y - rect.y - 1) as usize;
    if row >= inner_h {
        return None;
    }
    let start = len.saturating_sub(inner_h + scroll as usize);
    let idx = start + row;
    (idx < len).then_some(idx)
}

/// 滑条 x → 档位（n 档端点对齐，四舍五入；clamp 到 [0, n-1]）。
pub fn slider_index_at(rect: Rect, x: u16, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let span = u32::from(rect.width.saturating_sub(1));
    if span == 0 {
        return 0;
    }
    let rel = u32::from(x.saturating_sub(rect.x)).min(span);
    let idx = (rel * (n as u32 - 1) + span / 2) / span;
    (idx as usize).min(n - 1)
}

/// 表单 (x,y) → 槽位（见文件头几何口径）。
pub fn form_slot_at(rect: Rect, x: u16, y: u16) -> FormSlot {
    if y <= rect.y || y >= rect.y.saturating_add(rect.height).saturating_sub(1) {
        return FormSlot::None;
    }
    let row = (y - rect.y - 1) as usize;
    if row < FORM_FIELDS.len() {
        return FormSlot::Field(FORM_FIELDS[row]);
    }
    if row == FORM_FIELDS.len() + 1 {
        let mid = rect.x + rect.width / 2;
        return if x < mid {
            FormSlot::Save
        } else {
            FormSlot::Cancel
        };
    }
    FormSlot::None
}

impl App {
    /// 鼠标事件入口（main 事件循环分发）。返回是否需要重绘。
    pub fn handle_mouse(&mut self, m: MouseEvent) -> bool {
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => self.mouse_down(x, y),
            MouseEventKind::Up(MouseButton::Left) => self.mouse_up(),
            MouseEventKind::Drag(MouseButton::Left) => self.mouse_drag(x, y),
            MouseEventKind::ScrollUp => self.mouse_scroll(x, y, SCROLL_STEP),
            MouseEventKind::ScrollDown => self.mouse_scroll(x, y, -SCROLL_STEP),
            // Moved / 右键 / 中键 / 横向滚轮：暂无语义（登记在 keymap::MOUSE_HELP）。
            _ => false,
        }
    }

    /// 左键按下：按命中区聚焦/选中/开浮层/定档/开表单（双击=开会话）。
    fn mouse_down(&mut self, x: u16, y: u16) -> bool {
        let dbl = is_double(self.mouse.last_down, x, y);
        self.mouse.last_down = Some((x, y, Instant::now()));
        self.mouse.moved = false;

        // 模型浮层（模态）：点模型条=收起，点其它=收起并吞掉本次点击。
        if self.model_popup_open() {
            if self.hit.hit(x, y) == Some(HitTarget::ModelBar) {
                self.dispatch(Action::ModelPopupCancel);
            } else {
                self.dispatch(Action::ModelPopupCancel);
                return true;
            }
            return true;
        }
        // 表单（模态）：只在表单内生效，表外点击吞掉（防误丢已填内容）。
        if self.form_open() {
            if self.hit.hit(x, y) == Some(HitTarget::Form) {
                let slot = self
                    .hit
                    .form
                    .map(|r| form_slot_at(r, x, y))
                    .unwrap_or(FormSlot::None);
                match slot {
                    FormSlot::Field(f) => {
                        if let Mode::Hub(h) = &mut self.mode {
                            if let Some(form) = &mut h.form {
                                form.focus_field(f);
                            }
                        }
                    }
                    FormSlot::Save => self.dispatch(Action::FormSubmit),
                    FormSlot::Cancel => self.dispatch(Action::FormCancel),
                    FormSlot::None => {}
                }
            }
            return true;
        }

        // 斜杠浮层（输入过滤态）：浮层内点击=按行选中并立即执行，
        // 浮层外点击=关浮层并吞掉本次点击（抑制自动重开）。
        if self.slash_open {
            return self.slash_click(x, y);
        }

        match self.hit.hit(x, y) {
            Some(HitTarget::Input) => {
                self.focus = Focus::Input;
                self.selection = None;
                true
            }
            Some(HitTarget::History) => {
                self.focus = Focus::List;
                if dbl {
                    self.dispatch(Action::OpenSelected);
                } else if let Some(idx) = self.history_index_at(y) {
                    if let Mode::Hub(h) = &mut self.mode {
                        h.selected = idx;
                    }
                }
                self.begin_select(HitTarget::History, x, y);
                true
            }
            Some(HitTarget::Chat) => {
                self.begin_select(HitTarget::Chat, x, y);
                true
            }
            Some(HitTarget::ModelBar) => {
                self.dispatch(Action::ModelPopupOpen);
                true
            }
            Some(HitTarget::EffortSlider) => {
                self.slider_set(x);
                self.mouse.drag = Some(DragKind::Slider);
                true
            }
            Some(HitTarget::AddBtn) => {
                self.dispatch(Action::FormOpen);
                true
            }
            Some(HitTarget::Form) => false,  // 未打开的表单不应被命中
            Some(HitTarget::Slash) => false, // 斜杠浮层在 slash_open 分支已消费
            Some(HitTarget::Stats) | None => {
                self.selection = None;
                false
            }
        }
    }

    /// 左键拖动：滑条实时改档 / 拖选扩展选区。
    fn mouse_drag(&mut self, x: u16, y: u16) -> bool {
        match self.mouse.drag {
            Some(DragKind::Slider) => {
                self.slider_set(x);
                true
            }
            Some(DragKind::Select(_)) => {
                self.mouse.moved = true;
                if let Some(anchor) = self.mouse.anchor {
                    self.selection = Some(super::clip::normalize_sel(anchor, (x, y)));
                }
                true
            }
            None => false,
        }
    }

    /// 左键松开：拖选结束 → OSC 52 复制（零选区则清掉高亮）。
    fn mouse_up(&mut self) -> bool {
        let drag = self.mouse.drag.take();
        let moved = self.mouse.moved;
        self.mouse.anchor = None;
        match drag {
            Some(DragKind::Select(region)) if moved => {
                let text = self.selection_text(region);
                if text.is_empty() {
                    self.selection = None;
                    return true;
                }
                super::clip::write_clipboard(&text);
                self.selection = None;
                true
            }
            Some(DragKind::Select(_)) => {
                self.selection = None;
                true
            }
            Some(DragKind::Slider) => true, // 每次改档已同步落盘
            None => false,
        }
    }

    /// 滚轮：按鼠标所在区滚动（历史/对话 = 距底部偏移；浮层 = 移动高亮/选中）。
    fn mouse_scroll(&mut self, x: u16, y: u16, delta: i32) -> bool {
        if self.model_popup_open() {
            if delta > 0 {
                self.dispatch(Action::ModelPopupPrev);
            } else {
                self.dispatch(Action::ModelPopupNext);
            }
            return true;
        }
        // 斜杠浮层内滚轮 = 切换选中（浮层外走所在区滚动）。
        if self.slash_open {
            if let Some(rect) = self.hit.slash {
                if rect.contains((x, y).into()) {
                    self.slash_move(delta > 0);
                    return true;
                }
            }
        }
        match self.hit.hit(x, y) {
            Some(HitTarget::History) => {
                if let Mode::Hub(h) = &mut self.mode {
                    let max = h.sessions.len() as u16;
                    h.scroll = scroll_offset(h.scroll, delta, max);
                    return true;
                }
                false
            }
            Some(HitTarget::Chat) => {
                if let Mode::Worker(w) = &mut self.mode {
                    w.scroll = scroll_offset(w.scroll, delta, 10_000);
                    return true;
                }
                false
            }
            // 表单无滚动内容；其余区/空白：无语义。
            _ => false,
        }
    }

    /// 开始拖选锚点。
    fn begin_select(&mut self, region: HitTarget, x: u16, y: u16) {
        self.mouse.drag = Some(DragKind::Select(region));
        self.mouse.anchor = Some((x, y));
        self.selection = Some(((x, y), (x, y)));
    }

    /// 滑条按 x 定档（改档即经 upsert+save 落盘）。
    fn slider_set(&mut self, x: u16) {
        let Some(rect) = self.hit.effort_slider else {
            return;
        };
        let n = crate::slash::EFFORTS.len();
        self.set_effort_idx(slider_index_at(rect, x, n));
    }

    /// 历史列表 y → 项索引（hub 才有历史）。
    fn history_index_at(&self, y: u16) -> Option<usize> {
        let rect = self.hit.history?;
        let (len, scroll) = match &self.mode {
            Mode::Hub(h) => (h.sessions.len(), h.scroll),
            Mode::Worker(_) => return None,
        };
        list_index_at(rect, y, scroll, len)
    }

    /// 选区 → 文本（按归属区取可见窗口纯文本行后切片）。
    fn selection_text(&self, region: HitTarget) -> String {
        let Some(sel) = self.selection else {
            return String::new();
        };
        match region {
            HitTarget::Chat => {
                let Some(rect) = self.hit.chat else {
                    return String::new();
                };
                let (feed, scroll) = match &self.mode {
                    Mode::Worker(w) => (&w.feed, w.scroll),
                    Mode::Hub(_) => return String::new(),
                };
                let all = super::clip::feed_plain_lines(feed);
                let lines = super::clip::visible_window(&all, scroll, inner_h(rect));
                super::clip::extract_selection(&lines, rect.x + 1, rect.y + 1, sel)
            }
            HitTarget::History => {
                let Some(rect) = self.hit.history else {
                    return String::new();
                };
                let (sessions, scroll) = match &self.mode {
                    Mode::Hub(h) => (&h.sessions, h.scroll),
                    Mode::Worker(_) => return String::new(),
                };
                let all = super::clip::history_plain_lines(sessions);
                let lines = super::clip::visible_window(&all, scroll, inner_h(rect));
                super::clip::extract_selection(&lines, rect.x + 1, rect.y + 1, sel)
            }
            _ => String::new(),
        }
    }
}

/// 块矩形内容高度（扣上下边框）。
fn inner_h(rect: Rect) -> usize {
    rect.height.saturating_sub(2) as usize
}

/// 供 main 的 draw 回调接住 ui::draw 返回值：
/// 渲染路改返回 HitRects 前返回 `()`，两个形态都接得住（Into 兜底）。
impl From<()> for HitRects {
    fn from(_: ()) -> Self {
        HitRects::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::HitRects;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn list_index_maps_rows_inside_border() {
        // 列表块 (0,0,20,6)：内容行 y=1..4，共 4 行。
        let r = rect(0, 0, 20, 6);
        assert_eq!(list_index_at(r, 0, 0, 10), None); // 上边框
        assert_eq!(list_index_at(r, 5, 0, 10), None); // 下边框
        assert_eq!(list_index_at(r, 1, 0, 0), None); // 空列表
        // 2 项不足一窗：行→项 0、1，第 3 行起为空白。
        assert_eq!(list_index_at(r, 1, 0, 2), Some(0));
        assert_eq!(list_index_at(r, 2, 0, 2), Some(1));
        assert_eq!(list_index_at(r, 3, 0, 2), None);
    }

    #[test]
    fn list_index_respects_scroll_tail_window() {
        let r = rect(0, 0, 20, 6); // 窗口 4 行
        // 10 项 scroll=0 → 显示 6..9。
        assert_eq!(list_index_at(r, 1, 0, 10), Some(6));
        // scroll=2 → 显示 4..7。
        assert_eq!(list_index_at(r, 1, 2, 10), Some(4));
        assert_eq!(list_index_at(r, 4, 2, 10), Some(7));
    }

    #[test]
    fn slider_index_endpoints_and_rounding() {
        let r = rect(10, 0, 7, 1); // 7 档端点对齐：x=10→0 … x=16→6
        for (x, want) in [
            (10, 0),
            (11, 1),
            (12, 2),
            (13, 3),
            (14, 4),
            (15, 5),
            (16, 6),
        ] {
            assert_eq!(slider_index_at(r, x, 7), want, "x={x}");
        }
        assert_eq!(slider_index_at(r, 0, 7), 0); // 左侧 clamp
        assert_eq!(slider_index_at(r, 99, 7), 6); // 右侧 clamp
        assert_eq!(slider_index_at(rect(0, 0, 1, 1), 0, 7), 0); // 零宽
    }

    #[test]
    fn form_slots_follow_field_order_and_buttons() {
        // 表单块 (0,0,30,9)：内容行 y=1..7；行0..4=字段，行5=留白，行6=按钮。
        let r = rect(0, 0, 30, 9);
        assert_eq!(form_slot_at(r, 5, 0), FormSlot::None); // 上边框
        assert_eq!(form_slot_at(r, 5, 1), FormSlot::Field(FormField::Label));
        assert_eq!(form_slot_at(r, 5, 5), FormSlot::Field(FormField::Effort));
        assert_eq!(form_slot_at(r, 5, 6), FormSlot::None); // 留白行
        assert_eq!(form_slot_at(r, 5, 7), FormSlot::Save); // 按钮行左半
        assert_eq!(form_slot_at(r, 25, 7), FormSlot::Cancel); // 按钮行右半
        assert_eq!(form_slot_at(r, 5, 8), FormSlot::None); // 下边框
    }

    #[test]
    fn hit_rects_consumption_priority() {
        // 重叠矩形按契约优先级：表单 > + > 滑条 > 模型条 > 输入 > 历史 > 对话 > 统计。
        let mut h = HitRects::default();
        h.stats = Some(rect(0, 0, 20, 20));
        h.chat = Some(rect(0, 0, 20, 20));
        h.history = Some(rect(0, 0, 20, 20));
        h.input = Some(rect(0, 0, 20, 20));
        h.model_bar = Some(rect(0, 0, 20, 20));
        h.effort_slider = Some(rect(0, 0, 20, 20));
        h.add_btn = Some(rect(0, 0, 20, 20));
        h.form = Some(rect(0, 0, 20, 20));
        assert_eq!(h.hit(5, 5), Some(HitTarget::Form));
        h.form = None;
        assert_eq!(h.hit(5, 5), Some(HitTarget::AddBtn));
        h.add_btn = None;
        assert_eq!(h.hit(5, 5), Some(HitTarget::EffortSlider));
        h.effort_slider = None;
        assert_eq!(h.hit(5, 5), Some(HitTarget::ModelBar));
        h.model_bar = None;
        assert_eq!(h.hit(5, 5), Some(HitTarget::Input));
        h.input = None;
        assert_eq!(h.hit(5, 5), Some(HitTarget::History));
        h.history = None;
        assert_eq!(h.hit(5, 5), Some(HitTarget::Chat));
        h.chat = None;
        assert_eq!(h.hit(5, 5), Some(HitTarget::Stats));
        h.stats = None;
        assert_eq!(h.hit(5, 5), None);
    }

    #[test]
    fn double_click_window_and_cell() {
        let now = Instant::now();
        assert!(is_double(Some((3, 4, now)), 3, 4));
        assert!(!is_double(Some((3, 4, now)), 3, 5)); // 换格不算
        assert!(!is_double(Some((3, 4, now - Duration::from_secs(1))), 3, 4)); // 超窗
        assert!(!is_double(None, 3, 4));
    }

    #[test]
    fn scroll_offset_semantics() {
        assert_eq!(scroll_offset(0, SCROLL_STEP, 100), 3);
        assert_eq!(scroll_offset(1, -SCROLL_STEP, 100), 0);
        assert_eq!(scroll_offset(99, SCROLL_STEP, 100), 100);
    }
}
