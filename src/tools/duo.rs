//! Tools for Duo mode: Player-Coach autocoding workflow.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::duo::{
    DuoPhase, SharedDuoSession, generate_coach_prompt, generate_player_prompt, session_summary,
};
use crate::rlm::{SharedRlmSession, context_id_from_path, unique_context_id};
use crate::tools::rlm::normalize_load_path;
use crate::tools::spec::{
    ApprovalRequirement, ToolCapability, ToolContext, ToolError, ToolResult, ToolSpec,
    optional_str, required_str,
};

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
            Box::pin(async move {
                client.create_message(request).await
            })
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
    rlm_session: Option<SharedRlmSession>,
    coach: Option<CoachExecution>,
}

impl DuoCoachTool {
    #[must_use]
    pub fn new(
        session: SharedDuoSession,
        rlm_session: Option<SharedRlmSession>,
        coach: Option<CoachExecution>,
    ) -> Self {
        Self {
            session,
            rlm_session,
            coach,
        }
    }

    /// Load verification files into the RLM context store, mirroring
    /// `RlmLoadTool` id/reuse semantics. Returns the loaded context ids.
    fn preload_verification_contexts(
        &self,
        files: &[&str],
        context: &ToolContext,
    ) -> Result<Vec<String>, ToolError> {
        let rlm_session = self.rlm_session.as_ref().ok_or_else(|| {
            ToolError::invalid_input(
                "RLM verification is unavailable: enable [features] rlm to preload coach verification contexts",
            )
        })?;

        let mut loaded = Vec::new();
        let mut session = rlm_session
            .lock()
            .map_err(|_| ToolError::execution_failed("Failed to lock RLM session"))?;
        for raw in files {
            let normalized = normalize_load_path(raw)?;
            let resolved = context.resolve_path(&normalized)?;
            let base_id = context_id_from_path(&resolved);
            let id = if session.contexts.contains_key(&base_id) {
                base_id
            } else {
                unique_context_id(&session, &base_id)
            };
            session.load_file(&id, &resolved).map_err(|e| {
                ToolError::execution_failed(format!(
                    "Failed to load '{}' for verification: {e}",
                    resolved.display()
                ))
            })?;
            loaded.push(id);
        }
        Ok(loaded)
    }
}

#[async_trait]
impl ToolSpec for DuoCoachTool {
    fn name(&self) -> &'static str {
        "duo_coach"
    }

    fn description(&self) -> &'static str {
        "Generate the coach prompt for validation. Must be in Coach phase. Does NOT advance state. Pass files to load them into the RLM context store for ground-truth verification."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Files under review to load into the RLM context store for verification (prefix with @ for workspace-relative paths)"
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

        // Preload (and collect) verification contexts before locking the Duo
        // session so at most one shared session is held at a time.
        let mut loaded_ids = Vec::new();
        let mut context_ids = Vec::new();
        if let Some(rlm_session) = self.rlm_session.as_ref() {
            if !files.is_empty() {
                loaded_ids = self.preload_verification_contexts(&files, context)?;
            }
            if let Ok(session) = rlm_session.lock() {
                context_ids = session.contexts.keys().cloned().collect::<Vec<_>>();
            }
        } else if !files.is_empty() {
            return Err(ToolError::invalid_input(
                "RLM verification is unavailable: enable [features] rlm to preload coach verification contexts",
            ));
        }
        context_ids.sort();
        context_ids.dedup();

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

            generate_coach_prompt(
                state,
                self.rlm_session.as_ref().map(|_| context_ids.as_slice()),
            )
        };

        let mut result = String::new();
        if !loaded_ids.is_empty() {
            result.push_str(&format!(
                "Loaded {} verification context(s): {}\n\n",
                loaded_ids.len(),
                loaded_ids.join(", ")
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
    use crate::rlm::RlmSession;
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

    fn test_duo_coach_tool_schema() {
        let session = new_shared_duo_session();
        let tool = DuoCoachTool::new(session, None, None);

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
    async fn duo_coach_preloads_verification_contexts() {
        let tmp = tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("impl.rs"),
            "fn add(a: i32, b: i32) -> i32 { a + b }",
        )
        .expect("write");
        let ctx = ToolContext::new(tmp.path().to_path_buf());
        let duo = coach_phase_session();
        let rlm = Arc::new(Mutex::new(RlmSession::default()));

        let tool = DuoCoachTool::new(duo, Some(rlm.clone()), None);
        let result = tool
            .execute(json!({"files": ["@impl.rs"]}), &ctx)
            .await
            .expect("coach execution");

        assert!(
            result
                .content
                .contains("Loaded 1 verification context(s): impl.rs")
        );
        assert!(result.content.contains("Ground-Truth Verification (RLM)"));
        assert!(rlm.lock().unwrap().contexts.contains_key("impl.rs"));
    }

    #[tokio::test]
    async fn duo_coach_rejects_files_without_rlm() {
        let tmp = tempdir().expect("tempdir");
        let ctx = ToolContext::new(tmp.path().to_path_buf());
        let duo = coach_phase_session();

        let tool = DuoCoachTool::new(duo, None, None);
        let err = tool
            .execute(json!({"files": ["@impl.rs"]}), &ctx)
            .await
            .expect_err("files without RLM must fail");

        assert!(err.to_string().contains("RLM verification is unavailable"));
    }

    #[tokio::test]
    async fn duo_coach_without_files_still_returns_prompt() {
        let tmp = tempdir().expect("tempdir");
        let ctx = ToolContext::new(tmp.path().to_path_buf());
        let duo = coach_phase_session();

        let tool = DuoCoachTool::new(duo, None, None);
        let result = tool
            .execute(json!({}), &ctx)
            .await
            .expect("coach without files succeeds");

        assert!(result.content.contains("=== COACH PROMPT ==="));
        assert!(!result.content.contains("Ground-Truth Verification"));
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

        let tool = DuoCoachTool::new(duo, None, Some(coach));
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

        let tool = DuoCoachTool::new(duo, None, Some(coach));
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
