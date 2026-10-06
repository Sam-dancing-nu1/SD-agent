//! 扩展权限声明 + 人审闸口（扩展点安全边界的第一原则落点）。
//!
//! 【第一原则（已定，焊死）】任何自修改类动作——修改自身或宿主的代码、配置、
//! 提示词、规则——必须经既有裁决口（policy 层 ApprovalPort）人审，用户同意才
//! 执行；程序与智能体零自我批准权。"Agent 给自己做手术"允许存在，但手术同意权
//! 永远在人手里。
//!
//! 【三步运行模型（已定）】扩展动作一律走：声明（[`crate::ext::ExtensionDecl`]
//! 登记将要做的动作）→ 人审（本模块闸口）→ 执行。本模块不自造安全机制，
//! 只固化"什么动作必须问人"的判定与闸口签名；人审回调对接既有裁决往返。

/// 自修改目标：修改"自身"的哪一类资产。全部目标一律人审，无例外。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfModifyTarget {
    /// 代码（自身或宿主的实现代码）。
    Code,
    /// 配置（运行配置与环境约定）。
    Config,
    /// 提示词（系统提示、上下文冻结前缀）。
    Prompt,
    /// 规则（裁决规则表、纪律条款）。
    Rules,
}

impl SelfModifyTarget {
    /// 目标标识（稳定字符串，进声明记录与事件留痕）。
    pub fn as_str(&self) -> &'static str {
        match self {
            SelfModifyTarget::Code => "code",
            SelfModifyTarget::Config => "config",
            SelfModifyTarget::Prompt => "prompt",
            SelfModifyTarget::Rules => "rules",
        }
    }
}

/// 扩展动作类型：权限声明与人审判定共用同一枚举（枚举只增不删；
/// 新增变体必须在 [`needs_human_approval`] 显式归类，穷尽匹配保证不漏判）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtAction {
    /// 观察事件流（只读订阅，零副作用）。
    ObserveEvents,
    /// 生命周期钩子回调（返回值只许收紧不许放松，见 [`crate::ext::hooks`]）。
    HookCallback,
    /// 上下文附录位追加条目（只追加、全量留痕、不碰冻结前缀）。
    InjectContextAppendix,
    /// 收紧审批策略（扩大拒绝面/缩小放行面）。安全增强可直接挂，免人审。
    TightenPolicy,
    /// 放松审批策略（缩小拒绝面/扩大放行面）。改裁决规则，一律人审。
    ///（与 [`SelfModifyTarget::Rules`] 同等人审强度；单列成变体是为把
    /// "只许收紧不许放松"的边界写进类型本身。）
    RelaxPolicy,
    /// 注册新工具（扩张模型能力面），一律人审。
    RegisterTool,
    /// 自修改（改自身/宿主的代码、配置、提示词、规则），一律人审（第一原则）。
    SelfModify(SelfModifyTarget),
}

/// 人审判定（第一原则的函数落点）：返回该动作是否必须经人审才能执行。
///
/// 触发线（已定）：
/// - 自修改类（[`ExtAction::SelfModify`] 全部目标）→ 一律 `true`，无例外；
/// - 放松裁决（[`ExtAction::RelaxPolicy`]）、扩张能力面（[`ExtAction::RegisterTool`]）
///   → `true`；
/// - 纯观察（`ObserveEvents` / `HookCallback`）、纯收紧（`TightenPolicy`）、
///   附录位只追加注入（`InjectContextAppendix`）→ `false`（全量留痕照旧）。
pub fn needs_human_approval(action: &ExtAction) -> bool {
    use ExtAction::*;
    match action {
        ObserveEvents | HookCallback | InjectContextAppendix | TightenPolicy => false,
        RelaxPolicy | RegisterTool | SelfModify(_) => true,
    }
}

/// 人审闸口：命中人审的动作必须经 `ask` 回调取得用户同意才放行；
/// 免人审动作直接放行（不打扰用户）。
///
/// 【零自我批准权】`ask` 的实现必须是真人裁决往返（对接既有裁决口的阻塞式
/// 询问），程序与智能体不得代答、不得缓存历史同意、不得以任何自动批准端口
/// 替代本处的人审——自动放行视同零批准。返回 `true` = 用户同意执行。
pub fn human_gate<F: FnOnce() -> bool>(action: &ExtAction, ask: F) -> bool {
    if needs_human_approval(action) {
        ask()
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全部自修改目标的枚举清单（新增目标时补进此清单，测试即覆盖）。
    fn all_self_modify_targets() -> [SelfModifyTarget; 4] {
        [
            SelfModifyTarget::Code,
            SelfModifyTarget::Config,
            SelfModifyTarget::Prompt,
            SelfModifyTarget::Rules,
        ]
    }

    #[test]
    fn self_modify_always_needs_human_approval() {
        // 第一原则：自修改类一律人审——逐目标断言，一个都不能漏。
        for target in all_self_modify_targets() {
            let action = ExtAction::SelfModify(target);
            assert!(
                needs_human_approval(&action),
                "自修改目标 {} 必须命中人审",
                target.as_str()
            );
        }
    }

    #[test]
    fn relaxation_and_capability_expansion_need_approval() {
        assert!(needs_human_approval(&ExtAction::RelaxPolicy));
        assert!(needs_human_approval(&ExtAction::RegisterTool));
    }

    #[test]
    fn observation_and_tightening_skip_approval() {
        // 收紧属安全增强可直接挂（免人审）；纯观察/附录注入同样免人审。
        assert!(!needs_human_approval(&ExtAction::ObserveEvents));
        assert!(!needs_human_approval(&ExtAction::HookCallback));
        assert!(!needs_human_approval(&ExtAction::TightenPolicy));
        assert!(!needs_human_approval(&ExtAction::InjectContextAppendix));
    }

    #[test]
    fn gate_denies_self_modify_without_human_consent() {
        // 零自我批准权：用户不同意 → 闸口拒绝，自修改不得执行。
        for target in all_self_modify_targets() {
            let action = ExtAction::SelfModify(target);
            assert!(!human_gate(&action, || false), "无人同意时必须拒绝");
        }
        assert!(!human_gate(&ExtAction::RelaxPolicy, || false));
    }

    #[test]
    fn gate_passes_free_actions_without_asking() {
        // 免人审动作不应触发询问（ask 被调用即 panic 暴露）。
        assert!(human_gate(&ExtAction::TightenPolicy, || panic!("不该问人")));
        assert!(human_gate(&ExtAction::ObserveEvents, || panic!("不该问人")));
        // 命中人审的动作，用户同意才放行。
        assert!(human_gate(
            &ExtAction::SelfModify(SelfModifyTarget::Code),
            || true
        ));
    }
}
