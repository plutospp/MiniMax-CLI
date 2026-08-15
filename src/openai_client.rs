//! OpenAI-compatible Chat Completions client.
//!
//! Translates the engine's Anthropic-shaped [`MessageRequest`] /
//! [`StreamEvent`] types to/from OpenAI Chat Completions so the rest of the
//! app can stay protocol-agnostic.

use std::collections::HashMap;
use std::pin::Pin;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};

use crate::config::{ActiveProvider, Config, RetryPolicy};
use crate::llm_client::StreamEventBox;
use crate::logging;
use crate::models::{
    ContentBlock, ContentBlockStart, Delta, Message, MessageDelta, MessageRequest, MessageResponse,
    StreamEvent, SystemPrompt, Tool, Usage,
};

/// Client for OpenAI-compatible Chat Completions APIs.
#[derive(Clone)]
pub struct OpenAiTextClient {
    http_client: reqwest::Client,
    chat_url: String,
    retry: RetryPolicy,
    default_model: String,
    provider_name: String,
}

impl OpenAiTextClient {
    /// Build from the resolved active provider.
    pub fn from_provider(provider: &ActiveProvider, retry: RetryPolicy) -> Result<Self> {
        let chat_url = provider.openai_chat_completions_url();
        logging::info(format!(
            "OpenAI-compatible chat URL ({}) : {chat_url}",
            provider.name
        ));

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", provider.api_key))
                .context("Invalid API key for Authorization header")?,
        );

        let http_client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        Ok(Self {
            http_client,
            chat_url,
            retry,
            default_model: provider.default_model.clone(),
            provider_name: provider.name.clone(),
        })
    }

    /// Build from full CLI config (active provider must be OpenAI-compatible).
    #[allow(dead_code)]
    pub fn from_config(config: &Config) -> Result<Self> {
        let provider = config.active_provider()?;
        Self::from_provider(&provider, config.retry_policy())
    }

    #[must_use]
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    #[must_use]
    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    /// Non-streaming chat completion mapped back to [`MessageResponse`].
    pub async fn create_message(&self, request: MessageRequest) -> Result<MessageResponse> {
        let body = to_openai_request(&request, false)?;
        let response =
            send_with_retry(&self.retry, || self.http_client.post(&self.chat_url).json(&body))
                .await?;
        let value: Value = response.json().await?;
        from_openai_response(&value, &request.model)
    }

    /// Streaming chat completion emitting Anthropic-shaped [`StreamEvent`]s.
    pub async fn create_message_stream(&self, request: MessageRequest) -> Result<StreamEventBox> {
        let body = to_openai_request(&request, true)?;
        let response =
            send_with_retry(&self.retry, || self.http_client.post(&self.chat_url).json(&body))
                .await?;

        let byte_stream = response.bytes_stream();
        let model = request.model.clone();
        let mapped = openai_sse_to_stream_events(byte_stream, model);
        Ok(Pin::from(Box::new(mapped)))
    }
}

async fn send_with_retry<F>(policy: &RetryPolicy, mut build: F) -> Result<reqwest::Response>
where
    F: FnMut() -> reqwest::RequestBuilder,
{
    let mut attempt: u32 = 0;

    loop {
        let result = build().send().await;

        match result {
            Ok(response) => {
                if response.status().is_success() {
                    return Ok(response);
                }

                let status = response.status();
                let retryable = status.as_u16() == 429 || status.is_server_error();

                if !policy.enabled || !retryable || attempt >= policy.max_retries {
                    let text = response
                        .text()
                        .await
                        .unwrap_or_else(|e| format!("(failed to read body: {e})"));
                    anyhow::bail!("Failed to send API request: HTTP {status}: {text}");
                }
                logging::warn(format!(
                    "Retryable HTTP {} (attempt {} of {})",
                    status.as_u16(),
                    attempt + 1,
                    policy.max_retries + 1
                ));
            }
            Err(err) => {
                if !policy.enabled || attempt >= policy.max_retries {
                    return Err(err.into());
                }
                logging::warn(format!(
                    "Request error: {} (attempt {} of {})",
                    err,
                    attempt + 1,
                    policy.max_retries + 1
                ));
            }
        }

        let delay = policy.delay_for_attempt(attempt);
        attempt += 1;
        logging::info(format!("Retrying after {:.2}s", delay.as_secs_f64()));
        tokio::time::sleep(delay).await;
    }
}

/// Convert Anthropic-shaped request into OpenAI Chat Completions JSON.
pub fn to_openai_request(request: &MessageRequest, stream: bool) -> Result<Value> {
    let mut messages = Vec::new();

    if let Some(system) = &request.system {
        let text = system_prompt_text(system);
        if !text.trim().is_empty() {
            messages.push(json!({
                "role": "system",
                "content": text,
            }));
        }
    }

    for msg in &request.messages {
        messages.extend(convert_message(msg)?);
    }

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "max_tokens": request.max_tokens,
        "stream": stream,
    });

    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(top_p) = request.top_p {
        body["top_p"] = json!(top_p);
    }
    if let Some(tools) = &request.tools {
        body["tools"] = json!(tools.iter().map(convert_tool).collect::<Vec<_>>());
    }
    if let Some(choice) = &request.tool_choice {
        body["tool_choice"] = convert_tool_choice(choice);
    }

    Ok(body)
}

fn system_prompt_text(system: &SystemPrompt) -> String {
    match system {
        SystemPrompt::Text(t) => t.clone(),
        SystemPrompt::Blocks(blocks) => blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn convert_tool(tool: &Tool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.input_schema,
        }
    })
}

fn convert_tool_choice(choice: &Value) -> Value {
    // Anthropic: {"type":"auto"} / {"type":"any"} / {"type":"tool","name":"..."}
    if let Some(t) = choice.get("type").and_then(|v| v.as_str()) {
        match t {
            "auto" => return json!("auto"),
            "any" | "required" => return json!("required"),
            "none" => return json!("none"),
            "tool" => {
                if let Some(name) = choice.get("name").and_then(|v| v.as_str()) {
                    return json!({
                        "type": "function",
                        "function": { "name": name }
                    });
                }
            }
            _ => {}
        }
    }
    choice.clone()
}

fn convert_message(msg: &Message) -> Result<Vec<Value>> {
    let role = msg.role.as_str();
    match role {
        "user" => {
            let mut out = Vec::new();
            let mut text_parts = Vec::new();
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text, .. } => text_parts.push(text.clone()),
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                    } => {
                        // Flush pending user text first.
                        if !text_parts.is_empty() {
                            out.push(json!({
                                "role": "user",
                                "content": text_parts.join("\n"),
                            }));
                            text_parts.clear();
                        }
                        out.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": content,
                        }));
                    }
                    ContentBlock::Thinking { .. } | ContentBlock::ToolUse { .. } => {}
                }
            }
            if !text_parts.is_empty() {
                out.push(json!({
                    "role": "user",
                    "content": text_parts.join("\n"),
                }));
            }
            if out.is_empty() {
                out.push(json!({ "role": "user", "content": "" }));
            }
            Ok(out)
        }
        "assistant" => {
            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text, .. } => text_parts.push(text.clone()),
                    ContentBlock::Thinking { thinking } => {
                        // Preserve thinking as plain text context when present.
                        if !thinking.trim().is_empty() {
                            text_parts.push(thinking.clone());
                        }
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": serde_json::to_string(input).unwrap_or_else(|_| "{}".into()),
                            }
                        }));
                    }
                    ContentBlock::ToolResult { .. } => {}
                }
            }
            let mut message = json!({ "role": "assistant" });
            if !text_parts.is_empty() {
                message["content"] = json!(text_parts.join("\n"));
            } else if tool_calls.is_empty() {
                message["content"] = json!("");
            } else {
                message["content"] = Value::Null;
            }
            if !tool_calls.is_empty() {
                message["tool_calls"] = json!(tool_calls);
            }
            Ok(vec![message])
        }
        "system" => {
            let text = msg
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(vec![json!({ "role": "system", "content": text })])
        }
        other => {
            // Treat unknown roles as user text.
            let text = msg
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(vec![json!({ "role": other, "content": text })])
        }
    }
}

/// Map a non-stream OpenAI response into Anthropic [`MessageResponse`].
pub fn from_openai_response(value: &Value, fallback_model: &str) -> Result<MessageResponse> {
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("openai-response")
        .to_string();
    let model = value
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(fallback_model)
        .to_string();

    let choice = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .context("OpenAI response missing choices[0]")?;

    let message = choice
        .get("message")
        .context("OpenAI choice missing message")?;

    let mut content = Vec::new();
    if let Some(text) = message.get("content").and_then(|v| v.as_str())
        && !text.is_empty()
    {
        content.push(ContentBlock::Text {
            text: text.to_string(),
            cache_control: None,
        });
    }
    if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for call in calls {
            let id = call
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("tool_call")
                .to_string();
            let name = call
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let args_str = call
                .pointer("/function/arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let input = serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));
            content.push(ContentBlock::ToolUse { id, name, input });
        }
    }

    let stop_reason = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .map(|r| match r {
            "tool_calls" => "tool_use".to_string(),
            "length" => "max_tokens".to_string(),
            other => other.to_string(),
        });

    let usage = Usage {
        input_tokens: value
            .pointer("/usage/prompt_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        output_tokens: value
            .pointer("/usage/completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
    };

    Ok(MessageResponse {
        id,
        r#type: "message".to_string(),
        role: "assistant".to_string(),
        content,
        model,
        stop_reason,
        stop_sequence: None,
        usage,
    })
}

struct ToolCallBuilder {
    id: String,
    name: String,
    arguments: String,
    started: bool,
}

fn openai_sse_to_stream_events(
    stream: impl futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> + Unpin + Send + 'static,
    model: String,
) -> impl futures_util::Stream<Item = Result<StreamEvent>> + Send {
    async_stream::try_stream! {
        let mut buffer = String::new();
        let mut stream = stream;
        let mut started = false;
        let mut next_index: u32 = 0;
        let mut text_index: Option<u32> = None;
        let mut tool_builders: HashMap<u32, ToolCallBuilder> = HashMap::new();
        let mut tool_block_index: HashMap<u32, u32> = HashMap::new();
        let mut finish_reason: Option<String> = None;
        let mut usage = Usage { input_tokens: 0, output_tokens: 0 };

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(chunk) => chunk,
                Err(err) => {
                    logging::warn(format!("OpenAI SSE chunk error: {err}"));
                    continue;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find("\n\n") {
                let block = buffer[..pos].to_string();
                buffer.drain(..pos + 2);

                for line in block.lines() {
                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };
                    if data == "[DONE]" {
                        if let Some(idx) = text_index.take() {
                            yield StreamEvent::ContentBlockStop { index: idx };
                        }
                        let mut keys: Vec<u32> = tool_builders.keys().copied().collect();
                        keys.sort_unstable();
                        for openai_idx in keys {
                            let Some(builder) = tool_builders.remove(&openai_idx) else {
                                continue;
                            };
                            let block_idx = if let Some(&idx) = tool_block_index.get(&openai_idx) {
                                idx
                            } else {
                                let idx = next_index;
                                next_index += 1;
                                yield StreamEvent::ContentBlockStart {
                                    index: idx,
                                    content_block: ContentBlockStart::ToolUse {
                                        id: if builder.id.is_empty() {
                                            format!("tool_{openai_idx}")
                                        } else {
                                            builder.id.clone()
                                        },
                                        name: if builder.name.is_empty() {
                                            "unknown".into()
                                        } else {
                                            builder.name.clone()
                                        },
                                        input: json!({}),
                                    },
                                };
                                if !builder.arguments.is_empty() {
                                    yield StreamEvent::ContentBlockDelta {
                                        index: idx,
                                        delta: Delta::InputJsonDelta {
                                            partial_json: builder.arguments.clone(),
                                        },
                                    };
                                }
                                idx
                            };
                            if builder.started && !builder.arguments.is_empty() {
                                yield StreamEvent::ContentBlockDelta {
                                    index: block_idx,
                                    delta: Delta::InputJsonDelta {
                                        partial_json: builder.arguments,
                                    },
                                };
                            }
                            yield StreamEvent::ContentBlockStop { index: block_idx };
                        }
                        let stop = finish_reason.map(|r| match r.as_str() {
                            "tool_calls" => "tool_use".to_string(),
                            "length" => "max_tokens".to_string(),
                            other => other.to_string(),
                        });
                        yield StreamEvent::MessageDelta {
                            delta: MessageDelta {
                                stop_reason: stop,
                                stop_sequence: None,
                            },
                            usage: Some(usage.clone()),
                        };
                        yield StreamEvent::MessageStop;
                        return;
                    }

                    let Ok(value) = serde_json::from_str::<Value>(data) else {
                        logging::warn(format!("Failed to parse OpenAI SSE: {data}"));
                        continue;
                    };

                    if !started {
                        started = true;
                        let id = value
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("openai-stream")
                            .to_string();
                        let resp_model = value
                            .get("model")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&model)
                            .to_string();
                        yield StreamEvent::MessageStart {
                            message: MessageResponse {
                                id,
                                r#type: "message".to_string(),
                                role: "assistant".to_string(),
                                content: vec![],
                                model: resp_model,
                                stop_reason: None,
                                stop_sequence: None,
                                usage: Usage {
                                    input_tokens: 0,
                                    output_tokens: 0,
                                },
                            },
                        };
                    }

                    if let Some(u) = value.get("usage") {
                        if let Some(p) = u.get("prompt_tokens").and_then(|v| v.as_u64()) {
                            usage.input_tokens = p as u32;
                        }
                        if let Some(c) = u.get("completion_tokens").and_then(|v| v.as_u64()) {
                            usage.output_tokens = c as u32;
                        }
                    }

                    let Some(choice) = value
                        .get("choices")
                        .and_then(|c| c.as_array())
                        .and_then(|arr| arr.first())
                    else {
                        continue;
                    };

                    if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                        finish_reason = Some(fr.to_string());
                    }

                    let Some(delta) = choice.get("delta") else {
                        continue;
                    };

                    if let Some(content) = delta.get("content").and_then(|v| v.as_str())
                        && !content.is_empty()
                    {
                        if text_index.is_none() {
                            let idx = next_index;
                            next_index += 1;
                            text_index = Some(idx);
                            yield StreamEvent::ContentBlockStart {
                                index: idx,
                                content_block: ContentBlockStart::Text {
                                    text: String::new(),
                                },
                            };
                        }
                        if let Some(idx) = text_index {
                            yield StreamEvent::ContentBlockDelta {
                                index: idx,
                                delta: Delta::TextDelta {
                                    text: content.to_string(),
                                },
                            };
                        }
                    }

                    if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                        if text_index.is_some() && !tool_calls.is_empty()
                            && let Some(idx) = text_index.take()
                        {
                            yield StreamEvent::ContentBlockStop { index: idx };
                        }

                        for call in tool_calls {
                            let openai_idx =
                                call.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let entry =
                                tool_builders
                                    .entry(openai_idx)
                                    .or_insert_with(|| ToolCallBuilder {
                                        id: String::new(),
                                        name: String::new(),
                                        arguments: String::new(),
                                        started: false,
                                    });

                            if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                                entry.id = id.to_string();
                            }
                            if let Some(name) =
                                call.pointer("/function/name").and_then(|v| v.as_str())
                            {
                                entry.name = name.to_string();
                            }
                            let new_args = call
                                .pointer("/function/arguments")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

                            if !entry.started && !entry.id.is_empty() && !entry.name.is_empty() {
                                let block_idx = next_index;
                                next_index += 1;
                                tool_block_index.insert(openai_idx, block_idx);
                                entry.started = true;
                                yield StreamEvent::ContentBlockStart {
                                    index: block_idx,
                                    content_block: ContentBlockStart::ToolUse {
                                        id: entry.id.clone(),
                                        name: entry.name.clone(),
                                        input: json!({}),
                                    },
                                };
                                if !new_args.is_empty() {
                                    yield StreamEvent::ContentBlockDelta {
                                        index: block_idx,
                                        delta: Delta::InputJsonDelta {
                                            partial_json: new_args.to_string(),
                                        },
                                    };
                                }
                                // Keep buffer empty — fragments already emitted.
                                entry.arguments.clear();
                            } else if entry.started
                                && !new_args.is_empty()
                                && let Some(&block_idx) = tool_block_index.get(&openai_idx)
                            {
                                yield StreamEvent::ContentBlockDelta {
                                    index: block_idx,
                                    delta: Delta::InputJsonDelta {
                                        partial_json: new_args.to_string(),
                                    },
                                };
                            } else if !new_args.is_empty() {
                                // Not started yet — buffer until id+name arrive.
                                entry.arguments.push_str(new_args);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Message;

    #[test]
    fn maps_basic_user_assistant() {
        let req = MessageRequest {
            model: "gpt-4o".into(),
            messages: vec![Message {
                role: "user".into(),
                content: vec![ContentBlock::Text {
                    text: "hi".into(),
                    cache_control: None,
                }],
            }],
            max_tokens: 100,
            system: Some(SystemPrompt::Text("sys".into())),
            tools: None,
            tool_choice: None,
            metadata: None,
            thinking: None,
            stream: None,
            temperature: None,
            top_p: None,
        };
        let body = to_openai_request(&req, false).unwrap();
        assert_eq!(body["model"], "gpt-4o");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "hi");
    }

    #[test]
    fn maps_tool_use_and_result() {
        let req = MessageRequest {
            model: "gpt-4o".into(),
            messages: vec![
                Message {
                    role: "assistant".into(),
                    content: vec![ContentBlock::ToolUse {
                        id: "call_1".into(),
                        name: "bash".into(),
                        input: json!({"cmd": "ls"}),
                    }],
                },
                Message {
                    role: "user".into(),
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: "call_1".into(),
                        content: "ok".into(),
                    }],
                },
            ],
            max_tokens: 100,
            system: None,
            tools: Some(vec![Tool {
                name: "bash".into(),
                description: "run".into(),
                input_schema: json!({"type": "object"}),
                cache_control: None,
            }]),
            tool_choice: Some(json!({"type": "auto"})),
            metadata: None,
            thinking: None,
            stream: None,
            temperature: None,
            top_p: None,
        };
        let body = to_openai_request(&req, false).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "assistant");
        assert!(messages[0]["tool_calls"].is_array());
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_1");
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn maps_openai_response_with_tools() {
        let value = json!({
            "id": "chatcmpl-1",
            "model": "gpt-4o",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"a.rs\"}"
                        }
                    }]
                }
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });
        let resp = from_openai_response(&value, "fallback").unwrap();
        assert_eq!(resp.stop_reason.as_deref(), Some("tool_use"));
        assert!(matches!(resp.content[0], ContentBlock::ToolUse { .. }));
        assert_eq!(resp.usage.input_tokens, 10);
    }
}
