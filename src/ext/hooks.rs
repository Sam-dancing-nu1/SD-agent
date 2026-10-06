//! 生命周期钩子扩展位：挂接既有生命周期收敛点的接口立桩。
//!
//! 【挂载位】`event::hooks::emit_hook` 是全部生命周期通知的唯一收敛点（硬约束 14），
//! 本模块的 [`HookRegistry::dispatch`] 是其未来的扇出接点：收敛点落盘后把钩子
//! 事件派发给扩展回调、把收紧性裁决回馈给决策链。当前只立桩、未接线
//! （接线属后续开发，见 docs/ext-design.md 路线图）。
//!
//! 【裁决语义（已定）】扩展回调只许收紧不许放松：返回值三态
//! （放行 / 转人审 / 阻断），任何一档都不能放行掉既有策略裁决——
//! 这是第一原则在钩子面的表达（放松属改规则，须走人审动作位）。
//!
//! 【同步约束】同步只允许存在于决策链（硬约束 14）：观察类扇出的异步化
//! 属接线期工作，本立桩的同步分发只服务决策链内的收紧性钩子。

use crate::event::hooks::LifecycleHook;

/// 钩子回调返回值（三态；严格序：放行 < 转人审 < 阻断，聚合取最严者）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HookVerdict {
    /// 放行：本次不收紧（不代表越过既有裁决口）。
    Proceed,
    /// 转人审：命中既有裁决往返，交用户裁决（回调无权代答）。
    RequireApproval,
    /// 阻断：直接拒绝执行（收紧的最强档）。
    Block,
}

/// 钩子事件（回调入参；字段按需扩展、只增不删）。
#[derive(Debug, Clone)]
pub struct HookEvent {
    /// 生命周期点位（复用既有收敛点的点位枚举）。
    pub point: LifecycleHook,
    /// 附注文本（与收敛点落盘记录同源）。
    pub note: String,
}

/// 钩子回调签名：观察 + 收紧性裁决。
///
/// 回调约定零副作用：需要改变任何东西时走动作位声明 + 人审闸口
/// （见 [`crate::ext::permissions`]），不在回调里偷偷动手。
pub type HookCallback = Box<dyn Fn(&HookEvent) -> HookVerdict + Send + Sync>;

/// 钩子注册项（扩展名 + 挂载点位 + 回调；闭包不可比，不派生调试输出）。
pub struct HookEntry {
    /// 所属扩展名。
    pub ext: String,
    /// 挂载点位。
    pub point: LifecycleHook,
    cb: HookCallback,
}

impl HookEntry {
    /// 调用回调（分发器内部使用）。
    pub fn call(&self, event: &HookEvent) -> HookVerdict {
        (self.cb)(event)
    }
}

/// 钩子注册面错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookError {
    /// 同扩展同点位重复注册（禁止隐式覆盖，改回调须先注销）。
    DuplicateRegistration(String),
    /// 扩展名为空。
    EmptyName,
}

/// 钩子注册表：允许为空。
///
/// 空表 = 未挂任何扩展，分发放行——不启用扩展时行为与本立桩出现前一致。
#[derive(Default)]
pub struct HookRegistry {
    entries: Vec<HookEntry>,
}

impl HookRegistry {
    /// 新建空注册表。
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// 注册回调：同扩展同点位只许一个，重复注册拒绝。
    pub fn register(
        &mut self,
        ext: impl Into<String>,
        point: LifecycleHook,
        cb: HookCallback,
    ) -> Result<(), HookError> {
        let ext = ext.into();
        if ext.trim().is_empty() {
            return Err(HookError::EmptyName);
        }
        if self
            .entries
            .iter()
            .any(|e| e.ext == ext && e.point == point)
        {
            return Err(HookError::DuplicateRegistration(ext));
        }
        self.entries.push(HookEntry { ext, point, cb });
        Ok(())
    }

    /// 注销指定扩展在指定点位的回调，返回是否确有移除。
    pub fn unregister(&mut self, ext: &str, point: LifecycleHook) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| !(e.ext == ext && e.point == point));
        self.entries.len() != before
    }

    /// 分发：按点位过滤回调并取全部裁决的最严者；空表或无命中返回放行。
    pub fn dispatch(&self, event: &HookEvent) -> HookVerdict {
        self.entries
            .iter()
            .filter(|e| e.point == event.point)
            .map(|e| e.call(event))
            .max()
            .unwrap_or(HookVerdict::Proceed)
    }

    /// 注册项数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空（无任何扩展挂载）。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(point: LifecycleHook) -> HookEvent {
        HookEvent {
            point,
            note: "n".to_string(),
        }
    }

    #[test]
    fn empty_registry_proceeds_unchanged() {
        // 未挂扩展时行为不变：放行。
        let reg = HookRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(
            reg.dispatch(&event(LifecycleHook::ToolBefore)),
            HookVerdict::Proceed
        );
    }

    #[test]
    fn dispatch_takes_strictest_verdict() {
        let mut reg = HookRegistry::new();
        reg.register(
            "obs",
            LifecycleHook::ToolBefore,
            Box::new(|_| HookVerdict::Proceed),
        )
        .unwrap();
        reg.register(
            "ask",
            LifecycleHook::ToolBefore,
            Box::new(|_| HookVerdict::RequireApproval),
        )
        .unwrap();
        reg.register(
            "guard",
            LifecycleHook::ToolBefore,
            Box::new(|_| HookVerdict::Block),
        )
        .unwrap();
        // 三态聚合取最严（阻断 > 转人审 > 放行）。
        assert_eq!(
            reg.dispatch(&event(LifecycleHook::ToolBefore)),
            HookVerdict::Block
        );
        // 其它点位不受影响。
        assert_eq!(
            reg.dispatch(&event(LifecycleHook::RunEnd)),
            HookVerdict::Proceed
        );
    }

    #[test]
    fn register_unregister_and_duplicate_rejected() {
        let mut reg = HookRegistry::new();
        reg.register(
            "a",
            LifecycleHook::RunStart,
            Box::new(|_| HookVerdict::Proceed),
        )
        .unwrap();
        assert_eq!(reg.len(), 1);
        // 同扩展同点位重复注册拒绝（禁止隐式覆盖）。
        assert_eq!(
            reg.register(
                "a",
                LifecycleHook::RunStart,
                Box::new(|_| HookVerdict::Block)
            ),
            Err(HookError::DuplicateRegistration("a".to_string()))
        );
        // 空名拒绝。
        assert_eq!(
            reg.register(
                "  ",
                LifecycleHook::RunStart,
                Box::new(|_| HookVerdict::Proceed)
            ),
            Err(HookError::EmptyName)
        );
        // 注销后可重新注册；注销不存在的项返回 false。
        assert!(reg.unregister("a", LifecycleHook::RunStart));
        assert!(!reg.unregister("a", LifecycleHook::RunStart));
        assert!(reg.is_empty());
    }
}
