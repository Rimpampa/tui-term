//! Shared helpers for the examples.

use crossterm::event::{
    KeyModifiers, MouseButton as CrosstermMouseButton, MouseEvent as CrosstermMouseEvent,
    MouseEventKind as CrosstermMouseEventKind,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Borders},
};
use tui_term::mouse::{Modifiers, MouseButton, MouseEvent, MouseEventKind};

/// Returns the inner area of the full-screen terminal widget used by most
/// examples.
#[allow(dead_code)]
pub fn terminal_area(size: ratatui::layout::Size) -> Rect {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Percentage(100), Constraint::Min(1)])
        .split(Rect::new(0, 0, size.width, size.height));

    Block::default().borders(Borders::ALL).inner(chunks[0])
}

/// Converts a host mouse event into a terminal-relative mouse event.
///
/// Returns `None` when the event is outside `area`.
pub fn mouse_event(event: CrosstermMouseEvent, area: Rect) -> Option<MouseEvent> {
    let kind = match event.kind {
        CrosstermMouseEventKind::Down(button) => MouseEventKind::Press(map_button(button)),
        CrosstermMouseEventKind::Up(button) => MouseEventKind::Release(map_button(button)),
        CrosstermMouseEventKind::Drag(button) => MouseEventKind::Drag(map_button(button)),
        CrosstermMouseEventKind::Moved => MouseEventKind::Moved,
        CrosstermMouseEventKind::ScrollUp => MouseEventKind::ScrollUp,
        CrosstermMouseEventKind::ScrollDown => MouseEventKind::ScrollDown,
        CrosstermMouseEventKind::ScrollLeft => MouseEventKind::ScrollLeft,
        CrosstermMouseEventKind::ScrollRight => MouseEventKind::ScrollRight,
    };

    MouseEvent::new(kind, event.column, event.row)
        .modifiers(map_modifiers(event.modifiers))
        .relative_to(area)
}

fn map_button(button: CrosstermMouseButton) -> MouseButton {
    match button {
        CrosstermMouseButton::Left => MouseButton::Left,
        CrosstermMouseButton::Middle => MouseButton::Middle,
        CrosstermMouseButton::Right => MouseButton::Right,
    }
}

fn map_modifiers(modifiers: KeyModifiers) -> Modifiers {
    Modifiers {
        shift: modifiers.contains(KeyModifiers::SHIFT),
        alt: modifiers.contains(KeyModifiers::ALT) || modifiers.contains(KeyModifiers::META),
        ctrl: modifiers.contains(KeyModifiers::CONTROL),
    }
}
