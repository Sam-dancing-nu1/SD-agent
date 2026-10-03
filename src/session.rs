//! 会话持久层（历史对话 / 对话切换的外围最小闭环）：每个会话一个 JSON
//! 文件，落在 `<工作区>/.sd-agent/sessions/` 下，供桌面端 / TUI 做
//! "历史会话列表 / 切换会话 / 恢复会话继续对话"。
//!
//! 口径：
//! - 一个会话 = 一个 JSON 文件（serde_json 全量读写），文件名 = `<id>.json`；
//! - id 由 create() 生成（毫秒时间戳 + 进程号 + 自增序号），文件名安全字符
//!   白名单校验，任何入口的 id 都不直接拼路径（防路径穿越）；
//! - 列表（list）只取元数据不载消息，按更新时间倒序；坏文件跳过不拖垮列表；
//! - 会话 ↔ 模型消息形态的转换属于调用方（session 层零业务依赖），
//!   "恢复会话继续对话" 走 agent::AgentConfig.history 带入初始对话历史。

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// 会话内一条消息（role 语义："user" | "assistant" | "tool"）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    /// "user" | "assistant" | "tool"。
    pub role: String,
    /// 消息正文。
    pub text: String,
    /// 写入时间（毫秒时间戳）。
    pub ts_unix_ms: u64,
    /// 思维链正文（MiMo reasoning_content，可选）：assistant 消息存档，
    /// 恢复会话续跑时回传历史（官方硬要求，缺失 400）。
    /// 只增字段：旧会话文件缺字段时按 None 解析。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

impl SessionMessage {
    /// 构造一条消息（无思维链）。
    pub fn new(role: impl Into<String>, text: impl Into<String>, ts_unix_ms: u64) -> Self {
        Self {
            role: role.into(),
            text: text.into(),
            ts_unix_ms,
            reasoning_content: None,
        }
    }

    /// 追加思维链正文（assistant 消息存档回传用；空串归一为 None）。
    pub fn with_reasoning_content(mut self, reasoning: impl Into<String>) -> Self {
        let r = reasoning.into();
        self.reasoning_content = if r.is_empty() { None } else { Some(r) };
        self
    }
}

/// 一次会话（全量形态：元数据 + 全部消息）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub messages: Vec<SessionMessage>,
}

/// 会话列表元数据（不载消息，列表展示零成本）。
#[derive(Debug, Clone, Serialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub updated_at_ms: u64,
    pub message_count: usize,
}

/// 会话仓库：`<工作区>/.sd-agent/sessions/`。
pub struct SessionStore {
    root: PathBuf,
}

/// create() 用的进程内自增序号（防同毫秒碰撞）。
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

impl SessionStore {
    /// 绑定工作区；目录惰性创建（首次写入时建）。
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            root: workspace_root.join(".sd-agent").join("sessions"),
        }
    }

    /// 会话文件目录（展示用）。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 会话列表（updated_at 倒序）；目录不存在返回空列表，坏文件跳过。
    pub fn list(&self) -> Vec<SessionMeta> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(session) = serde_json::from_str::<Session>(&text) else {
                continue;
            };
            out.push(SessionMeta {
                id: session.id,
                title: session.title,
                updated_at_ms: session.updated_at_ms,
                message_count: session.messages.len(),
            });
        }
        out.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
        out
    }

    /// 新建会话并立即落盘（返回的 Session 即落盘形态）。
    pub fn create(&self, title: &str) -> std::io::Result<Session> {
        let now = crate::event::now_unix_ms();
        let seq = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        let session = Session {
            id: format!("s{now}-{}-{seq}", std::process::id()),
            title: title.to_string(),
            created_at_ms: now,
            updated_at_ms: now,
            messages: Vec::new(),
        };
        self.save(&session)?;
        Ok(session)
    }

    /// 读一个会话；文件不存在返回 Ok(None)，id 非法返回 InvalidInput。
    pub fn load(&self, id: &str) -> std::io::Result<Option<Session>> {
        let path = self.file_for(id)?;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let session = serde_json::from_str::<Session>(&text)
                    .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
                Ok(Some(session))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 全量写回（按 s.id 落盘，不改写内容）。
    pub fn save(&self, s: &Session) -> std::io::Result<()> {
        let path = self.file_for(&s.id)?;
        std::fs::create_dir_all(&self.root)?;
        let json =
            serde_json::to_string_pretty(s).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }

    /// 追加一条消息并更新会话更新时间；会话不存在返回 NotFound。
    pub fn append(&self, id: &str, msg: SessionMessage) -> std::io::Result<()> {
        let mut session = self
            .load(id)?
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("会话不存在：{id}")))?;
        session.messages.push(msg);
        session.updated_at_ms = crate::event::now_unix_ms();
        self.save(&session)
    }

    /// 删除会话文件（幂等：不存在也算成功）。
    pub fn delete(&self, id: &str) -> std::io::Result<()> {
        let path = self.file_for(id)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// id → 文件路径（白名单校验后拼接，防路径穿越）。
    fn file_for(&self, id: &str) -> std::io::Result<PathBuf> {
        if !is_safe_id(id) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("会话 id 含非法字符：{id}"),
            ));
        }
        Ok(self.root.join(format!("{id}.json")))
    }
}

/// id 白名单：非空、长度有限、只允许字母数字与 - _。
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sd-session-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp workspace");
        dir
    }

    #[test]
    fn session_roundtrip_create_append_load_list_delete() {
        let ws = temp_workspace("roundtrip");
        let store = SessionStore::new(&ws);

        let s = store.create("第一次对话").expect("create");
        assert!(s.id.starts_with('s'));
        assert!(s.messages.is_empty());

        store
            .append(
                &s.id,
                SessionMessage::new("user", "帮我看看这个目录", 1),
            )
            .expect("append user");
        store
            .append(
                &s.id,
                SessionMessage::new("assistant", "看完了", 2)
                    .with_reasoning_content("先想了一下目录结构"),
            )
            .expect("append assistant");

        let loaded = store
            .load(&s.id)
            .expect("load")
            .expect("session must exist");
        assert_eq!(loaded.id, s.id);
        assert_eq!(loaded.title, "第一次对话");
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].role, "user");
        assert_eq!(loaded.messages[1].text, "看完了");
        // reasoning_content 存档 roundtrip（恢复会话回传的依据）。
        assert_eq!(loaded.messages[0].reasoning_content, None);
        assert_eq!(
            loaded.messages[1].reasoning_content.as_deref(),
            Some("先想了一下目录结构")
        );

        // 修改后全量写回，再读一致（save 是全量语义）。
        let mut edited = loaded.clone();
        edited.title = "改名后的会话".to_string();
        store.save(&edited).expect("save");
        let reloaded = store.load(&s.id).expect("load").expect("exists");
        assert_eq!(reloaded.title, "改名后的会话");
        assert_eq!(reloaded.messages.len(), 2);

        // 列表：元数据齐全、按更新时间倒序。
        let second = store.create("第二个会话").expect("create 2");
        let list = store.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, second.id, "新会话应排最前");
        let meta = list.iter().find(|m| m.id == s.id).expect("meta");
        assert_eq!(meta.title, "改名后的会话");
        assert_eq!(meta.message_count, 2);

        // 删除幂等。
        store.delete(&s.id).expect("delete");
        store.delete(&s.id).expect("delete again is ok");
        assert!(store.load(&s.id).expect("load").is_none());
        assert_eq!(store.list().len(), 1);

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn session_rejects_unsafe_id_and_missing_append() {
        let ws = temp_workspace("safety");
        let store = SessionStore::new(&ws);
        assert!(store.load("../evil").is_err(), "路径穿越 id 必须拒绝");
        assert!(store.delete("a/b").is_err(), "分隔符 id 必须拒绝");
        assert!(
            store
                .save(&Session {
                    id: "..".to_string(),
                    title: "x".to_string(),
                    created_at_ms: 0,
                    updated_at_ms: 0,
                    messages: vec![],
                })
                .is_err()
        );

        let err = store
            .append("missing-id", SessionMessage::new("user", "hi", 0))
            .expect_err("missing session must error");
        assert_eq!(err.kind(), ErrorKind::NotFound);

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn old_session_files_without_reasoning_content_still_load() {
        // 只增字段兼容：旧会话文件（无 reasoning_content）照常载入，字段为 None。
        let ws = temp_workspace("oldfmt");
        let dir = ws.join(".sd-agent").join("sessions");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("s-old.json"),
            r#"{"id":"s-old","title":"旧会话","created_at_ms":1,"updated_at_ms":1,
                "messages":[{"role":"assistant","text":"答","ts_unix_ms":2}]}"#,
        )
        .expect("write old session");
        let store = SessionStore::new(&ws);
        let s = store.load("s-old").expect("load").expect("exists");
        assert_eq!(s.messages[0].reasoning_content, None);
        // 新字段不序列化 None（保持旧形态字节稳定）。
        let json = serde_json::to_string(&s.messages[0]).unwrap();
        assert!(!json.contains("reasoning_content"));

        let _ = std::fs::remove_dir_all(&ws);
    }
}
