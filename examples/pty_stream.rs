//! This example creates the PTY itself and then hands the reader and writer
//! to `tui_term::stream::PtyStream`.
//!
//! Run with:
//!
//! ```sh
//! cargo run --example pty_stream
//! ```
//!
//! Press `Ctrl+q` to quit.

use std::{io, time::Duration};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    DefaultTerminal, Frame,
};
use tui_term::{
    stream::{PtyStream, Size},
    widget::PseudoTerminal,
};

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal);
    ratatui::restore();
    result
}

fn run_app(terminal: &mut DefaultTerminal) -> io::Result<()> {
    let terminal_size = terminal.size()?;
    let mut size = Size::new(terminal_size.height, terminal_size.width);

    // Create the PTY ourselves. `PtyStream` only needs the two streams.
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(io::Error::other)?;

    let mut cmd = CommandBuilder::new_default_prog();
    cmd.cwd(std::env::current_dir()?);
    let mut child = pair.slave.spawn_command(cmd).map_err(io::Error::other)?;
    drop(pair.slave);
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    let reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
    let writer = pair.master.take_writer().map_err(io::Error::other)?;
    let master = pair.master;
    let stream = PtyStream::new(reader, writer, size)?;

    loop {
        stream.with_screen(|screen| terminal.draw(|frame| ui(frame, screen, size)))?;

        if event::poll(Duration::from_millis(10))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if key.code == KeyCode::Char('q')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        return Ok(());
                    }

                    let application_cursor =
                        stream.with_screen(|screen| screen.application_cursor());
                    if let Some(bytes) = key_to_bytes(&key, application_cursor) {
                        stream.write_input(&bytes)?;
                    }
                }
                Event::Paste(text) => {
                    let bracketed = stream.with_screen(|screen| screen.bracketed_paste());
                    if bracketed {
                        stream.write_input(b"\x1b[200~")?;
                        stream.write_input(text.as_bytes())?;
                        stream.write_input(b"\x1b[201~")?;
                    } else {
                        stream.write_input(text.as_bytes())?;
                    }
                }
                Event::Resize(cols, rows) => {
                    size = Size::new(rows, cols);
                    stream.resize(rows, cols);
                    master
                        .resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        })
                        .map_err(io::Error::other)?;
                }
                _ => {}
            }
        }

        if stream.finished() {
            return Ok(());
        }
    }
}

fn ui(frame: &mut Frame, screen: &vt100::Screen, size: Size) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Percentage(100), Constraint::Min(1)])
        .split(frame.area());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("{}x{}", size.cols, size.rows))
        .style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(PseudoTerminal::new(screen).block(block), chunks[0]);

    let explanation = Paragraph::new("Ctrl+q to quit")
        .style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED))
        .alignment(Alignment::Center);
    frame.render_widget(explanation, chunks[1]);
}

fn key_to_bytes(key: &KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    let bytes = match key.code {
        KeyCode::Char(character) => {
            let mut bytes = if key.modifiers.contains(KeyModifiers::CONTROL) {
                control_character(character)?
            } else {
                character.to_string().into_bytes()
            };
            if key.modifiers.contains(KeyModifiers::ALT) {
                bytes.insert(0, 27);
            }
            bytes
        }
        KeyCode::Enter => {
            #[cfg(unix)]
            {
                vec![b'\n']
            }
            #[cfg(windows)]
            {
                vec![b'\r', b'\n']
            }
        }
        KeyCode::Backspace => vec![8],
        KeyCode::Tab => vec![9],
        KeyCode::BackTab => vec![27, 91, 90],
        KeyCode::Esc => vec![27],
        KeyCode::Left => cursor_key(application_cursor, b'D'),
        KeyCode::Right => cursor_key(application_cursor, b'C'),
        KeyCode::Up => cursor_key(application_cursor, b'A'),
        KeyCode::Down => cursor_key(application_cursor, b'B'),
        KeyCode::Home => cursor_key(application_cursor, b'H'),
        KeyCode::End => cursor_key(application_cursor, b'F'),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        _ => return None,
    };
    Some(bytes)
}

fn cursor_key(application_cursor: bool, key: u8) -> Vec<u8> {
    if application_cursor {
        vec![27, b'O', key]
    } else {
        vec![27, b'[', key]
    }
}

fn control_character(character: char) -> Option<Vec<u8>> {
    let character = character.to_ascii_uppercase();
    let byte = match character {
        '@' | ' ' | '2' => 0,
        '[' | '3' => 27,
        '\\' | '4' => 28,
        ']' | '5' => 29,
        '^' | '6' => 30,
        '_' | '-' | '7' => 31,
        'A'..='Z' => character as u8 - 64,
        _ => return None,
    };
    Some(vec![byte])
}
