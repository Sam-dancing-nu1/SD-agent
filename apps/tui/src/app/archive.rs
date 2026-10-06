//! 会话存档：feed 投影 → 会话库追加。
//!
//! 口径：会话库只存对话正文（user / assistant 含思维链正文）；工具调用与
//! 干预条目的完整留痕在轨迹文件（JSONL 事件），不重复入会话库。存档是
//! **追加语义**——只写 feed 中尚未归档的新增（saved_count 记账），杜绝
//! 全量替换吃掉旧消息（--session 冷启动 / /clear 场景）。
//! reasoning_content 随 assistant 消息入档（续跑回传是端点硬要求）。

use sd_agent::session::{SessionMessage, SessionStore};

use super::App;
use super::worker::FeedItem;

/// now_ms：消息入档时间。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl App {
    /// worker 收尾后存档（run 刚结束 / 退出前调用）。
    pub fn archive_session(&mut self) {
        let super::Mode::Worker(w) = &mut self.mode else {
            return;
        };
        // 只投影尚未归档的新增（追加语义）。
        let new_msgs: Vec<SessionMessage> = w
            .feed
            .iter()
            .skip(w.saved_count)
            .filter_map(|item| match item {
                FeedItem::User(t) => Some(SessionMessage::new("user", t.clone(), now_ms())),
                FeedItem::Assistant {
                    text, reasoning, ..
                } if !text.is_empty() || !reasoning.is_empty() => Some(
                    SessionMessage::new("assistant", text.clone(), now_ms())
                        .with_reasoning_content(reasoning.clone()),
                ),
                _ => None,
            })
            .collect();
        let already = w.feed.len();
        if new_msgs.is_empty() {
            return;
        }
        let store = SessionStore::new(&self.root);
        let result: std::io::Result<String> = (|| -> std::io::Result<String> {
            // 已有会话 → 逐条追加；无会话（或 load 失败）→ 新建后追加。
            let session = match &w.session_id {
                Some(id) => match store.load(id) {
                    Ok(Some(s)) => Some(s),
                    _ => None,
                },
                None => None,
            };
            let s = match session {
                Some(s) => s,
                None => store.create(&first_title(&w.feed))?,
            };
            for m in new_msgs {
                store.append(&s.id, m)?;
            }
            Ok(s.id.clone())
        })();
        match result {
            Ok(id) => {
                w.session_id = Some(id.clone());
                w.saved_count = already;
                let _ = self.tx.send(super::UiMsg::SessionSaved {
                    id,
                    title: String::new(),
                });
            }
            Err(e) => {
                let msg = format!("会话存档失败: {e}");
                let hint = "检查工作区可写（/doctor 第 3 项）".to_string();
                if let super::Mode::Worker(w) = &mut self.mode {
                    w.push(FeedItem::Error { message: msg, hint });
                }
            }
        }
    }
}

/// 会话标题取首条用户消息（截断）。
fn first_title(feed: &[FeedItem]) -> String {
    feed.iter()
        .find_map(|i| match i {
            FeedItem::User(t) => Some(t.clone()),
            _ => None,
        })
        .map(|t| t.chars().take(30).collect::<String>())
        .unwrap_or_else(|| "会话".into())
}
