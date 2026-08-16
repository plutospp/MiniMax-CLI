//! Core commands: help, clear, exit, model

use std::fmt::Write;

use crate::tools::plan::PlanState;
use crate::tui::app::{App, AppAction};
use crate::tui::views::{HelpView, ModalKind};

use super::CommandResult;

/// Show help information
pub fn help(app: &mut App, topic: Option<&str>) -> CommandResult {
    if let Some(topic) = topic {
        // Show help for specific command
        if let Some(cmd) = super::get_command_info(topic) {
            let mut help = format!(
                "{}\n\n  {}\n\n  Usage: {}",
                cmd.name, cmd.description, cmd.usage
            );
            if !cmd.aliases.is_empty() {
                let _ = write!(help, "\n  Aliases: {}", cmd.aliases.join(", "));
            }
            return CommandResult::message(help);
        }
        return CommandResult::error(format!("Unknown command: {topic}"));
    }

    // Show help overlay
    if app.view_stack.top_kind() != Some(ModalKind::Help) {
        app.view_stack.push(HelpView::new());
    }
    CommandResult::ok()
}

/// Clear conversation history
pub fn clear(app: &mut App) -> CommandResult {
    app.history.clear();
    app.mark_history_updated();
    app.api_messages.clear();
    app.transcript_selection.clear();
    app.total_conversation_tokens = 0;
    app.clear_todos();
    if let Ok(mut plan) = app.plan_state.lock() {
        *plan = PlanState::default();
    }
    app.tool_log.clear();
    CommandResult::message("Conversation cleared")
}

/// Exit the application
pub fn exit() -> CommandResult {
    CommandResult::action(AppAction::Quit)
}

use crate::settings::Settings;
use crate::tui::model_picker::validate_model;

/// Switch or view current model
///
/// When called without arguments, opens an interactive model picker.
/// When called with an argument, validates and sets the model directly.
pub fn model(app: &mut App, model_name: Option<&str>) -> CommandResult {
    if let Some(name) = model_name {
        // Known MiniMax catalog models, or free-form for other providers.
        let (new_model, description) = if let Some(model_info) = validate_model(name) {
            (
                model_info.id.to_string(),
                model_info.description.to_string(),
            )
        } else if app.provider != "minimax" {
            let trimmed = name.trim().to_string();
            if trimmed.is_empty() {
                return CommandResult::error("Model name cannot be empty".to_string());
            }
            (
                trimmed,
                format!("Custom model for provider '{}'", app.provider),
            )
        } else {
            let available = crate::tui::model_picker::AVAILABLE_MODELS
                .iter()
                .map(|m| format!("  • {} - {}", m.id, m.name))
                .collect::<Vec<_>>()
                .join("\n");
            return CommandResult::error(format!(
                "Unknown model: '{name}'\n\nAvailable models:\n{available}"
            ));
        };

        let old_model = app.model.clone();
        app.model = new_model.clone();

        let mut settings = Settings::load().unwrap_or_default();
        settings.default_model = Some(new_model.clone());
        if let Err(e) = settings.save() {
            return CommandResult::message(format!(
                "Model changed: {old_model} → {new_model} (failed to save: {e})"
            ));
        }

        CommandResult::message(format!(
            "Model changed: {old_model} → {new_model} (saved)\n\n{description}"
        ))
    } else {
        CommandResult::action(crate::tui::app::AppAction::OpenModelPicker)
    }
}

/// Switch or view the active LLM provider.
pub fn provider(_app: &mut App, name: Option<&str>) -> CommandResult {
    if let Some(name) = name {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return CommandResult::error("Provider name cannot be empty");
        }
        CommandResult::with_message_and_action(
            format!("Switching provider to {trimmed}..."),
            AppAction::SwitchProvider {
                name: trimmed.to_string(),
            },
        )
    } else {
        CommandResult::action(AppAction::OpenProviderPicker)
    }
}

/// Set or view the Duo coach model.
///
/// - `/coach <model>` — coach runs on this model (active provider must serve it)
/// - `/coach off` — coach runs on the active model (same as the player)
/// - `/coach` — show the current setting
pub fn coach(app: &mut App, arg: Option<&str>) -> CommandResult {
    let Some(arg) = arg else {
        return CommandResult::message(match app.coach_model.as_deref() {
            Some(model) => format!(
                "Coach model: {model} (player uses the active model: {})\n\n/coach <model> to change | /coach off to reset",
                app.model
            ),
            None => format!(
                "Coach model: (active model — same as player)\nActive model: {}\n\n/coach <model> to set a separate coach model",
                app.model
            ),
        });
    };

    let trimmed = arg.trim();
    if trimmed.is_empty() {
        return CommandResult::error("Usage: /coach [model|off]");
    }

    let model = if trimmed.eq_ignore_ascii_case("off")
        || trimmed.eq_ignore_ascii_case("clear")
        || trimmed.eq_ignore_ascii_case("none")
    {
        None
    } else {
        Some(trimmed.to_string())
    };

    CommandResult::action(AppAction::SetCoachModel { model })
}

/// Manage sub-agent status from the engine.
///
/// Usage:
/// - `/subagents` or `/subagents list`
/// - `/subagents show <agent_id>`
/// - `/subagents wait <agent_id> [timeout_ms]`
/// - `/subagents cancel <agent_id>`
/// - `/subagents clean [max_age_ms]`
pub fn subagents(_app: &mut App, arg: Option<&str>) -> CommandResult {
    let mut parts = arg.unwrap_or("list").split_whitespace();
    let subcommand = parts.next().unwrap_or("list").to_lowercase();

    match subcommand.as_str() {
        "list" => CommandResult::with_message_and_action(
            "Fetching sub-agent status...",
            AppAction::ListSubAgents,
        ),
        "show" | "get" => {
            let Some(agent_id) = parts.next() else {
                return CommandResult::error("Usage: /subagents show <agent_id>");
            };
            CommandResult::with_message_and_action(
                format!("Fetching sub-agent {agent_id}..."),
                AppAction::GetSubAgent {
                    agent_id: agent_id.to_string(),
                    block: false,
                    timeout_ms: 1_000,
                },
            )
        }
        "wait" => {
            let Some(agent_id) = parts.next() else {
                return CommandResult::error("Usage: /subagents wait <agent_id> [timeout_ms]");
            };
            let timeout_ms = parts
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(120_000)
                .clamp(1_000, 600_000);

            CommandResult::with_message_and_action(
                format!("Waiting for sub-agent {agent_id}..."),
                AppAction::GetSubAgent {
                    agent_id: agent_id.to_string(),
                    block: true,
                    timeout_ms,
                },
            )
        }
        "cancel" | "kill" => {
            let Some(agent_id) = parts.next() else {
                return CommandResult::error("Usage: /subagents cancel <agent_id>");
            };
            CommandResult::with_message_and_action(
                format!("Cancelling sub-agent {agent_id}..."),
                AppAction::CancelSubAgent {
                    agent_id: agent_id.to_string(),
                },
            )
        }
        "clean" => {
            let max_age_ms = parts
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(300_000)
                .min(86_400_000);
            CommandResult::with_message_and_action(
                "Cleaning completed sub-agents...",
                AppAction::CleanSubAgents { max_age_ms },
            )
        }
        _ => CommandResult::error(
            "Usage: /subagents [list|show <agent_id>|wait <agent_id> [timeout_ms]|cancel <agent_id>|clean [max_age_ms]]",
        ),
    }
}

/// Show `MiniMax` dashboard and docs links
pub fn minimax_links() -> CommandResult {
    CommandResult::message(
        "MiniMax Links:\n\
─────────────────────────────\n\
Dashboard: https://platform.minimax.io\n\
Docs:      https://platform.minimax.io/docs\n\n\
Tip: API keys are available in the dashboard console.",
    )
}

/// Copy last assistant message (or Nth message) to clipboard
pub fn copy(app: &mut App, arg: Option<&str>) -> CommandResult {
    use crate::tui::history::HistoryCell;

    if app.history.is_empty() {
        return CommandResult::error("No messages to copy");
    }

    let content = if let Some(n_str) = arg {
        // Copy specific message by 1-indexed number
        match n_str.parse::<usize>() {
            Ok(n) if n >= 1 && n <= app.history.len() => {
                extract_text_from_cell(&app.history[n - 1])
            }
            Ok(n) => {
                return CommandResult::error(format!(
                    "Message {n} out of range (1-{})",
                    app.history.len()
                ));
            }
            Err(_) => {
                return CommandResult::error("Usage: /copy [n]  — n is a message number");
            }
        }
    } else {
        // No arg: copy last assistant message
        app.history
            .iter()
            .rev()
            .find_map(|cell| {
                if let HistoryCell::Assistant { content, .. } = cell {
                    Some(content.clone())
                } else {
                    None
                }
            })
            .ok_or(())
            .unwrap_or_default()
    };

    if content.is_empty() {
        return CommandResult::error("No assistant message to copy");
    }

    match app.clipboard.write_text(&content) {
        Ok(()) => CommandResult::message("Copied to clipboard ✓"),
        Err(e) => CommandResult::error(format!("Failed to copy: {e}")),
    }
}

fn extract_text_from_cell(cell: &crate::tui::history::HistoryCell) -> String {
    use crate::tui::history::HistoryCell;
    match cell {
        HistoryCell::User { content }
        | HistoryCell::Assistant { content, .. }
        | HistoryCell::System { content } => content.clone(),
        HistoryCell::ThinkingSummary { summary } => summary.clone(),
        HistoryCell::Error { message, .. } => message.clone(),
        HistoryCell::Tool(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tui::app::TuiOptions;
    use std::path::PathBuf;

    fn create_test_app() -> App {
        let options = TuiOptions {
            model: "test-model".to_string(),
            workspace: PathBuf::from("."),
            allow_shell: false,
            max_subagents: 1,
            skills_dir: PathBuf::from("."),
            memory_path: PathBuf::from("memory.md"),
            notes_path: PathBuf::from("notes.txt"),
            mcp_config_path: PathBuf::from("mcp.json"),
            use_memory: false,
            start_in_agent_mode: false,
            yolo: false,
            resume_session_id: None,
        };
        App::new(options, &Config::default())
    }

    #[test]
    fn subagents_wait_parses_action() {
        let mut app = create_test_app();
        let result = subagents(&mut app, Some("wait agent-123 5000"));
        match result.action {
            Some(AppAction::GetSubAgent {
                agent_id,
                block,
                timeout_ms,
            }) => {
                assert_eq!(agent_id, "agent-123");
                assert!(block);
                assert_eq!(timeout_ms, 5000);
            }
            other => panic!("expected GetSubAgent action, got {other:?}"),
        }
    }

    #[test]
    fn subagents_wait_requires_id() {
        let mut app = create_test_app();
        let result = subagents(&mut app, Some("wait"));
        assert!(
            result
                .message
                .unwrap_or_default()
                .contains("Usage: /subagents wait"),
        );
    }

    #[test]
    fn coach_sets_and_clears_model() {
        let mut app = create_test_app();

        let result = coach(&mut app, Some("verifier-model"));
        assert!(matches!(
            result.action,
            Some(AppAction::SetCoachModel { ref model }) if model.as_deref() == Some("verifier-model")
        ));

        let result = coach(&mut app, Some("off"));
        assert!(matches!(
            result.action,
            Some(AppAction::SetCoachModel { ref model }) if model.is_none()
        ));
    }

    #[test]
    fn coach_shows_current_setting() {
        let mut app = create_test_app();
        let result = coach(&mut app, None);
        assert!(result.message.unwrap_or_default().contains("active model"));

        app.coach_model = Some("verifier-model".to_string());
        let result = coach(&mut app, None);
        assert!(
            result
                .message
                .unwrap_or_default()
                .contains("verifier-model")
        );
    }
}
