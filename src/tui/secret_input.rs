//! Masked single-line secret input modal (used for provider API keys).
//!
//! Typed characters are never rendered; Enter submits, Esc cancels.

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

/// Result of a secret input session.
#[derive(Debug, Clone)]
pub struct SecretInputResult {
    /// Provider the key is for.
    pub provider: String,
    /// Submitted key, or `None` when cancelled / left empty.
    pub value: Option<String>,
}

/// Interactive masked input for a secret belonging to one provider.
pub struct SecretInputView {
    provider: String,
    prompt: String,
    input: String,
}

impl SecretInputView {
    /// Create a masked input titled for `provider`.
    pub fn new(provider: String) -> Self {
        let prompt = format!("API key for '{provider}'");
        Self {
            provider,
            prompt,
            input: String::new(),
        }
    }

    fn submit(&self) -> ViewAction {
        let value = self.input.trim();
        let value = if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        };
        ViewAction::EmitAndClose(ViewEvent::SecretInputResult {
            result: SecretInputResult {
                provider: self.provider.clone(),
                value,
            },
        })
    }
}

impl ModalView for SecretInputView {
    fn kind(&self) -> ModalKind {
        ModalKind::SecretInput
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn handle_key(&mut self, key: KeyEvent) -> ViewAction {
        match key.code {
            KeyCode::Esc => ViewAction::EmitAndClose(ViewEvent::SecretInputResult {
                result: SecretInputResult {
                    provider: self.provider.clone(),
                    value: None,
                },
            }),
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.input.pop();
                ViewAction::None
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                ViewAction::None
            }
            KeyCode::Char(c) if !c.is_control() => {
                self.input.push(c);
                ViewAction::None
            }
            _ => ViewAction::None,
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let popup_width = (area.width * 3 / 5).clamp(50, 70);
        let popup_height = 5u16;
        let popup_x = (area.width - popup_width) / 2;
        let popup_y = (area.height - popup_height) / 2;
        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        Clear.render(popup_area, buf);

        let block = Block::default()
            .title(" Login ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette::MINIMAX_BLUE));
        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

        Paragraph::new(Line::from(Span::styled(
            self.prompt.clone(),
            Style::default()
                .fg(palette::TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )))
        .render(chunks[0], buf);

        // Never render the secret itself; show one bullet per character.
        let masked: String = std::iter::repeat_n('*', self.input.chars().count()).collect();
        let field = if masked.is_empty() {
            Line::from(Span::styled(
                "(type or paste your key)",
                Style::default().fg(palette::TEXT_MUTED),
            ))
        } else {
            Line::from(vec![
                Span::styled(masked, Style::default().fg(palette::MINIMAX_ORANGE)),
                Span::styled("_", Style::default().fg(palette::MINIMAX_BLUE)),
            ])
        };
        Paragraph::new(field).render(chunks[1], buf);

        Paragraph::new(Line::from(Span::styled(
            "Enter to save | Esc to cancel | Ctrl+U to clear",
            Style::default().fg(palette::TEXT_DIM),
        )))
        .render(chunks[2], buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_submit_and_mask() {
        let mut view = SecretInputView::new("kimi".to_string());
        for c in "sk-secret".chars() {
            assert!(matches!(
                view.handle_key(key(KeyCode::Char(c))),
                ViewAction::None
            ));
        }
        let ViewAction::EmitAndClose(ViewEvent::SecretInputResult { result }) =
            view.handle_key(key(KeyCode::Enter))
        else {
            panic!("expected submit event");
        };
        assert_eq!(result.provider, "kimi");
        assert_eq!(result.value.as_deref(), Some("sk-secret"));
    }

    #[test]
    fn escape_cancels() {
        let mut view = SecretInputView::new("zai".to_string());
        view.handle_key(key(KeyCode::Char('x')));
        let ViewAction::EmitAndClose(ViewEvent::SecretInputResult { result }) =
            view.handle_key(key(KeyCode::Esc))
        else {
            panic!("expected cancel event");
        };
        assert_eq!(result.value, None);
    }

    #[test]
    fn empty_input_submits_none() {
        let mut view = SecretInputView::new("deepseek".to_string());
        let ViewAction::EmitAndClose(ViewEvent::SecretInputResult { result }) =
            view.handle_key(key(KeyCode::Enter))
        else {
            panic!("expected submit event");
        };
        assert_eq!(result.value, None);
    }

    #[test]
    fn backspace_and_clear_line() {
        let mut view = SecretInputView::new("kimi".to_string());
        for c in "abc".chars() {
            view.handle_key(key(KeyCode::Char(c)));
        }
        view.handle_key(key(KeyCode::Backspace));
        assert_eq!(view.input, "ab");
        view.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(view.input, "");
    }
}
