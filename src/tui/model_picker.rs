//! Interactive model picker for switching between chat models.
//!
//! The picker renders whatever model list it is constructed with: models
//! auto-discovered from the active provider's API (see
//! [`crate::model_discovery`]) or the built-in MiniMax catalog as a fallback.

use crate::palette;
use crate::tui::views::{ModalKind, ModalView, ViewAction, ViewEvent};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Widget,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

/// Information about a MiniMax model
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub capabilities: &'static str,
}

/// Available MiniMax models
pub const AVAILABLE_MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "MiniMax-M3",
        name: "MiniMax M3",
        description: "Frontier coding & agentic model with multimodal support and 1M context",
        capabilities: "Coding, agents, multimodal, long context",
    },
    ModelInfo {
        id: "MiniMax-M2.7",
        name: "MiniMax M2.7",
        description: "Recursive self-improvement model for engineering, office, and interaction",
        capabilities: "Text, reasoning, agents, office productivity, code",
    },
    ModelInfo {
        id: "MiniMax-M2.7-highspeed",
        name: "MiniMax M2.7 Highspeed",
        description: "Same performance as M2.7 with significantly faster inference",
        capabilities: "Fast text, reasoning, agents, code",
    },
    ModelInfo {
        id: "MiniMax-M2.5",
        name: "MiniMax 2.5",
        description: "Enhanced reasoning, tool calling, and long-context generation",
        capabilities: "Text, reasoning, agents, office productivity, code",
    },
    ModelInfo {
        id: "MiniMax-M2.5-lightning",
        name: "MiniMax 2.5 Lightning",
        description: "Fast version of M2.5 for quick responses with same capabilities",
        capabilities: "Fast text, reasoning, agents, code",
    },
    ModelInfo {
        id: "MiniMax-M2.1",
        name: "MiniMax M2.1",
        description: "Polyglot programming mastery with precision code refactoring",
        capabilities: "Text generation, reasoning, analysis, code",
    },
    ModelInfo {
        id: "MiniMax-M2",
        name: "MiniMax M2",
        description: "Efficient agentic model for coding (10B active params, 230B total)",
        capabilities: "Code generation, agents, tool use",
    },
    ModelInfo {
        id: "MiniMax-Text-01",
        name: "MiniMax Text 01",
        description: "Text-optimized model for natural language tasks (256K context)",
        capabilities: "Text generation, summarization, Q&A",
    },
    ModelInfo {
        id: "MiniMax-Coding-01",
        name: "MiniMax Coding 01",
        description: "Code-specialized model for programming tasks (128K context)",
        capabilities: "Code generation, debugging, review",
    },
];

/// One selectable row in the model picker.
///
/// `description` and `capabilities` are optional presentation lines; entries
/// discovered from a provider API omit them to keep long lists compact.
#[derive(Debug, Clone)]
pub struct PickerModel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub capabilities: String,
}

impl PickerModel {
    /// Build a compact entry for a model discovered from a provider API.
    #[must_use]
    pub fn discovered(id: String, display_name: Option<String>) -> Self {
        let name = display_name
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| id.clone());
        Self {
            id,
            name,
            description: String::new(),
            capabilities: String::new(),
        }
    }
}

/// Fallback picker entries built from the built-in MiniMax catalog.
#[must_use]
pub fn builtin_models() -> Vec<PickerModel> {
    AVAILABLE_MODELS
        .iter()
        .map(|m| PickerModel {
            id: m.id.to_string(),
            name: m.name.to_string(),
            description: m.description.to_string(),
            capabilities: m.capabilities.to_string(),
        })
        .collect()
}

/// Result of a model selection
#[derive(Debug, Clone)]
pub enum ModelPickerResult {
    /// User selected a model
    Selected(String),
    /// User cancelled
    Cancelled,
}

/// Interactive picker for selecting a model
pub struct ModelPicker {
    /// Currently selected index
    selected: usize,
    /// ID of the currently active model (to highlight)
    current_model: String,
    /// Models offered for selection (discovered or built-in fallback)
    models: Vec<PickerModel>,
    /// Where the model list came from, shown in the footer
    source: String,
}

impl ModelPicker {
    /// Create a new model picker over the given model list.
    pub fn new(current_model: String, models: Vec<PickerModel>, source: String) -> Self {
        let selected = models
            .iter()
            .position(|m| m.id == current_model)
            .unwrap_or(0);

        Self {
            selected,
            current_model,
            models,
            source,
        }
    }

    /// Get the currently selected model ID
    pub fn selected_model_id(&self) -> Option<String> {
        self.models.get(self.selected).map(|m| m.id.clone())
    }

    /// Check if a model is the currently active one
    fn is_current_model(&self, id: &str) -> bool {
        self.current_model == id
    }

    /// Move selection up
    fn select_up(&mut self) {
        if self.models.is_empty() {
            self.selected = 0;
        } else if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = self.models.len() - 1;
        }
    }

    /// Move selection down
    fn select_down(&mut self) {
        if self.models.is_empty() {
            self.selected = 0;
        } else if self.selected < self.models.len() - 1 {
            self.selected += 1;
        } else {
            self.selected = 0;
        }
    }

    /// Render a model item
    fn render_model_item(&self, model: &PickerModel, index: usize) -> ListItem<'_> {
        let is_selected = index == self.selected;
        let is_current = self.is_current_model(&model.id);

        // Selection style
        let base_style = if is_selected {
            Style::default()
                .bg(palette::MINIMAX_BLUE)
                .fg(palette::MINIMAX_SNOW)
                .add_modifier(Modifier::BOLD)
        } else if is_current {
            Style::default()
                .fg(palette::MINIMAX_ORANGE)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette::TEXT_PRIMARY)
        };

        // Current indicator
        let current_indicator = if is_current { " ● " } else { "   " };

        let mut lines = vec![];

        // Title line with model name and current indicator
        let title_style = if is_selected {
            Style::default()
                .bg(palette::MINIMAX_BLUE)
                .fg(palette::MINIMAX_SNOW)
                .add_modifier(Modifier::BOLD)
        } else if is_current {
            Style::default()
                .fg(palette::MINIMAX_ORANGE)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(palette::TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD)
        };

        let mut title_line = Line::from(vec![
            Span::styled(
                current_indicator,
                if is_current && !is_selected {
                    Style::default()
                        .fg(palette::MINIMAX_ORANGE)
                        .add_modifier(Modifier::BOLD)
                } else {
                    base_style
                },
            ),
            Span::styled(model.name.clone(), title_style),
        ]);

        if is_current {
            title_line.push_span(Span::styled(
                " (current)",
                if is_selected {
                    Style::default()
                        .bg(palette::MINIMAX_BLUE)
                        .fg(palette::MINIMAX_ORANGE)
                } else {
                    Style::default().fg(palette::TEXT_DIM)
                },
            ));
        }
        lines.push(title_line);

        // Description line
        if !model.description.is_empty() {
            let desc_style = if is_selected {
                Style::default()
                    .bg(palette::MINIMAX_BLUE)
                    .fg(palette::MINIMAX_SILVER)
            } else {
                Style::default().fg(palette::TEXT_DIM)
            };
            lines.push(Line::from(vec![
                Span::styled("     ", base_style),
                Span::styled(model.description.clone(), desc_style),
            ]));
        }

        // Capabilities line
        if !model.capabilities.is_empty() {
            let caps_style = if is_selected {
                Style::default()
                    .bg(palette::MINIMAX_BLUE)
                    .fg(palette::MINIMAX_SILVER)
            } else {
                Style::default().fg(palette::TEXT_MUTED)
            };
            lines.push(Line::from(vec![
                Span::styled("     ", base_style),
                Span::styled(format!("Capabilities: {}", model.capabilities), caps_style),
            ]));
        }

        // Spacing between items
        lines.push(Line::from(""));

        ListItem::new(lines)
    }
}

impl ModalView for ModelPicker {
    fn kind(&self) -> ModalKind {
        ModalKind::ModelPicker
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn handle_key(&mut self, key: KeyEvent) -> ViewAction {
        match key.code {
            KeyCode::Esc => ViewAction::EmitAndClose(ViewEvent::ModelPickerResult {
                result: ModelPickerResult::Cancelled,
            }),
            KeyCode::Enter => {
                if let Some(id) = self.selected_model_id() {
                    ViewAction::EmitAndClose(ViewEvent::ModelPickerResult {
                        result: ModelPickerResult::Selected(id),
                    })
                } else {
                    ViewAction::Close
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.select_up();
                ViewAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.select_down();
                ViewAction::None
            }
            _ => ViewAction::None,
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        // Create a centered popup sized for the list, capped to the screen
        let item_height: u16 = self
            .models
            .iter()
            .map(|m| {
                1 + u16::from(!m.description.is_empty()) + u16::from(!m.capabilities.is_empty()) + 1
            })
            .sum();
        let popup_width = (area.width * 3 / 5).clamp(50, 70);
        let popup_height = (item_height + 6).min(area.height.saturating_sub(4).max(6));
        let popup_x = (area.width - popup_width) / 2;
        let popup_y = (area.height - popup_height) / 2;
        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        // Clear the background
        Clear.render(popup_area, buf);

        // Draw the border
        let block = Block::default()
            .title(" Model Selection ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette::MINIMAX_BLUE));
        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        if self.models.is_empty() {
            let empty = Paragraph::new(Line::from(vec![Span::styled(
                format!("No models found ({})", self.source),
                Style::default().fg(palette::TEXT_DIM),
            )]));
            empty.render(chunks[0], buf);
        } else {
            // Model list; ListState keeps the selected row visible when the
            // list is longer than the popup (common with provider discovery).
            let items: Vec<ListItem> = self
                .models
                .iter()
                .enumerate()
                .map(|(i, m)| self.render_model_item(m, i))
                .collect();

            let mut state = ListState::default();
            state.select(Some(self.selected));
            let models_list = List::new(items);
            ratatui::widgets::StatefulWidget::render(&models_list, chunks[0], buf, &mut state);
        }

        // Help footer
        let help_text = format!(
            "↑/↓ to navigate | Enter to select | Esc to cancel | {} models · {}",
            self.models.len(),
            self.source
        );
        let help = Paragraph::new(Line::from(vec![Span::styled(
            help_text,
            Style::default().fg(palette::TEXT_DIM),
        )]));
        help.render(chunks[1], buf);
    }
}

/// Validate a model name against available models
pub fn validate_model(model_name: &str) -> Option<&'static ModelInfo> {
    let normalized = model_name.trim().to_ascii_lowercase();
    let canonical = match normalized.as_str() {
        "minimax-m3" | "m3" => "MiniMax-M3",
        "minimax-m2.7" | "minimax-2.7" | "m2.7" => "MiniMax-M2.7",
        "minimax-m2.7-highspeed" | "minimax-2.7-highspeed" | "m2.7-highspeed" | "2.7-highspeed" => {
            "MiniMax-M2.7-highspeed"
        }
        "minimax-2.5" | "minimax-m2.5" | "m2.5" => "MiniMax-M2.5",
        "minimax-2.5-lightning" | "minimax-m2.5-lightning" | "m2.5-lightning" | "2.5-lightning" => {
            "MiniMax-M2.5-lightning"
        }
        "minimax-m2" | "m2" => "MiniMax-M2",
        _ => model_name,
    };

    AVAILABLE_MODELS
        .iter()
        .find(|m| m.id.eq_ignore_ascii_case(canonical) || m.name.eq_ignore_ascii_case(canonical))
}

/// Get the canonical model ID for a model name
#[allow(dead_code)]
pub fn resolve_model_id(model_name: &str) -> Option<String> {
    validate_model(model_name).map(|m| m.id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_model_exact_match() {
        let model = validate_model("MiniMax-M3");
        assert!(model.is_some());
        assert_eq!(model.unwrap().id, "MiniMax-M3");
    }

    #[test]
    fn test_validate_model_case_insensitive() {
        let model = validate_model("minimax-m2.7");
        assert!(model.is_some());
        assert_eq!(model.unwrap().id, "MiniMax-M2.7");
    }

    #[test]
    fn test_validate_model_alias() {
        let model = validate_model("m2.7-highspeed");
        assert!(model.is_some());
        assert_eq!(model.unwrap().id, "MiniMax-M2.7-highspeed");
    }

    #[test]
    fn test_validate_model_not_found() {
        let model = validate_model("NonExistent-Model");
        assert!(model.is_none());
    }

    #[test]
    fn test_resolve_model_id() {
        assert_eq!(
            resolve_model_id("MiniMax-2.5"),
            Some("MiniMax-M2.5".to_string())
        );
        assert_eq!(resolve_model_id("m3"), Some("MiniMax-M3".to_string()));
        assert_eq!(
            resolve_model_id("minimax-text-01"),
            Some("MiniMax-Text-01".to_string())
        );
    }

    #[test]
    fn test_model_picker_navigation_builtin() {
        let models = builtin_models();
        let last_index = models.len() - 1;
        let mut picker = ModelPicker::new("MiniMax-M3".to_string(), models, "built-in".into());
        assert_eq!(picker.selected, 0);

        // Move down through the full list, then wrap to top.
        for expected in 1..=last_index {
            picker.select_down();
            assert_eq!(picker.selected, expected);
        }
        picker.select_down();
        assert_eq!(picker.selected, 0);

        // Move up from top and wrap to last.
        picker.select_up();
        assert_eq!(picker.selected, last_index);
    }

    #[test]
    fn test_model_picker_preselects_current_discovered_model() {
        let models = vec![
            PickerModel::discovered("alpha".into(), None),
            PickerModel::discovered("beta".into(), Some("Beta Model".into())),
            PickerModel::discovered("gamma".into(), None),
        ];
        let picker = ModelPicker::new("beta".to_string(), models, "test-openai API".into());
        assert_eq!(picker.selected, 1);
        assert_eq!(picker.selected_model_id().as_deref(), Some("beta"));
    }

    #[test]
    fn test_model_picker_discovered_display_name_defaults_to_id() {
        let model = PickerModel::discovered("gpt-4o".into(), None);
        assert_eq!(model.name, "gpt-4o");
        let model = PickerModel::discovered("gpt-4o".into(), Some("GPT-4o".into()));
        assert_eq!(model.name, "GPT-4o");
        assert_eq!(model.id, "gpt-4o");
    }

    #[test]
    fn test_model_picker_empty_list_is_safe() {
        let mut picker = ModelPicker::new("anything".to_string(), Vec::new(), "nowhere".into());
        assert_eq!(picker.selected_model_id(), None);
        picker.select_down();
        picker.select_up();
        assert_eq!(picker.selected, 0);
        assert_eq!(picker.selected_model_id(), None);
    }
}
