//! Multi-field modal view for defining a new LLM provider.
//!
//! Fields: Name, API protocol (toggle), Base URL, API key (masked), Default model.

use crate::config::ProviderApi;
use crate::palette;
use crate::tui::views::{ModalKind, ModalView, ViewAction, ViewEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Widget,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

/// Result of a provider creation session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderFormResult {
    Submitted {
        name: String,
        api: ProviderApi,
        url: String,
        api_key: String,
        default_model: String,
    },
    Cancelled,
}

struct Field {
    label: &'static str,
    value: String,
    masked: bool,
    is_toggle: bool,
    api_val: ProviderApi,
}

/// Interactive 5-field form for defining a new provider.
pub struct ProviderFormView {
    fields: [Field; 5],
    selected: usize,
    error: Option<String>,
}

impl ProviderFormView {
    /// Create a new form with default empty fields.
    pub fn new() -> Self {
        Self {
            fields: [
                Field {
                    label: "Provider name",
                    value: String::new(),
                    masked: false,
                    is_toggle: false,
                    api_val: ProviderApi::Anthropic,
                },
                Field {
                    label: "API protocol",
                    value: String::new(),
                    masked: false,
                    is_toggle: true,
                    api_val: ProviderApi::Anthropic,
                },
                Field {
                    label: "Base URL",
                    value: String::new(),
                    masked: false,
                    is_toggle: false,
                    api_val: ProviderApi::Anthropic,
                },
                Field {
                    label: "API key (optional)",
                    value: String::new(),
                    masked: true,
                    is_toggle: false,
                    api_val: ProviderApi::Anthropic,
                },
                Field {
                    label: "Default model (optional)",
                    value: String::new(),
                    masked: false,
                    is_toggle: false,
                    api_val: ProviderApi::Anthropic,
                },
            ],
            selected: 0,
            error: None,
        }
    }

    fn toggle_api(&mut self) {
        self.fields[1].api_val = match self.fields[1].api_val {
            ProviderApi::Anthropic => ProviderApi::OpenAi,
            ProviderApi::OpenAi => ProviderApi::Anthropic,
        };
    }

    fn submit(&mut self) -> ViewAction {
        let name = self.fields[0].value.trim().to_string();
        let api = self.fields[1].api_val;
        let url = self.fields[2].value.trim().to_string();
        let api_key = self.fields[3].value.clone();
        let default_model = self.fields[4].value.trim().to_string();

        if name.is_empty() {
            self.error = Some("Provider name is required".to_string());
            return ViewAction::None;
        }
        if url.is_empty() {
            self.error = Some("Base URL is required".to_string());
            return ViewAction::None;
        }

        ViewAction::EmitAndClose(ViewEvent::ProviderAdded {
            result: ProviderFormResult::Submitted {
                name,
                api,
                url,
                api_key,
                default_model,
            },
        })
    }
}

impl Default for ProviderFormView {
    fn default() -> Self {
        Self::new()
    }
}

impl ModalView for ProviderFormView {
    fn kind(&self) -> ModalKind {
        ModalKind::ProviderForm
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn handle_key(&mut self, key: KeyEvent) -> ViewAction {
        match key.code {
            KeyCode::Esc => ViewAction::EmitAndClose(ViewEvent::ProviderAdded {
                result: ProviderFormResult::Cancelled,
            }),
            KeyCode::Up => {
                if self.selected == 0 {
                    self.selected = self.fields.len() - 1;
                } else {
                    self.selected -= 1;
                }
                ViewAction::None
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1) % self.fields.len();
                ViewAction::None
            }
            KeyCode::Char('k') | KeyCode::Char('p')
                if self.fields[self.selected].is_toggle
                    || key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if self.selected == 0 {
                    self.selected = self.fields.len() - 1;
                } else {
                    self.selected -= 1;
                }
                ViewAction::None
            }
            KeyCode::Char('j') | KeyCode::Char('n')
                if self.fields[self.selected].is_toggle
                    || key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.selected = (self.selected + 1) % self.fields.len();
                ViewAction::None
            }
            KeyCode::BackTab => {
                if self.selected == 0 {
                    self.selected = self.fields.len() - 1;
                } else {
                    self.selected -= 1;
                }
                ViewAction::None
            }
            KeyCode::Tab => {
                self.selected = (self.selected + 1) % self.fields.len();
                ViewAction::None
            }
            KeyCode::Left | KeyCode::Right if self.fields[self.selected].is_toggle => {
                self.toggle_api();
                ViewAction::None
            }
            KeyCode::Enter => {
                if self.fields[self.selected].is_toggle {
                    self.toggle_api();
                    ViewAction::None
                } else if self.selected == self.fields.len() - 1 {
                    self.submit()
                } else {
                    self.selected += 1;
                    ViewAction::None
                }
            }
            KeyCode::Backspace => {
                let field = &mut self.fields[self.selected];
                if !field.is_toggle {
                    field.value.pop();
                }
                ViewAction::None
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let field = &mut self.fields[self.selected];
                if !field.is_toggle {
                    field.value.clear();
                }
                ViewAction::None
            }
            KeyCode::Char(c) if !c.is_control() => {
                let field = &mut self.fields[self.selected];
                if !field.is_toggle {
                    field.value.push(c);
                }
                ViewAction::None
            }
            _ => ViewAction::None,
        }
    }

    fn handle_paste(&mut self, text: &str) -> ViewAction {
        let field = &mut self.fields[self.selected];
        if !field.is_toggle {
            let clean: String = text.chars().filter(|c| !c.is_control()).collect();
            field.value.push_str(&clean);
        }
        ViewAction::None
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let popup_width = (area.width * 4 / 5).clamp(55, 75);
        let popup_height = 11u16;
        let popup_x = (area.width - popup_width) / 2;
        let popup_y = (area.height - popup_height) / 2;
        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        Clear.render(popup_area, buf);

        let block = Block::default()
            .title(" Add Provider ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette::MINIMAX_BLUE));
        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let mut constraints = vec![
            Constraint::Length(1), // Name
            Constraint::Length(1), // API Protocol
            Constraint::Length(1), // URL
            Constraint::Length(1), // API Key
            Constraint::Length(1), // Default Model
            Constraint::Length(1), // Empty line or Error
            Constraint::Length(1), // Footer hint
        ];
        if inner.height < 7 {
            constraints.truncate(inner.height as usize);
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        for (i, field) in self.fields.iter().enumerate() {
            if i >= chunks.len() {
                break;
            }
            let is_selected = i == self.selected;
            let label_style = if is_selected {
                Style::default()
                    .fg(palette::MINIMAX_BLUE)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette::TEXT_MUTED)
            };

            let prefix = if is_selected { "▸ " } else { "  " };

            let val_span = if field.is_toggle {
                let toggle_text = match field.api_val {
                    ProviderApi::Anthropic => "< anthropic > (OpenAI / Anthropic)",
                    ProviderApi::OpenAi => "< openai > (OpenAI / Anthropic)",
                };
                Span::styled(
                    toggle_text,
                    if is_selected {
                        Style::default()
                            .fg(palette::TEXT_PRIMARY)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(palette::TEXT_MUTED)
                    },
                )
            } else if field.masked {
                let display = if field.value.is_empty() {
                    if is_selected {
                        "(type key...)"
                    } else {
                        "(empty)"
                    }
                } else {
                    "●●●●●●●●"
                };
                Span::styled(
                    display,
                    if field.value.is_empty() {
                        Style::default().fg(palette::TEXT_DIM)
                    } else {
                        Style::default().fg(palette::TEXT_PRIMARY)
                    },
                )
            } else {
                let display = if field.value.is_empty() {
                    if is_selected {
                        "_"
                    } else {
                        "(empty)"
                    }
                } else {
                    &field.value
                };
                Span::styled(
                    display,
                    if field.value.is_empty() {
                        Style::default().fg(palette::TEXT_DIM)
                    } else {
                        Style::default().fg(palette::TEXT_PRIMARY)
                    },
                )
            };

            let line = Line::from(vec![
                Span::styled(prefix, label_style),
                Span::styled(format!("{:<24}: ", field.label), label_style),
                val_span,
            ]);
            Paragraph::new(line).render(chunks[i], buf);
        }

        // Error or blank row
        if chunks.len() > 5 {
            if let Some(err) = &self.error {
                Paragraph::new(Line::from(vec![
                    Span::styled("  ✗ ", Style::default().fg(palette::STATUS_ERROR)),
                    Span::styled(err, Style::default().fg(palette::STATUS_ERROR)),
                ]))
                .render(chunks[5], buf);
            }
        }

        // Footer hint
        if chunks.len() > 6 {
            let hint = Line::from(Span::styled(
                "  Enter: next/save  Esc: cancel  ↑/↓: move  ←/→: toggle protocol",
                Style::default().fg(palette::TEXT_MUTED),
            ));
            Paragraph::new(hint).render(chunks[6], buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_form_types_j_and_k_in_text_field() {
        let mut view = ProviderFormView::new();
        assert_eq!(view.selected, 0); // Provider name

        // Type 'k' and 'j' into name field
        view.handle_key(KeyEvent::from(KeyCode::Char('k')));
        view.handle_key(KeyEvent::from(KeyCode::Char('j')));
        assert_eq!(view.selected, 0);
        assert_eq!(view.fields[0].value, "kj");
    }

    #[test]
    fn test_provider_form_navigation_and_cancel() {
        let mut view = ProviderFormView::new();
        assert_eq!(view.selected, 0);

        // Down navigates
        let action = view.handle_key(KeyEvent::from(KeyCode::Down));
        assert!(matches!(action, ViewAction::None));
        assert_eq!(view.selected, 1);

        // Up navigates back
        let action = view.handle_key(KeyEvent::from(KeyCode::Up));
        assert!(matches!(action, ViewAction::None));
        assert_eq!(view.selected, 0);

        // Esc produces Cancelled result
        let action = view.handle_key(KeyEvent::from(KeyCode::Esc));
        match action {
            ViewAction::EmitAndClose(ViewEvent::ProviderAdded { result }) => {
                assert_eq!(result, ProviderFormResult::Cancelled);
            }
            _ => panic!("expected EmitAndClose(ProviderAdded Cancelled)"),
        }
    }
}
