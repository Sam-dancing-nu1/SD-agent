//! SessionTap：壳层会话桥（核心 SessionStore 的薄封装）。
//!
//! 职责：run 线程把任务（user 消息）与模型回复（assistant 消息）持续 append
//! 进会话；恢复会话续跑时把历史转成模型消息形态交给 run_task
//! （AgentConfig.history）。持久化失败只记日志降级，不打断 run——
//! 诊断/持久化通道自身不允许制造执行故障，失败原因落 desktop.log。
//!
//! 会话 ↔ 模型消息形态的转换是壳层职责（session 层零业务依赖）：
//! role 只映射 "user"/"assistant"，"tool" 等角色无法还原 tool_call_id，
//! 一律跳过（不编造上下文）。

use std::path::Path;

use sd_agent::event::now_unix_ms;
use sd_agent::model::ChatMessage;
use sd_agent::session::{SessionMessage, SessionStore};

use crate::logging;

/// 一个会话的读写通道（工作区根 + 会话 id）。
pub struct SessionTap {
    store: SessionStore,
    id: String,
}

impl SessionTap {
    /// 绑定工作区里的某个会话（会话是否已存在由调用方校验）。
    pub fn new(workspace_root: &Path, id: impl Into<String>) -> Self {
        Self {
            store: SessionStore::new(workspace_root),
            id: id.into(),
        }
    }

    /// 追加一条任务（user 消息）。
    pub fn append_user(&self, text: &str) {
        self.append("user", text);
    }

    /// 追加一条模型回复（assistant 消息）。
    pub fn append_assistant(&self, text: &str) {
        self.append("assistant", text);
    }

    /// 追加一条消息（失败只记日志，不打断 run）。
    fn append(&self, role: &str, text: &str) {
        // SessionMessage::new 天然带 reasoning_content=None（user/tool 无思维链）。
        let msg = SessionMessage::new(role, text, now_unix_ms());
        if let Err(e) = self.store.append(&self.id, msg) {
            logging::log(format!(
                "会话写入失败（session={}，role={}）：{e}",
                self.id, role
            ));
        }
    }

    /// 会话历史 → 模型消息形态（恢复会话续跑的上下文）。
    /// 读取失败或空会话返回空历史（降级为不带上下文重开），失败原因落日志。
    pub fn history(&self) -> Vec<ChatMessage> {
        match self.store.load(&self.id) {
            Ok(Some(session)) => session
                .messages
                .iter()
                .filter_map(|m| match m.role.as_str() {
                    "user" => Some(ChatMessage::user(m.text.clone())),
                    "assistant" => Some(ChatMessage::assistant(m.text.clone(), Vec::new())),
                    _ => None,
                })
                .collect(),
            Ok(None) => {
                logging::log(format!("会话历史读取：会话不存在（session={}）", self.id));
                Vec::new()
            }
            Err(e) => {
                logging::log(format!(
                    "会话历史读取失败（session={}）：{e}",
                    self.id
                ));
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sd_agent::model::Role;

    fn temp_workspace(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sd-tap-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp workspace");
        dir
    }

    /// append/load roundtrip：任务与模型回复按序落盘，历史映射回模型消息形态。
    #[test]
    fn session_append_load_roundtrip() {
        let ws = temp_workspace("roundtrip");
        let store = SessionStore::new(&ws);
        let session = store.create("测试会话").expect("create");
        let tap = SessionTap::new(&ws, session.id.clone());

        tap.append_user("帮我看看这个目录");
        tap.append_assistant("看完了");
        tap.append("tool", "（工具输出，不该进历史）");

        let loaded = store
            .load(&session.id)
            .expect("load")
            .expect("session exists");
        assert_eq!(loaded.messages.len(), 3);
        assert_eq!(loaded.messages[0].role, "user");
        assert_eq!(loaded.messages[0].text, "帮我看看这个目录");
        assert_eq!(loaded.messages[1].role, "assistant");
        assert!(loaded.messages.iter().all(|m| m.ts_unix_ms > 0));

        // 历史映射：user/assistant 进上下文，tool 跳过（不编造）。
        let history = tap.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, Role::User);
        assert_eq!(history[0].content, "帮我看看这个目录");
        assert_eq!(history[1].role, Role::Assistant);
        assert_eq!(history[1].content, "看完了");

        // 不存在的会话：append 静默降级（不 panic），历史返回空。
        let ghost = SessionTap::new(&ws, "ghost-session");
        ghost.append_user("这句不该写进去");
        assert!(ghost.history().is_empty());

        let _ = std::fs::remove_dir_all(&ws);
    }
}
