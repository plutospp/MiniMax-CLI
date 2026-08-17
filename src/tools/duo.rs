//! Tools for Duo mode: Player-Coach autocoding workflow.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::duo::{
    CoachFile, DuoPhase, SharedDuoSession, generate_coach_prompt, generate_player_prompt,
    session_summary,
};
use crate::tools::spec::{
    ApprovalRequirement, ToolCapability, ToolContext, ToolError, ToolResult, ToolSpec,
    normalize_at_path, optional_str, required_str,
};

/// Per-file byte cap for coach ground-truth inlining.
const MAX_COACH_FILE_BYTES: usize = 64 * 1024;
/// Total byte cap across all files inlined into one coach prompt.
const MAX_COACH_TOTAL_BYTES: usize = 192 * 1024;

/// Executes the Duo coach turn against a specific model.
///
/// Built by the engine from the active provider's text client plus a model
/// override, so the coach can differ from the player (active) model.
pub struct CoachExecution {
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
    call: CoachCall,
}

/// Injectable coach model call (a real `TextClient` in production, a canned
/// response in tests).
pub type CoachCall = std::sync::Arc<
    dyn Fn(
            crate::models::MessageRequest,
        ) -> futures_util::future::BoxFuture<
            'static,
            anyhow::Result<crate::models::MessageResponse>,
        > + Send
        + Sync,
>;

impl CoachExecution {
    #[must_use]
    pub fn new(
        model: String,
        temperature: f32,
        max_tokens: u32,
        client: crate::client::TextClient,
    ) -> Self {
        let call: CoachCall = std::sync::Arc::new(move |request| {
            let client = client.clone();
            Box::pin(async move { client.create_message(request).await })
        });
        Self {
            model,
            temperature,
            max_tokens,
            call,
        }
    }

    /// Build a coach execution with a custom call (tests).
    #[cfg(test)]
    #[must_use]
    pub fn with_call(model: String, temperature: f32, max_tokens: u32, call: CoachCall) -> Self {
        Self {
            model,
            temperature,
            max_tokens,
            call,
        }
    }

    async fn run(&self, prompt: String) -> anyhow::Result<String> {
        let request = crate::models::MessageRequest {
            model: self.model.clone(),
            messages: vec![crate::models::Message {
                role: "user".to_string(),
                content: vec![crate::models::ContentBlock::Text {
                    text: prompt,
                    cache_control: None,
                }],
            }],
            max_tokens: self.max_tokens,
            system: None,
            tools: None,
            tool_choice: None,
            metadata: None,
            thinking: None,
            stream: Some(false),
            temperature: Some(self.temperature),
            top_p: None,
        };
        let response = (self.call)(request).await?;
        Ok(response
            .content
            .iter()
            .filter_map(|block| match block {
                crate::models::ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

/// Initialize an autocoding session with requirements.
pub struct DuoInitTool {
    session: SharedDuoSession,
}

impl DuoInitTool {
    #[must_use]
    pub fn new(session: SharedDuoSession) -> Self {
        Self { session }
    }
}

#[async_trait]
impl ToolSpec for DuoInitTool {
    fn name(&self) -> &'static str {
        "duo_init"
    }

    fn description(&self) -> &'static str {
        "Initialize a Duo autocoding session with requirements. Returns session summary."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "requirements": {
                    "type": "string",
                    "description": "The requirements document (source of truth). Should be structured as a checklist."
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Maximum turns before timeout (default: 10)"
                },
                "session_name": {
                    "type": "string",
                    "description": "Optional human-readable session name (e.g., 'auth-feature')"
                },
                "approval_threshold": {
                    "type": "number",
                    "description": "Minimum compliance score for approval (0-1, default: 0.9)"
                }
            },
            "required": ["requirements"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadOnly]
    }

    fn approval_requirement(&self) -> ApprovalRequirement {
        ApprovalRequirement::Auto
    }

    async fn execute(&self, input: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let requirements = required_str(&input, "requirements")?;
        let max_turns = input
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let session_name = optional_str(&input, "session_name").map(str::to_string);
        let approval_threshold = input.get("approval_threshold").and_then(|v| v.as_f64());

        let mut session = self
            .session
            .lock()
            .map_err(|_| ToolError::execution_failed("Failed to lock Duo session"))?;

        let state = session.start_session(
            requirements.to_string(),
            session_name,
            max_turns,
            approval_threshold,
        );

        let summary = state.summary();
        Ok(ToolResult::success(format!(
            "Duo session initialized. Ready for player phase.\n\n{}",
            summary
        )))
    }
}

/// Generate the player prompt for implementation.
pub struct DuoPlayerTool {
    session: SharedDuoSession,
}

impl DuoPlayerTool {
    #[must_use]
    pub fn new(session: SharedDuoSession) -> Self {
        Self { session }
    }
}

#[async_trait]
impl ToolSpec for DuoPlayerTool {
    fn name(&self) -> &'static str {
        "duo_player"
    }

    fn description(&self) -> &'static str {
        "Generate the player prompt for implementation. Must be in Init or Player phase. Call after implementing to advance to Coach phase."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "implementation_summary": {
                    "type": "string",
                    "description": "Optional summary of implementation work done (recorded in history)"
                }
            }
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadOnly]
    }

    fn approval_requirement(&self) -> ApprovalRequirement {
        ApprovalRequirement::Auto
    }

    async fn execute(&self, input: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let implementation_summary = optional_str(&input, "implementation_summary")
            .map(str::to_string)
            .unwrap_or_else(|| "Implementation in progress".to_string());

        let mut session = self
            .session
            .lock()
            .map_err(|_| ToolError::execution_failed("Failed to lock Duo session"))?;

        let state = session
            .get_active_mut()
            .ok_or_else(|| ToolError::invalid_input("No active session. Call duo_init first."))?;

        // Check we're in a valid phase for player
        match state.phase {
            DuoPhase::Init | DuoPhase::Player => {
                // Generate prompt first
                let prompt = generate_player_prompt(state);

                // Advance to Coach phase
                state
                    .advance_to_coach(implementation_summary)
                    .map_err(|e| ToolError::execution_failed(e.to_string()))?;

                Ok(ToolResult::success(format!(
                    "=== PLAYER PROMPT ===\n\n{}\n\n---\nAdvanced to Coach phase. Use duo_coach for verification.",
                    prompt
                )))
            }
            DuoPhase::Coach => Err(ToolError::invalid_input(
                "Already in Coach phase. Use duo_coach to get verification prompt.",
            )),
            DuoPhase::Approved => Err(ToolError::invalid_input(
                "Session already approved. Start a new session with duo_init.",
            )),
            DuoPhase::Timeout => Err(ToolError::invalid_input(
                "Session timed out. Start a new session with duo_init.",
            )),
        }
    }
}

/// Generate the coach prompt for validation, and execute it against the
/// configured coach model when one is set.
pub struct DuoCoachTool {
    session: SharedDuoSession,
    coach: Option<CoachExecution>,
}

impl DuoCoachTool {
    #[must_use]
    pub fn new(session: SharedDuoSession, coach: Option<CoachExecution>) -> Self {
        Self { session, coach }
    }

    /// Read the files under review into `CoachFile` values, workspace-bounded
    /// and byte-capped. A path escape is a hard error; an unreadable file is
    /// reported inline so one bad path cannot kill the coach turn.
    fn collect_coach_files(
        files: &[&str],
        context: &ToolContext,
    ) -> Result<Vec<CoachFile>, ToolError> {
        let mut seen: Vec<String> = Vec::new();
        let mut collected: Vec<CoachFile> = Vec::new();
        let mut used = 0usize;

        for raw in files {
            let path = normalize_at_path(raw)?;
            if seen.contains(&path) {
                continue;
            }
            seen.push(path.clone());

            let resolved = context.resolve_path(&path)?;
            let remaining = MAX_COACH_TOTAL_BYTES.saturating_sub(used);
            if remaining == 0 {
                collected.push(CoachFile {
                    path,
                    content: String::new(),
                    note: Some("not inlined: coach file budget exhausted".to_string()),
                });
                continue;
            }

            let text = match std::fs::read_to_string(&resolved) {
                Ok(text) => text,
                Err(err) => {
                    collected.push(CoachFile {
                        path,
                        content: String::new(),
                        note: Some(format!("unreadable: {err}")),
                    });
                    continue;
                }
            };

            let cap = MAX_COACH_FILE_BYTES.min(remaining);
            let total_lines = text.lines().count();
            let mut content = String::new();
            let mut shown = 0usize;
            for line in text.lines() {
                if content.len() + line.len() + 1 > cap {
                    break;
                }
                content.push_str(line);
                content.push('\n');
                shown += 1;
            }
            used += content.len();

            let note = (shown < total_lines).then(|| {
                format!("truncated: showing {shown} of {total_lines} lines ({cap} byte cap)")
            });
            collected.push(CoachFile {
                path,
                content,
                note,
            });
        }

        Ok(collected)
    }
}

#[async_trait]
impl ToolSpec for DuoCoachTool {
    fn name(&self) -> &'static str {
        "duo_coach"
    }

    fn description(&self) -> &'static str {
        "Generate the coach prompt for validation and run it when a coach model is configured. Must be in Coach phase. Does NOT advance state. Pass files to inline the code under review as ground truth."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Files under review to inline into the coach prompt as ground truth (workspace-relative; '@' prefix allowed)"
                }
            }
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadOnly]
    }

    fn approval_requirement(&self) -> ApprovalRequirement {
        ApprovalRequirement::Auto
    }

    async fn execute(&self, input: Value, context: &ToolContext) -> Result<ToolResult, ToolError> {
        let files: Vec<&str> = input
            .get("files")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let coach_files = Self::collect_coach_files(&files, context)?;

        // Scope the Duo lock: build the prompt, then release the guard before
        // awaiting the coach model so the tool future stays Send.
        let prompt = {
            let session = self
                .session
                .lock()
                .map_err(|_| ToolError::execution_failed("Failed to lock Duo session"))?;

            let state = session.get_active().ok_or_else(|| {
                ToolError::invalid_input("No active session. Call duo_init first.")
            })?;

            if state.phase != DuoPhase::Coach {
                return Err(ToolError::invalid_input(format!(
                    "Expected Coach phase, but current phase is {}. Use duo_player first.",
                    state.phase
                )));
            }

            generate_coach_prompt(state, &coach_files)
        };

        let mut result = String::new();
        if !coach_files.is_empty() {
            let listed = coach_files
                .iter()
                .map(|file| match &file.note {
                    Some(note) => format!("{} ({note})", file.path),
                    None => file.path.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            result.push_str(&format!(
                "Inlined {} file(s) for verification: {listed}\n\n",
                coach_files.len()
            ));
        }

        if let Some(coach) = self.coach.as_ref() {
            let verdict = coach.run(prompt).await.map_err(|e| {
                ToolError::execution_failed(format!(
                    "Coach model '{}' call failed: {e}",
                    coach.model
                ))
            })?;
            result.push_str(&format!(
                "=== COACH VERDICT (model: {}) ===\n\n{}\n\n---\nRecord the outcome with duo_advance (feedback + approved).",
                coach.model, verdict
            ));
        } else {
            result.push_str(&format!(
                "=== COACH PROMPT ===\n\n{}\n\n---\nAfter verification, use duo_advance with feedback and approval status.",
                prompt
            ));
        }

        Ok(ToolResult::success(result))
    }
}

/// Advance the session after coach review.
pub struct DuoAdvanceTool {
    session: SharedDuoSession,
}

impl DuoAdvanceTool {
    #[must_use]
    pub fn new(session: SharedDuoSession) -> Self {
        Self { session }
    }
}

#[async_trait]
impl ToolSpec for DuoAdvanceTool {
    fn name(&self) -> &'static str {
        "duo_advance"
    }

    fn description(&self) -> &'static str {
        "Advance the session after coach review. Updates turn count and records feedback. Returns new status."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "feedback": {
                    "type": "string",
                    "description": "The coach's feedback text (compliance checklist and actions needed)"
                },
                "approved": {
                    "type": "boolean",
                    "description": "Whether the coach approved the implementation (look for 'COACH APPROVED')"
                },
                "compliance_score": {
                    "type": "number",
                    "description": "Optional compliance score (0-1) based on checklist items satisfied"
                }
            },
            "required": ["feedback", "approved"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadOnly]
    }

    fn approval_requirement(&self) -> ApprovalRequirement {
        ApprovalRequirement::Auto
    }

    async fn execute(&self, input: Value, _context: &ToolContext) -> Result<ToolResult, ToolError> {
        let feedback = required_str(&input, "feedback")?;
        let approved = input
            .get("approved")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| ToolError::missing_field("approved"))?;
        let compliance_score = input.get("compliance_score").and_then(|v| v.as_f64());

        let mut session = self
            .session
            .lock()
            .map_err(|_| ToolError::execution_failed("Failed to lock Duo session"))?;

        let state = session
            .get_active_mut()
            .ok_or_else(|| ToolError::invalid_input("No active session. Call duo_init first."))?;

        if state.phase != DuoPhase::Coach {
            return Err(ToolError::invalid_input(format!(
                "Expected Coach phase, but current phase is {}",
                state.phase
            )));
        }

        // Advance the turn
        state
            .advance_turn(feedback.to_string(), approved, compliance_score)
            .map_err(|e| ToolError::execution_failed(e.to_string()))?;

        // Determine status message based on new phase
        let status_msg = match state.phase {
            DuoPhase::Approved => "🎉 APPROVED! All requirements verified.",
            DuoPhase::Timeout => "⏰ TIMEOUT. Max turns reached without approval.",
            DuoPhase::Player => "🔄 Continuing to next player turn...",
            _ => "Session updated.",
        };

        let summary = state.summary();
        let mut result = ToolResult::success(format!("{}\n\n{}", status_msg, summary));
        result.metadata = Some(json!({
            "phase": state.phase.to_string(),
            "status": state.status.to_string(),
            "turn": state.current_turn,
            "max_turns": state.max_turns,
            "approved": approved,
            "compliance_score": compliance_score,
            "is_complete": state.is_complete(),
        }));

        Ok(result)
    }
}

/// Show the current session status.
pub struct DuoStatusTool {
    session: SharedDuoSession,
}

impl DuoStatusTool {
    #[must_use]
    pub fn new(session: SharedDuoSession) -> Self {
        Self { session }
    }
}

#[async_trait]
impl ToolSpec for DuoStatusTool {
    fn name(&self) -> &'static str {
        "duo_status"
    }

    fn description(&self) -> &'static str {
        "Show the current Duo session status including phase, turn count, and requirements."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadOnly]
    }

    fn approval_requirement(&self) -> ApprovalRequirement {
        ApprovalRequirement::Auto
    }

    async fn execute(
        &self,
        _input: Value,
        _context: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let session = self
            .session
            .lock()
            .map_err(|_| ToolError::execution_failed("Failed to lock Duo session"))?;

        Ok(ToolResult::success(session_summary(&session)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duo::new_shared_duo_session;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[test]
    fn test_duo_init_tool_schema() {
        let session = new_shared_duo_session();
        let tool = DuoInitTool::new(session);

        assert_eq!(tool.name(), "duo_init");
        assert_eq!(tool.approval_requirement(), ApprovalRequirement::Auto);

        let schema = tool.input_schema();
        assert!(schema.get("properties").is_some());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("requirements"))
        );
    }

    #[test]
    fn test_duo_player_tool_schema() {
        let session = new_shared_duo_session();
        let tool = DuoPlayerTool::new(session);

        assert_eq!(tool.name(), "duo_player");
        assert_eq!(tool.approval_requirement(), ApprovalRequirement::Auto);
    }

    #[test]
    fn test_duo_coach_tool_schema() {
        let session = new_shared_duo_session();
        let tool = DuoCoachTool::new(session, None);

        assert_eq!(tool.name(), "duo_coach");
        assert_eq!(tool.approval_requirement(), ApprovalRequirement::Auto);
    }

    #[test]
    fn test_duo_advance_tool_schema() {
        let session = new_shared_duo_session();
        let tool = DuoAdvanceTool::new(session);

        assert_eq!(tool.name(), "duo_advance");
        assert_eq!(tool.approval_requirement(), ApprovalRequirement::Auto);

        let schema = tool.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("feedback")));
        assert!(required.contains(&json!("approved")));
    }

    #[test]
    fn test_duo_status_tool_schema() {
        let session = new_shared_duo_session();
        let tool = DuoStatusTool::new(session);

        assert_eq!(tool.name(), "duo_status");
        assert_eq!(tool.approval_requirement(), ApprovalRequirement::Auto);
    }

    fn coach_phase_session() -> crate::duo::SharedDuoSession {
        let duo = new_shared_duo_session();
        {
            let mut session = duo.lock().unwrap();
            session.start_session("- [ ] add()".into(), None, None, None);
            session
                .get_active_mut()
                .unwrap()
                .advance_to_coach("done".into())
                .unwrap();
        }
        duo
    }

    #[tokio::test]
    async fn duo_coach_inlines_files_under_review() {
        let tmp = tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("impl.rs"),
            "fn add(a: i32, b: i32) -> i32 { a + b }\n",
        )
        .expect("write");
        let ctx = ToolContext::new(tmp.path().to_path_buf());

        let tool = DuoCoachTool::new(coach_phase_session(), None);
        let result = tool
            .execute(json!({"files": ["@impl.rs"]}), &ctx)
            .await
            .expect("coach execution");

        assert!(
            result
                .content
                .contains("Inlined 1 file(s) for verification: impl.rs")
        );
        assert!(result.content.contains("### impl.rs"));
        assert!(
            result
                .content
                .contains("    1 | fn add(a: i32, b: i32) -> i32 { a + b }")
        );
        assert!(!result.content.to_lowercase().contains("rlm"));
    }

    #[tokio::test]
    async fn duo_coach_missing_file_reports_unreadable_note() {
        let tmp = tempdir().expect("tempdir");
        let ctx = ToolContext::new(tmp.path().to_path_buf());
        let tool = DuoCoachTool::new(coach_phase_session(), None);
        let result = tool
            .execute(json!({"files": ["@nope.rs"]}), &ctx)
            .await
            .expect("missing file should not fail tool execution");

        assert!(result.content.contains("nope.rs (unreadable:"));
        assert!(result.content.contains("### nope.rs"));
        assert!(result.content.contains("_unreadable:"));
    }

    #[tokio::test]
    async fn duo_coach_truncates_large_files() {
        let tmp = tempdir().expect("tempdir");
        let big_content = (0..4000)
            .map(|_| "x".repeat(40))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(tmp.path().join("big.rs"), &big_content).expect("write");
        let ctx = ToolContext::new(tmp.path().to_path_buf());

        let tool = DuoCoachTool::new(coach_phase_session(), None);
        let result = tool
            .execute(json!({"files": ["@big.rs"]}), &ctx)
            .await
            .expect("coach execution");

        assert!(result.content.contains("truncated: showing "));
        assert!(result.content.contains(" of 4000 lines"));
    }

    #[tokio::test]
    async fn duo_coach_without_files_still_returns_prompt() {
        let tmp = tempdir().expect("tempdir");
        let ctx = ToolContext::new(tmp.path().to_path_buf());
        let duo = coach_phase_session();

        let tool = DuoCoachTool::new(duo, None);
        let result = tool
            .execute(json!({}), &ctx)
            .await
            .expect("coach without files succeeds");

        assert!(result.content.contains("=== COACH PROMPT ==="));
        assert!(!result.content.contains("Files Under Review"));
    }

    fn canned_response(text: &str) -> crate::models::MessageResponse {
        crate::models::MessageResponse {
            id: "test".to_string(),
            r#type: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![crate::models::ContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
            model: "coach-model".to_string(),
            stop_reason: None,
            stop_sequence: None,
            usage: crate::models::Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        }
    }

    #[tokio::test]
    async fn duo_coach_executes_coach_model_when_set() {
        let tmp = tempdir().expect("tempdir");
        let ctx = ToolContext::new(tmp.path().to_path_buf());
        let duo = coach_phase_session();

        let seen: Arc<Mutex<Option<crate::models::MessageRequest>>> = Arc::new(Mutex::new(None));
        let seen_capture = seen.clone();
        let call: CoachCall = Arc::new(move |request| {
            let seen_capture = seen_capture.clone();
            Box::pin(async move {
                *seen_capture.lock().unwrap() = Some(request);
                Ok(canned_response("COACH APPROVED\ncompliance 1.0"))
            })
        });
        let coach = CoachExecution::with_call("coach-model".to_string(), 0.3, 512, call);

        let tool = DuoCoachTool::new(duo, Some(coach));
        let result = tool
            .execute(json!({}), &ctx)
            .await
            .expect("coach execution succeeds");

        assert!(
            result
                .content
                .contains("=== COACH VERDICT (model: coach-model) ===")
        );
        assert!(result.content.contains("COACH APPROVED"));
        assert!(!result.content.contains("=== COACH PROMPT ==="));

        let request = seen.lock().unwrap().take().expect("request captured");
        assert_eq!(request.model, "coach-model");
        assert_eq!(request.temperature, Some(0.3));
        assert_eq!(request.max_tokens, 512);
        assert!(request.messages[0]
            .content
            .iter()
            .any(|block| matches!(block, crate::models::ContentBlock::Text { text, .. } if text.contains("Coach Phase"))));
    }

    #[tokio::test]
    async fn duo_coach_coach_model_failure_reports_error() {
        let tmp = tempdir().expect("tempdir");
        let ctx = ToolContext::new(tmp.path().to_path_buf());
        let duo = coach_phase_session();

        let call: CoachCall =
            Arc::new(|_request| Box::pin(async { Err(anyhow::anyhow!("network down")) }));
        let coach = CoachExecution::with_call("coach-model".to_string(), 0.3, 512, call);

        let tool = DuoCoachTool::new(duo, Some(coach));
        let err = tool
            .execute(json!({}), &ctx)
            .await
            .expect_err("coach model failure must surface");

        assert!(
            err.to_string()
                .contains("Coach model 'coach-model' call failed: network down"),
            "unexpected error: {err}"
        );
    }
}
