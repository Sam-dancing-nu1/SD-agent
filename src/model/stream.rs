use std::collections::BTreeMap;

use super::wire::WireChunk;
use super::{ChatResponse, ModelError, StreamObserver, ToolCallRequest, Usage};

/// SSE 帧解析器（生产路径：流式响应字节流 → data 载荷序列）。
/// 只取 data 行（多行以 \n 连接），忽略注释行与其他字段；\r\n / \n 均收；
/// 字节级跨包切分不丢不重。
pub(super) struct SseParser {
    buf: Vec<u8>,
    /// 当前事件已收集的 data 行（空行 = 事件边界时拼接吐出）。
    event_data: Vec<String>,
}

impl SseParser {
    pub(super) fn new() -> Self {
        Self {
            buf: Vec::new(),
            event_data: Vec::new(),
        }
    }

    /// 喂入一段原始字节，吐出本次新完成事件的 data 载荷（按序）。
    pub(super) fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let Some(pos) = self.buf.iter().position(|b| *b == b'\n') else {
                break;
            };
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop(); // 去 \n
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if line.is_empty() {
                if !self.event_data.is_empty() {
                    out.push(self.event_data.join("\n"));
                    self.event_data.clear();
                }
                continue;
            }
            if line.starts_with(':') {
                continue; // 注释行（含 keepalive）
            }
            if let Some(value) = line.strip_prefix("data:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                self.event_data.push(value.to_string());
            }
            // 其他字段（event: / id: / retry:）忽略
        }
        out
    }
}

/// 流式累加器：chunk 解析 → 三路 delta 回调 → ChatResponse 收尾。
/// mock SSE 测试直接驱动本层（与生产路径同一解析代码）。
pub(super) struct StreamAssembler<'a> {
    round: u32,
    obs: Option<&'a dyn StreamObserver>,
    reasoning: String,
    text: String,
    calls: BTreeMap<usize, ToolCallRequest>,
    usage: Option<Usage>,
    /// 是否已上抛过任何 delta（决定失败后能否重试）。
    pub(super) emitted: bool,
}

impl<'a> StreamAssembler<'a> {
    pub(super) fn new(round: u32, obs: Option<&'a dyn StreamObserver>) -> Self {
        Self {
            round,
            obs,
            reasoning: String::new(),
            text: String::new(),
            calls: BTreeMap::new(),
            usage: None,
            emitted: false,
        }
    }

    /// 重试前清空半截状态（只在未上抛过 delta 时调用）。
    pub(super) fn reset(&mut self) {
        self.reasoning.clear();
        self.text.clear();
        self.calls.clear();
        self.usage = None;
    }

    /// 消费一个 SSE data 载荷（JSON chunk 原文）。
    pub(super) fn feed_chunk_json(&mut self, json: &str) -> Result<(), ModelError> {
        let chunk: WireChunk = serde_json::from_str(json)
            .map_err(|e| ModelError::BadResponse(format!("流式 chunk 解析失败：{e}")))?;
        self.feed_chunk(chunk);
        Ok(())
    }

    fn feed_chunk(&mut self, chunk: WireChunk) {
        if let Some(u) = chunk.usage {
            self.usage = Some(Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
            });
        }
        for choice in chunk.choices {
            let delta = choice.delta;
            if let Some(r) = delta.reasoning_content {
                if !r.is_empty() {
                    self.reasoning.push_str(&r);
                    self.emitted = true;
                    if let Some(obs) = self.obs {
                        obs.on_reasoning_delta(self.round, &r);
                    }
                }
            }
            if let Some(t) = delta.content {
                if !t.is_empty() {
                    self.text.push_str(&t);
                    self.emitted = true;
                    if let Some(obs) = self.obs {
                        obs.on_text_delta(self.round, &t);
                    }
                }
            }
            for tc in delta.tool_calls.unwrap_or_default() {
                let slot = self
                    .calls
                    .entry(tc.index)
                    .or_insert_with(|| ToolCallRequest {
                        id: String::new(),
                        name: String::new(),
                        arguments_json: String::new(),
                    });
                if let Some(id) = tc.id {
                    slot.id = id;
                }
                if let Some(f) = tc.function {
                    if let Some(n) = f.name {
                        slot.name.push_str(&n);
                    }
                    if let Some(a) = f.arguments {
                        slot.arguments_json.push_str(&a);
                    }
                }
                self.emitted = true;
                if let Some(obs) = self.obs {
                    obs.on_tool_call_delta(self.round, &slot.name, &slot.arguments_json);
                }
            }
        }
    }

    /// 一轮收尾：回调整轮完成（每轮恰一次），产出与 chat() 同形态的响应。
    pub(super) fn finish(&mut self) -> ChatResponse {
        let tool_calls: Vec<ToolCallRequest> = self.calls.values().cloned().collect();
        if let Some(obs) = self.obs {
            obs.on_turn_done(self.round);
        }
        ChatResponse {
            text: std::mem::take(&mut self.text),
            tool_calls,
            usage: self.usage.clone(),
            reasoning_content: std::mem::take(&mut self.reasoning),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 录制型观察者：记录三路 delta 与收尾（mock 测试断言用）。
    #[derive(Default)]
    struct Rec {
        reasoning: Mutex<String>,
        text: Mutex<String>,
        tools: Mutex<Vec<(String, String)>>,
        done_rounds: Mutex<Vec<u32>>,
        rounds: Mutex<Vec<u32>>,
    }

    impl StreamObserver for Rec {
        fn on_reasoning_delta(&self, round: u32, delta: &str) {
            self.rounds.lock().unwrap().push(round);
            self.reasoning.lock().unwrap().push_str(delta);
        }
        fn on_text_delta(&self, round: u32, delta: &str) {
            self.rounds.lock().unwrap().push(round);
            self.text.lock().unwrap().push_str(delta);
        }
        fn on_tool_call_delta(&self, round: u32, name: &str, args_so_far: &str) {
            self.rounds.lock().unwrap().push(round);
            self.tools
                .lock()
                .unwrap()
                .push((name.to_string(), args_so_far.to_string()));
        }
        fn on_turn_done(&self, round: u32) {
            self.done_rounds.lock().unwrap().push(round);
        }
    }

    /// 按字节粒度喂 SSE（模拟网络分片），收集 data 载荷。
    fn feed_sse_by_bytes(parser: &mut SseParser, text: &str, step: usize) -> Vec<String> {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let end = (i + step).min(bytes.len());
            out.extend(parser.feed(&bytes[i..end]));
            i = end;
        }
        out
    }

    const MOCK_SSE: &str = "\
data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先想\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"三步\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"答案是\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"42\"}}]}\n\n\
data: {\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7},\"choices\":[]}\n\n\
data: [DONE]\n\n";

    #[test]
    fn mock_sse_two_path_stream_and_usage() {
        let rec = Rec::default();
        let mut parser = SseParser::new();
        let mut asm = StreamAssembler::new(3, Some(&rec));
        for payload in feed_sse_by_bytes(&mut parser, MOCK_SSE, 7) {
            if payload.trim() == "[DONE]" {
                continue;
            }
            asm.feed_chunk_json(&payload).unwrap();
        }
        let resp = asm.finish();
        assert_eq!(resp.reasoning_content, "先想三步");
        assert_eq!(resp.text, "答案是42");
        assert_eq!(resp.usage.as_ref().unwrap().prompt_tokens, 11);
        assert_eq!(resp.usage.as_ref().unwrap().completion_tokens, 7);
        // 两路 delta 都以 round=3 上抛，turn 恰收尾一次。
        assert_eq!(*rec.reasoning.lock().unwrap(), "先想三步");
        assert_eq!(*rec.text.lock().unwrap(), "答案是42");
        assert!(rec.rounds.lock().unwrap().iter().all(|r| *r == 3));
        assert_eq!(*rec.done_rounds.lock().unwrap(), vec![3]);
    }

    const MOCK_SSE_TOOLS: &str = "\
data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"要读文件\",\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"pa\"}}]}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":\\\"a.txt\"}}]}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"}\"}}]}}]}\n\n\
data: {\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":6},\"choices\":[]}\n\n\
data: [DONE]\n\n";

    #[test]
    fn mock_sse_tool_call_delta_accumulates_args() {
        let rec = Rec::default();
        let mut parser = SseParser::new();
        let mut asm = StreamAssembler::new(1, Some(&rec));
        for payload in feed_sse_by_bytes(&mut parser, MOCK_SSE_TOOLS, 11) {
            if payload.trim() == "[DONE]" {
                continue;
            }
            asm.feed_chunk_json(&payload).unwrap();
        }
        let resp = asm.finish();
        assert_eq!(resp.reasoning_content, "要读文件");
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_9");
        assert_eq!(resp.tool_calls[0].name, "read");
        assert_eq!(resp.tool_calls[0].arguments_json, "{\"path\":\"a.txt\"}");
        // args_so_far 累计值逐段上抛（3 段）。
        let tools = rec.tools.lock().unwrap().clone();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].1, "{\"pa");
        assert_eq!(tools[1].1, "{\"path\":\"a.txt");
        assert_eq!(tools[2].1, "{\"path\":\"a.txt\"}");
        assert!(tools.iter().all(|(n, _)| n == "read"));
        assert_eq!(*rec.done_rounds.lock().unwrap(), vec![1]);
    }

    #[test]
    fn sse_parser_handles_comments_crlf_and_split_frames() {
        let mut p = SseParser::new();
        let text = ": keepalive\r\ndata: {\"a\":1}\r\n\r\ndata: {\"b\":2}\n\n";
        // 逐字节喂：跨包切分不丢不重。
        let payloads = feed_sse_by_bytes(&mut p, text, 1);
        assert_eq!(
            payloads,
            vec!["{\"a\":1}".to_string(), "{\"b\":2}".to_string()]
        );
    }
}
