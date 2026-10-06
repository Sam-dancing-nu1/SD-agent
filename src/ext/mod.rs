//! 扩展点 / SDK 空间预留：插件注册面单入口 + 扩展声明结构（架构空间立桩）。
//!
//! 【定位】为"解耦插件式、允许对运行时做手术刀式扩展、乃至智能体给自己做手术"
//! 预留架构空间。本次交付只落注册面骨架与安全边界，不是完整插件系统——
//! 扩展加载器、沙箱隔离、热替换均属后续开发（见 docs/ext-design.md 路线图）。
//!
//! 【第一原则（已定，焊死）】任何自修改类动作（改自身/宿主的代码、配置、
//! 提示词、规则）必须经既有裁决口人审，用户同意才执行；程序与智能体
//! 零自我批准权。判定与闸口见 [`permissions`]。
//!
//! 【依赖方向】本模块是核心 lib 的顶层新模块，只依赖 event 层类型
//! （生命周期点位）与标准库，不依赖工具/策略/模型层；扩展挂接在既有四个
//! 接口（事件提交/事件回放/裁决往返/模型客户端）的外圈，不改核心契约。

pub mod hooks;
pub mod permissions;

pub use hooks::{HookCallback, HookEntry, HookError, HookEvent, HookRegistry, HookVerdict};
pub use permissions::{ExtAction, SelfModifyTarget, human_gate, needs_human_approval};

use std::fmt;

/// 扩展面位：一个扩展可下刀的注册槽（手术刀下刀处清单；枚举只增不删）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtSurface {
    /// 工具注册扩展位：为智能体新增工具面（能力扩张，执行前人审）。
    Tool,
    /// 生命周期钩子扩展位：挂接生命周期收敛点的观察/收紧回调。
    Hook,
    /// 上下文注入扩展位：向上下文增量附录段追加条目（只追加、不碰冻结前缀）。
    Context,
    /// 系统提示补丁位：修改系统提示增量段（属自修改类，一律人审）。
    PromptPatch,
    /// 界面命令注册位：注册界面面板或命令入口（壳层职责）。
    UiCommand,
    /// 审批策略扩展位：扩展裁决规则——只许收紧免人审，放松一律人审。
    Policy,
    /// 自修改动作位：修改自身/宿主的代码、配置、提示词、规则（一律人审）。
    SelfModify,
}

impl ExtSurface {
    /// 面位标识（稳定字符串，进声明记录与事件留痕）。
    pub fn as_str(&self) -> &'static str {
        match self {
            ExtSurface::Tool => "tool",
            ExtSurface::Hook => "hook",
            ExtSurface::Context => "context",
            ExtSurface::PromptPatch => "prompt_patch",
            ExtSurface::UiCommand => "ui_command",
            ExtSurface::Policy => "policy",
            ExtSurface::SelfModify => "self_modify",
        }
    }
}

/// 扩展声明：声明式描述 + 所需权限声明（回调另行注册，见 [`hooks`]）。
///
/// 三步运行模型的第一步"声明"：扩展在这里登记将要动的面位与动作，
/// 后续每步执行前逐动作过人审闸口（[`needs_human_approval`] / [`human_gate`]）。
#[derive(Debug, Clone)]
pub struct ExtensionDecl {
    /// 扩展名（注册表内唯一，非空）。
    pub name: String,
    /// 版本串（仅登记留痕，暂不做版本协商）。
    pub version: String,
    /// 申请的扩展面位。
    pub surfaces: Vec<ExtSurface>,
    /// 所需权限声明：扩展将要执行的动作类型。
    pub actions: Vec<ExtAction>,
}

impl ExtensionDecl {
    /// 新建声明（默认无面位、无动作，用 with_* 逐项登记）。
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            surfaces: Vec::new(),
            actions: Vec::new(),
        }
    }

    /// 登记一个扩展面位。
    pub fn with_surface(mut self, surface: ExtSurface) -> Self {
        self.surfaces.push(surface);
        self
    }

    /// 登记一项所需权限（动作类型）。
    pub fn with_action(mut self, action: ExtAction) -> Self {
        self.actions.push(action);
        self
    }

    /// 声明的动作里是否含必须人审的项（注册前自检提示用，不替代执行期闸口）。
    pub fn requires_human_approval(&self) -> bool {
        self.actions.iter().any(needs_human_approval)
    }
}

/// 注册面错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtError {
    /// 扩展名重复（注册表内唯一）。
    DuplicateName(String),
    /// 扩展名为空。
    EmptyName,
}

impl fmt::Display for ExtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtError::DuplicateName(n) => write!(f, "扩展名重复: {n}"),
            ExtError::EmptyName => write!(f, "扩展名为空"),
        }
    }
}

impl std::error::Error for ExtError {}

/// 扩展注册表：注册面单入口（声明的增删查）。
///
/// 回调注册在 [`hooks::HookRegistry`]，权限判定在 [`permissions`]——
/// 本结构只管"谁声明了什么"，是扩展的花名册。
#[derive(Debug, Default)]
pub struct ExtensionRegistry {
    decls: Vec<ExtensionDecl>,
}

impl ExtensionRegistry {
    /// 新建空注册表。
    pub fn new() -> Self {
        Self { decls: Vec::new() }
    }

    /// 注册扩展声明：空名、重名拒绝（重名须先注销，禁止隐式覆盖）。
    pub fn register(&mut self, decl: ExtensionDecl) -> Result<(), ExtError> {
        if decl.name.trim().is_empty() {
            return Err(ExtError::EmptyName);
        }
        if self.decls.iter().any(|d| d.name == decl.name) {
            return Err(ExtError::DuplicateName(decl.name));
        }
        self.decls.push(decl);
        Ok(())
    }

    /// 注销扩展，返回被注销的声明（不存在返回 None）。
    pub fn unregister(&mut self, name: &str) -> Option<ExtensionDecl> {
        let idx = self.decls.iter().position(|d| d.name == name)?;
        Some(self.decls.remove(idx))
    }

    /// 按名查询声明。
    pub fn get(&self, name: &str) -> Option<&ExtensionDecl> {
        self.decls.iter().find(|d| d.name == name)
    }

    /// 全量声明清单（注册顺序）。
    pub fn list(&self) -> &[ExtensionDecl] {
        &self.decls
    }

    /// 声明数。
    pub fn len(&self) -> usize {
        self.decls.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.decls.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_register_get_list_unregister() {
        // 增删查全链路。
        let mut reg = ExtensionRegistry::new();
        assert!(reg.is_empty());
        reg.register(
            ExtensionDecl::new("a", "0.1.0")
                .with_surface(ExtSurface::Hook)
                .with_action(ExtAction::ObserveEvents),
        )
        .unwrap();
        reg.register(ExtensionDecl::new("b", "0.1.0")).unwrap();
        assert_eq!(reg.len(), 2);
        // 查：按名取回，面位与动作随声明保留。
        let a = reg.get("a").expect("a 应在册");
        assert_eq!(a.surfaces, vec![ExtSurface::Hook]);
        assert_eq!(a.actions, vec![ExtAction::ObserveEvents]);
        assert!(reg.get("missing").is_none());
        // 列表保持注册顺序。
        assert_eq!(
            reg.list()
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        // 删：注销返回声明本体，再删返回 None。
        let removed = reg.unregister("a").expect("a 应可注销");
        assert_eq!(removed.name, "a");
        assert!(reg.unregister("a").is_none());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn registry_rejects_empty_and_duplicate_names() {
        let mut reg = ExtensionRegistry::new();
        assert_eq!(
            reg.register(ExtensionDecl::new("  ", "0.1.0")),
            Err(ExtError::EmptyName)
        );
        reg.register(ExtensionDecl::new("a", "0.1.0")).unwrap();
        // 重名拒绝：禁止隐式覆盖既有声明。
        assert_eq!(
            reg.register(ExtensionDecl::new("a", "0.2.0")),
            Err(ExtError::DuplicateName("a".to_string()))
        );
    }

    #[test]
    fn decl_reports_human_approval_need() {
        // 声明自检：含自修改动作的声明必须亮人审旗。
        let self_mod = ExtensionDecl::new("surgeon", "0.1.0")
            .with_action(ExtAction::SelfModify(SelfModifyTarget::Code));
        assert!(self_mod.requires_human_approval());
        let observer = ExtensionDecl::new("watcher", "0.1.0")
            .with_action(ExtAction::ObserveEvents)
            .with_action(ExtAction::TightenPolicy);
        assert!(!observer.requires_human_approval());
    }
}
