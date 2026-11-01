//! Translation of mouse events into terminal input escape sequences.
//!
//! Terminals report mouse activity to applications as escape sequences. The
//! format depends on the mouse tracking mode and encoding requested by the
//! application running inside the PTY. The [`vt100::Screen`] tracks both of
//! these, so a consumer can translate host mouse events with [`MouseEvent`]
//! and forward the result to the PTY.
//!
//! The coordinate system used by [`MouseEvent`] is zero-based, matching
//! `ratatui` and common input crates. The escape sequences written by
//! [`MouseEvent::encode_into`] use the one-based coordinates required by the
//! terminal protocols.
//!
//! # Example
//!
//! ```rust
//! use tui_term::mouse::{MouseButton, MouseEvent, MouseEventKind, Modifiers};
//! use vt100::{MouseProtocolEncoding, MouseProtocolMode};
//!
//! let event = MouseEvent::new(MouseEventKind::Press(MouseButton::Left), 4, 2)
//!     .modifiers(Modifiers::SHIFT);
//! let mut bytes = Vec::new();
//! let encoded = event.encode_into(
//!     MouseProtocolMode::PressRelease,
//!     MouseProtocolEncoding::Sgr,
//!     &mut bytes,
//! );
//! assert!(encoded);
//! assert_eq!(bytes, b"\x1b[<4;5;3M");
//! ```

use alloc::vec::Vec;
use core::ops::{BitOr, BitOrAssign};

use ratatui_core::layout::Rect;
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseButton {
    /// The primary (left) mouse button.
    Left,
    /// The middle mouse button.
    Middle,
    /// The secondary (right) mouse button.
    Right,
}

impl MouseButton {
    const fn code(self) -> u16 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
        }
    }
}

/// Keyboard modifiers held during a mouse event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Shift is held.
    pub shift: bool,
    /// Alt (or Meta) is held.
    pub alt: bool,
    /// Control is held.
    pub ctrl: bool,
}

impl Modifiers {
    /// No modifiers.
    pub const NONE: Self = Self {
        shift: false,
        alt: false,
        ctrl: false,
    };
    /// The shift modifier.
    pub const SHIFT: Self = Self {
        shift: true,
        alt: false,
        ctrl: false,
    };
    /// The alt/meta modifier.
    pub const ALT: Self = Self {
        shift: false,
        alt: true,
        ctrl: false,
    };
    /// The control modifier.
    pub const CTRL: Self = Self {
        shift: false,
        alt: false,
        ctrl: true,
    };

    const fn bits(self) -> u16 {
        (self.shift as u16) * 4 + (self.alt as u16) * 8 + (self.ctrl as u16) * 16
    }
}

impl BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self {
            shift: self.shift || rhs.shift,
            alt: self.alt || rhs.alt,
            ctrl: self.ctrl || rhs.ctrl,
        }
    }
}

impl BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

/// The kind of mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseEventKind {
    /// A button was pressed.
    Press(MouseButton),
    /// A button was released.
    Release(MouseButton),
    /// The mouse moved while the given button was held.
    Drag(MouseButton),
    /// The mouse moved without a button being held.
    Moved,
    /// The scroll wheel was moved up.
    ScrollUp,
    /// The scroll wheel was moved down.
    ScrollDown,
    /// The scroll wheel was tilted left.
    ScrollLeft,
    /// The scroll wheel was tilted right.
    ScrollRight,
}

/// A mouse event in zero-based terminal-cell coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    /// The kind of event.
    pub kind: MouseEventKind,
    /// Zero-based column of the event.
    pub column: u16,
    /// Zero-based row of the event.
    pub row: u16,
    /// Modifiers held during the event.
    pub modifiers: Modifiers,
}

impl MouseEvent {
    /// Creates a new mouse event without modifiers.
    #[must_use]
    pub const fn new(kind: MouseEventKind, column: u16, row: u16) -> Self {
        Self {
            kind,
            column,
            row,
            modifiers: Modifiers::NONE,
        }
    }

    /// Sets the modifiers for the event.
    #[must_use]
    pub const fn modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// Converts screen coordinates into coordinates relative to `area`.
    ///
    /// Returns `None` when the event is outside `area`. When the terminal is
    /// rendered inside a block, pass [`Block::inner`](ratatui_widgets::block::Block::inner)
    /// rather than the block's outer area.
    #[must_use]
    pub fn relative_to(self, area: Rect) -> Option<Self> {
        let column = self.column.checked_sub(area.x)?;
        let row = self.row.checked_sub(area.y)?;
        if column < area.width && row < area.height {
            Some(Self {
                column,
                row,
                ..self
            })
        } else {
            None
        }
    }

    /// Encodes the event into `out`, returning whether it was written.
    ///
    /// The event is not encoded when mouse reporting is disabled, or when the
    /// requested mode does not report this kind of event. For example,
    /// [`MouseProtocolMode::Press`] only reports button presses and wheel
    /// events, while [`MouseProtocolMode::AnyMotion`] also reports motion
    /// without a button pressed.
    ///
    /// [`MouseProtocolEncoding::Default`] cannot represent coordinates larger
    /// than 222, because each coordinate is written as a single byte with an
    /// offset of 32. In that case this method returns `false`. Prefer SGR
    /// encoding (`MouseProtocolEncoding::Sgr`) when available.
    #[must_use = "the event may not be encodable in the requested mode"]
    pub fn encode_into(
        &self,
        mode: MouseProtocolMode,
        encoding: MouseProtocolEncoding,
        out: &mut Vec<u8>,
    ) -> bool {
        if !mode_reports(mode, self.kind) {
            return false;
        }

        let (button, motion, release) = match self.kind {
            MouseEventKind::Press(button) => (button.code(), false, false),
            MouseEventKind::Release(button) => (button.code(), false, true),
            MouseEventKind::Drag(button) => (button.code(), true, false),
            MouseEventKind::Moved => (3, true, false),
            MouseEventKind::ScrollUp => (64, false, false),
            MouseEventKind::ScrollDown => (65, false, false),
            MouseEventKind::ScrollLeft => (66, false, false),
            MouseEventKind::ScrollRight => (67, false, false),
        };

        match encoding {
            MouseProtocolEncoding::Sgr => {
                let column = match self.column.checked_add(1) {
                    Some(column) => column,
                    None => return false,
                };
                let row = match self.row.checked_add(1) {
                    Some(row) => row,
                    None => return false,
                };

                let mut cb = button | self.modifiers.bits();
                if motion {
                    cb |= 32;
                }

                out.extend_from_slice(b"\x1b[<");
                push_decimal(out, cb);
                out.push(b';');
                push_decimal(out, column);
                out.push(b';');
                push_decimal(out, row);
                out.push(if release { b'm' } else { b'M' });
            }
            MouseProtocolEncoding::Default | MouseProtocolEncoding::Utf8 => {
                // The legacy encoding has no way to distinguish which button
                // was released, so release is always reported as button 3.
                let mut cb = if release { 3 } else { button } | self.modifiers.bits();
                if motion {
                    cb |= 32;
                }

                let column = match self.column.checked_add(33) {
                    Some(column) => column,
                    None => return false,
                };
                let row = match self.row.checked_add(33) {
                    Some(row) => row,
                    None => return false,
                };

                match encoding {
                    MouseProtocolEncoding::Default => {
                        // Keep every encoded value in the single-byte range.
                        if cb + 32 > u16::from(u8::MAX)
                            || column > u16::from(u8::MAX)
                            || row > u16::from(u8::MAX)
                        {
                            return false;
                        }
                        out.extend_from_slice(b"\x1b[M");
                        out.push((cb + 32) as u8);
                        out.push(column as u8);
                        out.push(row as u8);
                    }
                    MouseProtocolEncoding::Utf8 => {
                        out.extend_from_slice(b"\x1b[M");
                        push_utf8(out, cb + 32);
                        push_utf8(out, column);
                        push_utf8(out, row);
                    }
                    MouseProtocolEncoding::Sgr => unreachable!(),
                }
            }
        }

        true
    }

    /// Encodes the event into a new byte vector.
    ///
    /// Returns `None` when the event cannot be represented in the requested
    /// mode or encoding. See [`MouseEvent::encode_into`].
    #[must_use]
    pub fn encode(
        &self,
        mode: MouseProtocolMode,
        encoding: MouseProtocolEncoding,
    ) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        self.encode_into(mode, encoding, &mut out).then_some(out)
    }
}

fn mode_reports(mode: MouseProtocolMode, kind: MouseEventKind) -> bool {
    use MouseEventKind::{
        Drag, Moved, Press, Release, ScrollDown, ScrollLeft, ScrollRight, ScrollUp,
    };

    match mode {
        MouseProtocolMode::None => false,
        MouseProtocolMode::Press => matches!(
            kind,
            Press(_) | ScrollUp | ScrollDown | ScrollLeft | ScrollRight
        ),
        MouseProtocolMode::PressRelease => matches!(
            kind,
            Press(_) | Release(_) | ScrollUp | ScrollDown | ScrollLeft | ScrollRight
        ),
        MouseProtocolMode::ButtonMotion => matches!(
            kind,
            Press(_) | Release(_) | Drag(_) | ScrollUp | ScrollDown | ScrollLeft | ScrollRight
        ),
        MouseProtocolMode::AnyMotion => matches!(
            kind,
            Press(_)
                | Release(_)
                | Drag(_)
                | Moved
                | ScrollUp
                | ScrollDown
                | ScrollLeft
                | ScrollRight
        ),
    }
}

fn push_decimal(out: &mut Vec<u8>, mut value: u16) {
    let mut buf = [0u8; 5];
    let mut index = buf.len();
    loop {
        index -= 1;
        buf[index] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[index..]);
}

fn push_utf8(out: &mut Vec<u8>, value: u16) {
    let character = char::from_u32(u32::from(value)).expect("u16 is a valid Unicode scalar value");
    let mut buf = [0u8; 4];
    out.extend_from_slice(character.encode_utf8(&mut buf).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const SGR: MouseProtocolEncoding = MouseProtocolEncoding::Sgr;
    const DEFAULT: MouseProtocolEncoding = MouseProtocolEncoding::Default;

    #[test]
    fn sgr_press_and_release() {
        let press = MouseEvent::new(MouseEventKind::Press(MouseButton::Left), 0, 0)
            .encode(MouseProtocolMode::PressRelease, SGR);
        assert_eq!(press.as_deref(), Some(b"\x1b[<0;1;1M".as_slice()));

        let release = MouseEvent::new(MouseEventKind::Release(MouseButton::Right), 4, 2)
            .encode(MouseProtocolMode::PressRelease, SGR);
        assert_eq!(release.as_deref(), Some(b"\x1b[<2;5;3m".as_slice()));
    }

    #[test]
    fn default_press_and_release() {
        let press = MouseEvent::new(MouseEventKind::Press(MouseButton::Left), 0, 0)
            .encode(MouseProtocolMode::PressRelease, DEFAULT);
        assert_eq!(press.as_deref(), Some(b"\x1b[M !!".as_slice()));

        let release = MouseEvent::new(MouseEventKind::Release(MouseButton::Right), 0, 0)
            .encode(MouseProtocolMode::PressRelease, DEFAULT);
        assert_eq!(release.as_deref(), Some(b"\x1b[M#!!".as_slice()));
    }

    #[test]
    fn drag_and_motion() {
        let drag = MouseEvent::new(MouseEventKind::Drag(MouseButton::Left), 1, 1)
            .encode(MouseProtocolMode::ButtonMotion, DEFAULT);
        assert_eq!(drag.as_deref(), Some(b"\x1b[M@\"\"".as_slice()));

        let moved = MouseEvent::new(MouseEventKind::Moved, 0, 0)
            .encode(MouseProtocolMode::AnyMotion, DEFAULT);
        assert_eq!(moved.as_deref(), Some(b"\x1b[MC!!".as_slice()));
    }

    #[test]
    fn wheel_events() {
        let up = MouseEvent::new(MouseEventKind::ScrollUp, 0, 0)
            .encode(MouseProtocolMode::PressRelease, DEFAULT);
        assert_eq!(up.as_deref(), Some(b"\x1b[M`!!".as_slice()));

        let right = MouseEvent::new(MouseEventKind::ScrollRight, 0, 0)
            .encode(MouseProtocolMode::PressRelease, SGR);
        assert_eq!(right.as_deref(), Some(b"\x1b[<67;1;1M".as_slice()));
    }

    #[test]
    fn modifiers_are_encoded() {
        let event = MouseEvent::new(MouseEventKind::Press(MouseButton::Left), 0, 0)
            .modifiers(Modifiers::SHIFT | Modifiers::CTRL);
        let bytes = event.encode(MouseProtocolMode::PressRelease, SGR).unwrap();
        assert_eq!(bytes, b"\x1b[<20;1;1M");
    }

    #[test]
    fn mode_filters_events() {
        let release = MouseEvent::new(MouseEventKind::Release(MouseButton::Left), 0, 0);
        assert!(
            release.encode(MouseProtocolMode::Press, SGR).is_none(),
            "X10 mode does not report releases"
        );

        let drag = MouseEvent::new(MouseEventKind::Drag(MouseButton::Left), 0, 0);
        assert!(drag.encode(MouseProtocolMode::PressRelease, SGR).is_none());
        assert!(drag.encode(MouseProtocolMode::ButtonMotion, SGR).is_some());

        let moved = MouseEvent::new(MouseEventKind::Moved, 0, 0);
        assert!(moved.encode(MouseProtocolMode::ButtonMotion, SGR).is_none());
        assert!(moved.encode(MouseProtocolMode::AnyMotion, SGR).is_some());
        assert!(moved.encode(MouseProtocolMode::None, SGR).is_none());
    }

    #[test]
    fn default_encoding_rejects_large_coordinates() {
        let event = MouseEvent::new(MouseEventKind::Press(MouseButton::Left), 223, 0);
        let mut out = Vec::new();
        assert!(!event.encode_into(MouseProtocolMode::PressRelease, DEFAULT, &mut out));
        assert!(out.is_empty());
        assert!(event.encode(MouseProtocolMode::PressRelease, SGR).is_some());
    }

    #[test]
    fn utf8_encoding_uses_multibyte_coordinates() {
        let event = MouseEvent::new(MouseEventKind::Press(MouseButton::Left), 100, 2);
        let bytes = event
            .encode(MouseProtocolMode::PressRelease, MouseProtocolEncoding::Utf8)
            .unwrap();
        assert_eq!(bytes, b"\x1b[M \xc2\x85#");
    }

    #[test]
    fn coordinates_can_be_made_relative_to_an_area() {
        let area = Rect::new(10, 5, 20, 10);
        let event = MouseEvent::new(MouseEventKind::Moved, 12, 7);
        assert_eq!(event.relative_to(area).unwrap().column, 2);
        assert_eq!(event.relative_to(area).unwrap().row, 2);
        assert!(MouseEvent::new(MouseEventKind::Moved, 9, 7)
            .relative_to(area)
            .is_none());
        assert!(MouseEvent::new(MouseEventKind::Moved, 12, 15)
            .relative_to(area)
            .is_none());
    }
}
