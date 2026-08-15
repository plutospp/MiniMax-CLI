//! Interactive provider picker for switching LLM backends.

use crate::palette;
use crate::tui::views::{ModalKind, ModalView, ViewAction, ViewEvent};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Widget,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

/// A configured provider entry for the picker.
#[derive(Debug, Clone)]
pub struct ProviderPickItem {
    pub name: String,
    pub api: String,
    pub url: String,
    pub default_model: String,
}

/// Result of a provider selection.
#[derive(Debug, Clone)]
pub enum ProviderPickerResult {
    Selected(String),
    Cancelled,
}

/// Interactive picker for selecting a configured provider.
pub struct ProviderPicker {
    selected: usize,
    current: String,
    items: Vec<ProviderPickItem>,
}

impl ProviderPicker {
    pub fn new(current: String, items: Vec<ProviderPickItem>) -> Self {
        let selected = items
            .iter()
            .position(|p| p.name == current)
            .unwrap_or(0);
        Self {
            selected,
            current,
            items,
        }
    }

    fn selected_name(&self) -> Option<String> {
        self.items.get(self.selected).map(|p| p.name.clone())
    }

    fn select_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = self.items.len().saturating_sub(1);
        }
    }

    fn select_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        } else {
            self.selected = 0;
        }
    }
}

impl ModalView for ProviderPicker {
    fn kind(&self) -> ModalKind {
        ModalKind::ProviderPicker
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn handle_key(&mut self, key: KeyEvent) -> ViewAction {
        match key.code {
            KeyCode::Esc => ViewAction::EmitAndClose(ViewEvent::ProviderPickerResult {
                result: ProviderPickerResult::Cancelled,
            }),
            KeyCode::Enter => {
                if let Some(name) = self.selected_name() {
                    ViewAction::EmitAndClose(ViewEvent::ProviderPickerResult {
                        result: ProviderPickerResult::Selected(name),
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
        let popup_width = (area.width * 3 / 5).clamp(50, 72);
        let popup_height = (self.items.len() as u16 * 4 + 5).min(area.height.saturating_sub(4));
        let popup_x = (area.width - popup_width) / 2;
        let popup_y = (area.height - popup_height) / 2;
        let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

        Clear.render(popup_area, buf);

        let block = Block::default()
            .title(" Provider Selection ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette::MINIMAX_BLUE));
        let inner = block.inner(popup_area);
        block.render(popup_area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let is_selected = i == self.selected;
                let is_current = p.name == self.current;
                let style = if is_selected {
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
                let current = if is_current { " ● " } else { "   " };
                let title = format!(
                    "{current}{} ({}){}",
                    p.name,
                    p.api,
                    if is_current { " (current)" } else { "" }
                );
                ListItem::new(vec![
                    Line::from(Span::styled(title, style)),
                    Line::from(Span::styled(
                        format!("     {} · {}", p.default_model, p.url),
                        if is_selected {
                            Style::default()
                                .bg(palette::MINIMAX_BLUE)
                                .fg(palette::MINIMAX_SILVER)
                        } else {
                            Style::default().fg(palette::TEXT_DIM)
                        },
                    )),
                    Line::from(""),
                ])
            })
            .collect();

        List::new(items).render(chunks[0], buf);

        let help = Paragraph::new(Line::from(vec![Span::styled(
            "↑/↓ navigate | Enter select | Esc cancel",
            Style::default().fg(palette::TEXT_DIM),
        )]));
        help.render(chunks[1], buf);
    }
}
