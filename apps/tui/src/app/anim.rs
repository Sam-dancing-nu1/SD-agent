//! 两态动画状态机（契约文件：结构冻结，双方只读引用）。
//!
//! hub 初始态（大 Logo+对话框）↔ WORK 态（顶栏+历史+统计）的过渡动画。
//! 渲染层按 progress() 取 0..1 插值系数混合两态布局；交互层触发 start()。
//! 终端无 alpha，动画形态=布局滑动（Logo 区高度插值 + 内容区自底部推入）。

use std::time::{Duration, Instant};

/// 动画时长（300ms，够顺又不拖沓）。
pub const ANIM_DURATION: Duration = Duration::from_millis(300);

/// 动画方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimKind {
    /// 初始态 → WORK 态（Logo+输入框上移、历史/统计推入）。
    ToWork,
    /// WORK 态 → 初始态（反向）。
    ToIdle,
}

/// 动画状态。
#[derive(Debug, Default)]
pub struct Anim {
    kind: Option<AnimKind>,
    started: Option<Instant>,
}

impl Anim {
    /// 触发动画（重复触发同方向不重启；反向触发从当前进度反走）。
    pub fn start(&mut self, kind: AnimKind) {
        if self.kind == Some(kind) {
            return;
        }
        self.kind = Some(kind);
        self.started = Some(Instant::now());
    }

    /// 当前动画方向（无动画时 None）。
    pub fn kind(&self) -> Option<AnimKind> {
        self.kind
    }

    /// 进度 0..1（ease-out；无动画或已结束返回 1.0）。
    pub fn progress(&self) -> f32 {
        match self.started {
            Some(t) => {
                let p = t.elapsed().as_secs_f32() / ANIM_DURATION.as_secs_f32();
                if p >= 1.0 {
                    1.0
                } else {
                    // ease-out quad：先快后慢，观感顺。
                    1.0 - (1.0 - p) * (1.0 - p)
                }
            }
            None => 1.0,
        }
    }

    /// 动画是否已结束（结束即定格，渲染层可停帧）。
    pub fn done(&self) -> bool {
        self.started
            .map(|t| t.elapsed() >= ANIM_DURATION)
            .unwrap_or(true)
    }

    /// 动画落点是否 WORK 态（含已结束定格）。
    pub fn heading_to_work(&self) -> bool {
        self.kind == Some(AnimKind::ToWork)
    }
}
