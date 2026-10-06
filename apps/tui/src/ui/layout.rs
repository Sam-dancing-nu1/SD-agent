//! 渲染层 → 交互层的布局登记（契约文件：结构冻结，双方只读引用）。
//!
//! 每帧渲染产出 HitRects（ui::draw 返回），交互层（鼠标命中检测）按
//! 组件 Rect 判定点击归属。渲染层只写不读，交互层只读不写。

use ratatui::layout::Rect;

/// 可交互组件的屏幕区域（None=该帧未渲染该组件）。
#[derive(Debug, Clone, Default)]
pub struct HitRects {
    /// 初始态大对话框 / WORK 态顶栏输入框。
    pub input: Option<Rect>,
    /// 历史会话列表。
    pub history: Option<Rect>,
    /// Token 统计面板。
    pub stats: Option<Rect>,
    /// 模型选择器（初始态对话框上方/内）。
    pub model_bar: Option<Rect>,
    /// 思考强度滑条。
    pub effort_slider: Option<Rect>,
    /// "+" 添加配置按钮。
    pub add_btn: Option<Rect>,
    /// 大 Logo 区。
    pub logo: Option<Rect>,
    /// worker 对话流区。
    pub chat: Option<Rect>,
    /// 配置表单弹窗。
    pub form: Option<Rect>,
    /// 斜杠命令浮层（行序=命令表顺序，点击按 y 定位行）。
    pub slash: Option<Rect>,
    /// 底部历史面板（初始态图1 布局用）。
    pub history_panel: Option<Rect>,
    /// 底部 token 记录面板（初始态图1 布局用）。
    pub token_panel: Option<Rect>,
}

impl HitRects {
    /// 命中检测：返回点 (x, y) 落入的组件名（按优先级：弹窗 > 表单控件 >
    /// 输入 > 模型条 > 滑条 > 按钮 > 列表 > 统计 > 对话）。
    pub fn hit(&self, x: u16, y: u16) -> Option<HitTarget> {
        let in_rect = |r: &Option<Rect>| r.map(|r| r.contains((x, y).into())) == Some(true);
        if in_rect(&self.form) {
            return Some(HitTarget::Form);
        }
        if in_rect(&self.slash) {
            return Some(HitTarget::Slash);
        }
        if in_rect(&self.add_btn) {
            return Some(HitTarget::AddBtn);
        }
        if in_rect(&self.effort_slider) {
            return Some(HitTarget::EffortSlider);
        }
        if in_rect(&self.model_bar) {
            return Some(HitTarget::ModelBar);
        }
        if in_rect(&self.input) {
            return Some(HitTarget::Input);
        }
        if in_rect(&self.history) {
            return Some(HitTarget::History);
        }
        if in_rect(&self.chat) {
            return Some(HitTarget::Chat);
        }
        if in_rect(&self.stats) {
            return Some(HitTarget::Stats);
        }
        None
    }
}

/// 命中目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTarget {
    Slash,
    Input,
    History,
    Stats,
    ModelBar,
    EffortSlider,
    AddBtn,
    Chat,
    Form,
}
