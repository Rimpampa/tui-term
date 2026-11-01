//! Responses to terminal capability queries.
//!
//! Terminal applications frequently ask the terminal questions such as "what
//! kind of terminal are you?" (DA1) or "where is the cursor?" (DSR). A real
//! terminal writes the answer back to the application. When the application
//! runs behind a PTY managed by `tui-term`, the TUI is the terminal and has to
//! provide those answers.
//!
//! [`TerminalResponder`] implements [`vt100::Callbacks`]. Construct the parser
//! with [`vt100::Parser::new_with_callbacks`] and drain the responses after
//! processing output:
//!
//! ```rust
//! use tui_term::terminal::TerminalResponder;
//!
//! let mut parser = vt100::Parser::new_with_callbacks(
//!     24,
//!     80,
//!     0,
//!     TerminalResponder::new(),
//! );
//! parser.process(b"\x1b[0c");
//! assert_eq!(parser.callbacks_mut().take_responses(), b"\x1b[?62;22c");
//! ```

use alloc::vec::Vec;

use vt100::{Callbacks, Screen};

/// Produces terminal-query responses for a PTY application.
#[derive(Debug, Clone, Default)]
pub struct TerminalResponder {
    responses: Vec<u8>,
}

impl TerminalResponder {
    /// Creates a new responder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the responses accumulated since the last drain.
    #[must_use]
    pub fn responses(&self) -> &[u8] {
        &self.responses
    }

    /// Takes the responses accumulated since the last drain.
    #[must_use]
    pub fn take_responses(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.responses)
    }

    fn cursor_position_report(&mut self, cursor: (u16, u16)) {
        self.responses.extend_from_slice(b"\x1b[");
        push_decimal(&mut self.responses, cursor.0.saturating_add(1));
        self.responses.push(b';');
        push_decimal(&mut self.responses, cursor.1.saturating_add(1));
        self.responses.push(b'R');
    }
}

impl Callbacks for TerminalResponder {
    fn unhandled_csi(
        &mut self,
        screen: &mut Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        match (i1, c) {
            // Primary Device Attributes.
            (None, 'c') => {
                self.responses.extend_from_slice(b"\x1b[?62;22c");
            }
            // Device Status Report / Cursor Position Report.
            (None, 'n') => match first_param(params) {
                Some(5) => self.responses.extend_from_slice(b"\x1b[0n"),
                Some(6) => self.cursor_position_report(screen.cursor_position()),
                _ => {}
            },
            // XTVERSION.
            (Some(b'>'), 'q') => {
                self.responses.extend_from_slice(b"\x1bP>|tui-term\x1b\\");
            }
            // Kitty keyboard protocol query. Report that the protocol is not
            // enabled.
            (Some(b'?'), 'u') => {
                self.responses.extend_from_slice(b"\x1b[?0u");
            }
            _ => {}
        }
    }

    fn unhandled_dcs(
        &mut self,
        _screen: &mut Screen,
        _params: &[&[u16]],
        intermediates: &[u8],
        action: char,
        payload: &[u8],
    ) {
        // XTGETTCAP: request for a terminfo capability.
        if intermediates == b"+" && action == 'q' {
            if payload == b"71756572792d6f732d6e616d65" {
                self.responses.extend_from_slice(b"\x1bP1+r");
                self.responses.extend_from_slice(payload);
                self.responses.push(b'=');
                push_hex(&mut self.responses, std::env::consts::OS.as_bytes());
                self.responses.extend_from_slice(b"\x1b\\");
            } else {
                self.responses.extend_from_slice(b"\x1bP0+r");
                self.responses.extend_from_slice(payload);
                self.responses.extend_from_slice(b"\x1b\\");
            }
        }
    }
}

fn first_param(params: &[&[u16]]) -> Option<u16> {
    params.first().and_then(|param| param.first().copied())
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

fn push_hex(out: &mut Vec<u8>, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize]);
        out.push(HEX[(byte & 0x0f) as usize]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser(responder: TerminalResponder) -> vt100::Parser<TerminalResponder> {
        vt100::Parser::new_with_callbacks(24, 80, 0, responder)
    }

    fn process(input: &[u8]) -> Vec<u8> {
        let mut parser = parser(TerminalResponder::new());
        parser.process(input);
        parser.callbacks_mut().take_responses()
    }

    #[test]
    fn responds_to_device_attributes() {
        assert_eq!(process(b"\x1b[0c"), b"\x1b[?62;22c");
        assert_eq!(process(b"\x1b[c"), b"\x1b[?62;22c");
    }

    #[test]
    fn responds_to_cursor_position_report() {
        assert_eq!(process(b"abc\x1b[6n"), b"\x1b[1;4R");
    }

    #[test]
    fn handles_split_queries() {
        let mut parser = parser(TerminalResponder::new());
        parser.process(b"\x1b[0");
        assert!(parser.callbacks_mut().take_responses().is_empty());
        parser.process(b"c");
        assert_eq!(parser.callbacks_mut().take_responses(), b"\x1b[?62;22c");
    }

    #[test]
    fn responds_to_xtversion_and_kitty_query() {
        assert_eq!(process(b"\x1b[>0q"), b"\x1bP>|tui-term\x1b\\");
        assert_eq!(process(b"\x1b[?u"), b"\x1b[?0u");
    }

    #[test]
    fn ignores_non_query_escape_sequences() {
        assert!(process(b"\x1b[31mhello\x1b[0m").is_empty());
    }

    #[test]
    fn handles_multiple_queries_in_one_chunk() {
        assert_eq!(process(b"\x1b[0c\x1b[6n"), b"\x1b[?62;22c\x1b[1;1R");
    }

    #[test]
    fn responds_to_xtgettcap_os_name() {
        let response = process(b"\x1bP+q71756572792d6f732d6e616d65\x1b\\");
        assert!(response.starts_with(b"\x1bP1+r71756572792d6f732d6e616d65="));
        assert!(response.ends_with(b"\x1b\\"));
    }
}
