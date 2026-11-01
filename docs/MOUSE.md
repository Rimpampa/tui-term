# Mouse input

A terminal application cannot receive mouse events directly from the operating
system. The outer terminal emulator sends escape sequences to the TUI, and the
TUI has to translate those events into the byte stream expected by the
application running inside the PTY.

`tui_term::mouse::MouseEvent` encodes a button, motion, or wheel event into
the xterm mouse protocol. The consumer reads the active protocol from the
parser and writes the encoded event to the PTY only when the inner application
has enabled mouse reporting.

## Protocol selection

The application inside the PTY requests mouse reporting with DEC private modes.
`vt100` records these modes, and the widget consumer can query them:

| Mode | `vt100::MouseProtocolMode` | Events reported |
| --- | --- | --- |
| `CSI ? 9 h` | `Press` | Button presses (X10 compatibility mode) |
| `CSI ? 1000 h` | `PressRelease` | Button presses and releases |
| `CSI ? 1002 h` | `ButtonMotion` | Presses, releases, and drag motion |
| `CSI ? 1003 h` | `AnyMotion` | Presses, releases, and all motion |
| none | `None` | Mouse reporting disabled |

The wire encoding is selected separately:

| Mode | `vt100::MouseProtocolEncoding` | Wire format |
| --- | --- | --- |
| default | `Default` | `ESC [ M Cb Cx Cy` |
| `CSI ? 1005 h` | `Utf8` | Same, with UTF-8 encoded values |
| `CSI ? 1006 h` | `Sgr` | `ESC [ < Cb ; Cx ; Cy M` (press/motion) or `m` (release) |

SGR is the most capable and most common encoding. Legacy encoding offsets every
value by 32 and can only represent coordinates up to 222. The encoder returns
`false` instead of producing a malformed sequence when a coordinate does not
fit.

## Encoding details

The encoded button value (`Cb`) is a bit field:

| Bits | Meaning |
| --- | --- |
| `0..=1` | Button: `0` = left, `1` = middle, `2` = right, `3` = release/none |
| `2` | Shift |
| `3` | Alt/Meta |
| `4` | Control |
| `5` | Motion |
| `6..=7` | Wheel buttons: `64` = up, `65` = down, `66` = left, `67` = right |

In SGR mode the button number is preserved on release and the final character is
lowercase `m`, which removes the ambiguity of the legacy "button 3 means
release" encoding. Wheel events are reported as presses and have no release.

Coordinates on the wire are one-based. `tui_term::mouse::MouseEvent` uses
zero-based coordinates, matching `ratatui` and common input crates. Subtract the
origin of the terminal widget before encoding. If the widget has a block, use
the block's inner area:

```rust,no_run
use crossterm::event::{MouseEvent as CrosstermMouseEvent, MouseEventKind};
use tui_term::mouse::{MouseButton, MouseEvent as TermMouseEvent, MouseEventKind as TermKind};

fn translate(event: CrosstermMouseEvent, inner: ratatui::layout::Rect) -> Option<TermMouseEvent> {
    let kind = match event.kind {
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            TermKind::Press(MouseButton::Left)
        }
        MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
            TermKind::Drag(MouseButton::Left)
        }
        MouseEventKind::ScrollUp => TermKind::ScrollUp,
        // ... map the remaining buttons and scroll directions ...
        _ => return None,
    };

    TermMouseEvent::new(kind, event.column, event.row).relative_to(inner)
}
```

## Forwarding events

Read the mouse mode and encoding from the parsed screen, encode the event, and
write the result to the PTY writer:

```rust,no_run
use std::io;

use tui_term::mouse::MouseEvent;
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

fn write_event(
    event: &MouseEvent,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
    writer: &mut impl io::Write,
) -> io::Result<()> {
    let mut bytes = Vec::new();
    if event.encode_into(mode, encoding, &mut bytes) {
        writer.write_all(&bytes)?;
    }
    Ok(())
}
```

## Coordinating with the outer terminal

The outer TUI must enable mouse capture on its own terminal (for example with
`crossterm::event::EnableMouseCapture`). This is separate from the inner
application's mouse mode: the outer terminal reports events to the TUI, and the
TUI re-encodes them for the PTY.

Common pitfalls:

- Do not forward mouse events when `mouse_protocol_mode()` is `None`; the inner
  application has not asked for them.
- Do not send release or motion events in `Press` mode.
- Do not send motion without a held button in `ButtonMotion` mode.
- Translate coordinates relative to the widget area, not the whole screen.
- Remember that SGR and legacy encodings use different coordinate bases and
  release conventions.
- Mouse wheel events are buttons `64..=67`, not motion events.

## Limitations

`tui-term` encodes xterm-compatible mouse protocols only. It does not implement
DEC Locator mode, highlight tracking, SGR-Pixel mode (`1016`), or the URXVT
encoding (`1015`). Applications that use those modes need custom handling.
Clipboard selection and drag-and-drop are also outside the scope of the widget.
