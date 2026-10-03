//! agent：工具调用循环 + 最大轮熔断（SD_AGENT_MAX_ROUNDS，默认 20）。
//!
//! 循环体只做四件事：组装上下文 → 模型调用 → policy::dispatch（唯一
//! 模型驱动执行入口）→ 事件留痕。任何裁决（放行/拒绝/路由）不在本层。

use std::path::Path;

use crate::config::{DisciplineTables, EnvProfile, resource::ResourceLedger};
use crate::context::ContextBuilder;
use crate::event::{
    EventPayload, LifecycleHook, ModelTurnFinished, ModelTurnStarted, RunFailed, RunFinished,
    RunStarted, SinkError, TraceRecorder, emit_hook,
};
use crate::model::{
    ChatMessage, ChatRequest, ChatResponse, ModelClient, ModelError, StreamObserver,
};
use crate::policy::{self, ApprovalPort, ToolCall};
use crate::tools::ToolSpec;

/// 一次 run 的配置。
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub task: String,
    pub max_rounds: u32,
    /// 初始对话历史（"恢复会话继续对话"的上下文来源）：run_task 开头
    /// 原样带入，task 作为最新一条 user 消息追加在其后。
    pub history: Vec<ChatMessage>,
}

/// run 收尾产物。
#[derive(Debug, Clone)]
pub struct RunOutcome {
    /// completed / max_rounds_reached。
    pub status: String,
    pub rounds: u32,
    pub final_text: String,
}

#[derive(Debug)]
pub enum AgentError {
    Sink(SinkError),
    Model(ModelError),
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentError::Sink(e) => write!(f, "trace sink error: {e}"),
            AgentError::Model(e) => write!(f, "model error: {e}"),
        }
    }
}

impl std::error::Error for AgentError {}

impl From<SinkError> for AgentError {
    fn from(e: SinkError) -> Self {
        AgentError::Sink(e)
    }
}

/// 显式依赖（壳层装配，不引全局状态）。
pub struct AgentDeps<'a> {
    pub model: &'a dyn ModelClient,
    pub recorder: &'a TraceRecorder,
    pub approval: &'a dyn ApprovalPort,
    pub env: &'a EnvProfile,
    pub root: &'a Path,
    pub ledger: &'a ResourceLedger,
    pub tables: &'a DisciplineTables,
    /// 流式观察者（可选）：Some → 每轮走 chat_stream，reasoning / 正文 /
    /// 工具调用 delta 实时上抛给壳（UI 两路接线点）；None → 行为与旧版一致
    /// （走 chat() 非流式）。
    pub stream_observer: Option<&'a dyn StreamObserver>,
}

/// 工具调用循环主入口。
pub async fn run_task(deps: &AgentDeps<'_>, cfg: &AgentConfig) -> Result<RunOutcome, AgentError> {
    let rec = deps.recorder;
    rec.record(EventPayload::RunStarted(RunStarted {
        task: cfg.task.clone(),
        max_rounds: cfg.max_rounds,
    }))?;
    emit_hook(rec, LifecycleHook::RunStart, "run started")?;

    let tools: Vec<ToolSpec> = crate::tools::catalog();
    let builder = ContextBuilder::new(&cfg.task, cfg.max_rounds);
    // 初始对话历史原样带入（追加不改写，硬约束 15），task 是最新一条 user 消息。
    let mut history: Vec<ChatMessage> = cfg.history.clone();
    history.push(ChatMessage::user(cfg.task.clone()));
    let mut final_text = String::new();

    for round in 1..=cfg.max_rounds {
        emit_hook(rec, LifecycleHook::RoundStart, &format!("round {round}"))?;
        rec.record(EventPayload::ModelTurnStarted(ModelTurnStarted {
            round,
            history_len: history.len(),
        }))?;

        let request = ChatRequest {
            messages: builder.build(&history, round),
            tools: tools.clone(),
            round,
        };
        // 有流式观察者 → chat_stream（三路 delta 实时上抛）；无 → chat()（行为与旧版一致）。
        let response: Result<ChatResponse, ModelError> = match deps.stream_observer {
            Some(obs) => deps.model.chat_stream(request, obs).await,
            None => deps.model.chat(request).await,
        };
        let response = match response {
            Ok(r) => r,
            Err(e) => {
                rec.record(EventPayload::RunFailed(RunFailed {
                    error: e.to_string(),
                }))?;
                // 失败收尾钩子与成功路径对称（钩子序列不留半截）。
                emit_hook(rec, LifecycleHook::RunEnd, "run failed")?;
                return Err(AgentError::Model(e));
            }
        };

        rec.record(EventPayload::ModelTurnFinished(ModelTurnFinished {
            round,
            text_chars: response.text.len(),
            tool_call_count: response.tool_calls.len(),
            // 思维链与 usage 进事件（成本账地基；delta 不逐 token 落盘）。
            reasoning_chars: response.reasoning_content.chars().count(),
            usage_prompt_tokens: response.usage.as_ref().map(|u| u.prompt_tokens),
            usage_completion_tokens: response.usage.as_ref().map(|u| u.completion_tokens),
        }))?;

        if response.tool_calls.is_empty() {
            final_text = response.text.clone();
            // assistant 消息带 reasoning_content 入历史（下轮回传，官方硬要求）。
            history.push(
                ChatMessage::assistant(response.text.clone(), vec![])
                    .with_reasoning_content(response.reasoning_content.clone()),
            );
            rec.record(EventPayload::RunFinished(RunFinished {
                rounds: round,
                status: "completed".to_string(),
            }))?;
            emit_hook(rec, LifecycleHook::RunEnd, "completed")?;
            return Ok(RunOutcome {
                status: "completed".to_string(),
                rounds: round,
                final_text,
            });
        }

        // 带工具调用的 assistant 消息进历史（含文本部分与 reasoning_content，
        // 追加不改写；reasoning_content 缺失回传会 400）。
        history.push(
            ChatMessage::assistant(response.text.clone(), response.tool_calls.clone())
                .with_reasoning_content(response.reasoning_content.clone()),
        );

        for call in &response.tool_calls {
            let tool_call = ToolCall {
                id: call.id.clone(),
                name: call.name.clone(),
                args_json: call.arguments_json.clone(),
            };
            let policy_ctx = policy::PolicyContext {
                env: deps.env,
                root: deps.root,
                recorder: rec,
                approval: deps.approval,
                ledger: deps.ledger,
                tables: deps.tables,
            };
            let result = policy::dispatch(&policy_ctx, &tool_call).await?;
            history.push(ChatMessage::tool_result(result.tool_call_id, result.text));
        }

        emit_hook(rec, LifecycleHook::RoundEnd, &format!("round {round} done"))?;
    }

    // 熔断：显式事件 + 显式状态（不吞）。
    rec.record(EventPayload::RunFinished(RunFinished {
        rounds: cfg.max_rounds,
        status: "max_rounds_reached".to_string(),
    }))?;
    emit_hook(rec, LifecycleHook::RunEnd, "max rounds reached")?;
    Ok(RunOutcome {
        status: "max_rounds_reached".to_string(),
        rounds: cfg.max_rounds,
        final_text,
    })
}
