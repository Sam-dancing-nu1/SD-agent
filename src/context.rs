//! 上下文组装（审查 #1：独立落点，P2 四段结构只填实现不动依赖图）。
//!
//! 硬约束 15：基础前缀会话内冻结、不重写，一切变化以"追加"表达——
//! `static_prefix()` 一次构建、逐轮原样携带（字节不变，前缀缓存友好）；
//! `dynamic_appendix()` 每轮重建的增量附录，以独立段标记接在前缀之后。
//! 硬约束 17：目标锚写入冻结前缀，永不改写、永不需重注入。
//!
//! 段标记：`---[static]---` / `---[appendix]---`，读端（人/模型/审计）可辨识。
//!【口径】P0 的附录与前缀同处 system 文本（成本工程 P2 优化到位前的形态），
//! 追加段在后、前缀段字节不变，语义上仍符合"追加表达"。

use crate::model::ChatMessage;

/// 冻结前缀：角色与纪律 + 服从契约 + 工具契约 + 目标锚（结构化、一次写入）。
pub fn static_prefix(task: &str) -> String {
    let mut s = String::new();
    s.push_str("---[static]---\n");
    s.push_str("You are sd-agent, a careful engineering agent working inside a version-controlled workspace.\n");
    s.push_str("Discipline: prefer minimal changes; verify before claiming done; never fabricate results.\n");
    s.push_str("Tools are executed through a policy gate: dangerous commands and out-of-workspace paths are denied by rule tables.\n");
    s.push_str("All file paths you pass to tools must be workspace-relative.\n");
    s.push_str("\n---[obedience-contract]---\n");
    s.push_str("These are hard rules, not style suggestions. Breaking any of them is a failure.\n");
    s.push_str("1. Execute exactly what the task description says. Do only the work the description asks for: no extra features, no unsolicited refactoring, no renaming, no 'improvements', no scope expansion of any kind.\n");
    s.push_str("2. Never substitute your own plan for the task description. If your idea conflicts with the description, follow the description.\n");
    s.push_str("3. When the task description is ambiguous, incomplete, or contradictory, STOP and ask the user in your reply. Do not guess, do not improvise a plausible interpretation, do not proceed on assumption.\n");
    s.push_str("4. Define done by the task description alone: walk through it item by item and verify each item against its literal wording. Only report done when every item is verified. Partial work is reported as partial.\n");
    s.push_str("5. Stay inside the described scope even when you notice adjacent problems; mention them in your reply instead of silently fixing them.\n");
    s.push_str("\n---[goal-anchor]---\n");
    s.push_str(&format!("Task (frozen at session start): {task}\n"));
    s.push_str("This anchor is written once and never rewritten; treat it as the single source of the goal.\n");
    s
}

/// 增量附录：每轮重建的可变段（轮数记账 + 提醒）。固定节结构，重注入零缓存代价
///（P2 起此段挪入独立增量通道）。
pub fn dynamic_appendix(round: u32, max_rounds: u32) -> String {
    format!(
        "---[appendix]---\nround: {round}/{max_rounds}\nWhen the task is fully done and verified, reply with the final summary and do not call tools.\n"
    )
}

/// 上下文组装器：冻结前缀持有 + 每轮拼接请求消息。
pub struct ContextBuilder {
    frozen_prefix: String,
    max_rounds: u32,
}

impl ContextBuilder {
    /// 构造即冻结前缀（会话内唯一一次构建）。
    pub fn new(task: &str, max_rounds: u32) -> Self {
        Self {
            frozen_prefix: static_prefix(task),
            max_rounds,
        }
    }

    /// 冻结前缀（审计/测试可读）。
    pub fn frozen_prefix(&self) -> &str {
        &self.frozen_prefix
    }

    /// 组装一次模型请求：system（冻结前缀 + 增量附录）+ 对话历史。
    /// 历史只追加不改写（硬约束 15）。
    pub fn build(&self, history: &[ChatMessage], round: u32) -> Vec<ChatMessage> {
        let mut messages = Vec::with_capacity(history.len() + 1);
        let system = format!(
            "{}{}",
            self.frozen_prefix,
            dynamic_appendix(round, self.max_rounds)
        );
        messages.push(ChatMessage::system(system));
        messages.extend_from_slice(history);
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_prefix_is_byte_stable_across_rounds() {
        let cb = ContextBuilder::new("demo task", 20);
        let p1 = cb.frozen_prefix().to_string();
        let _ = cb.build(&[], 3);
        assert_eq!(p1, cb.frozen_prefix());
        assert!(p1.contains("---[static]---"));
        assert!(p1.contains("---[goal-anchor]---"));
        assert!(p1.contains("demo task"));
    }

    #[test]
    fn prefix_carries_hard_obedience_contract() {
        // 服从契约必须进冻结前缀（用户痛点：给提示都不做、按模型自己思路来）。
        let p = static_prefix("t");
        assert!(p.contains("---[obedience-contract]---"));
        assert!(p.contains("Do only the work the description asks for"));
        assert!(
            p.contains("ask the user in your reply"),
            "不明确必须提问不许猜"
        );
        assert!(p.contains("item by item"), "完成判定必须逐条对照任务描述");
    }

    #[test]
    fn appendix_changes_per_round_but_prefix_not() {
        let cb = ContextBuilder::new("t", 20);
        let m1 = cb.build(&[], 1);
        let m2 = cb.build(&[], 2);
        let s1 = m1[0].content.clone();
        let s2 = m2[0].content.clone();
        assert_ne!(s1, s2, "appendix should differ per round");
        assert!(
            s1.starts_with(cb.frozen_prefix()),
            "prefix must lead unchanged"
        );
        assert!(s2.starts_with(cb.frozen_prefix()));
    }
}
