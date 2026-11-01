//! Helpers for driving a terminal screen from an already-created PTY.
//!
//! `tui-term` intentionally does not create pseudo-terminals. If you already
//! have a reader and a writer for a PTY, [`PtyStream`] connects them to a
//! [`vt100::Parser`] and exposes the parsed screen for rendering.
//!
//! The reader is consumed by a background thread. Input bytes are written to
//! the writer synchronously, so callers that need non-blocking input can put
//! their own channel or task between the UI and [`PtyStream::write_input`].
//!
//! # Example
//!
//! ```rust,no_run
//! use std::io;
//!
//! use tui_term::stream::{PtyStream, Size};
//!
//! # fn open_pty() -> io::Result<(std::io::Cursor<Vec<u8>>, Vec<u8>)> {
//! #     Ok((std::io::Cursor::new(Vec::new()), Vec::new()))
//! # }
//! let (reader, writer) = open_pty()?;
//! let terminal = PtyStream::new(reader, writer, Size::new(24, 80))?;
//! terminal.write_input(b"ls\n")?;
//! let screen = terminal.screen();
//! # Ok::<(), io::Error>(())
//! ```

use alloc::sync::Arc;
use std::{
    io::{self, Read, Write},
    sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard},
    thread::{self, JoinHandle},
};

use vt100::{Parser, Screen};

/// The number of rows and columns used to initialize the terminal parser.
///
/// The fields are deliberately named to make the order explicit. This is the
/// same order used by [`vt100::Parser::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    /// Number of visible rows.
    pub rows: u16,
    /// Number of visible columns.
    pub cols: u16,
}

impl Size {
    /// Creates a new terminal size.
    #[must_use]
    pub const fn new(rows: u16, cols: u16) -> Self {
        Self { rows, cols }
    }
}

impl Default for Size {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

/// A terminal screen fed by an existing PTY output stream.
///
/// `PtyStream` owns the reader, the writer, and a parser. The reader is moved
/// to a background thread which continuously feeds output into the parser.
/// The parsed screen can be rendered with
/// [`PseudoTerminal`](crate::widget::PseudoTerminal).
///
/// The PTY itself is not managed by this type. Creating, resizing, and closing
/// the PTY remains the responsibility of the caller; call [`PtyStream::resize`]
/// in addition to resizing the PTY so the parser stays in sync.
pub struct PtyStream<W> {
    parser: Arc<RwLock<Parser>>,
    writer: Mutex<W>,
    reader: Option<JoinHandle<io::Result<()>>>,
}

impl<W: Write + Send> PtyStream<W> {
    /// Creates a new stream with no scrollback.
    ///
    /// `reader` should be the output side of the PTY (the bytes produced by
    /// the child process). `writer` should be the input side (the bytes sent
    /// to the child process).
    ///
    /// # Errors
    ///
    /// Returns an error if the background reader thread cannot be spawned.
    pub fn new<R>(reader: R, writer: W, size: Size) -> io::Result<Self>
    where
        R: Read + Send + 'static,
    {
        Self::with_scrollback(reader, writer, size, 0)
    }

    /// Creates a new stream with a bounded scrollback buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the background reader thread cannot be spawned.
    pub fn with_scrollback<R>(
        reader: R,
        writer: W,
        size: Size,
        scrollback_len: usize,
    ) -> io::Result<Self>
    where
        R: Read + Send + 'static,
    {
        let parser = Arc::new(RwLock::new(Parser::new(
            size.rows,
            size.cols,
            scrollback_len,
        )));
        let reader_parser = Arc::clone(&parser);
        let reader = thread::Builder::new()
            .name("tui-term-pty-reader".into())
            .spawn(move || read_loop(reader, reader_parser))?;

        Ok(Self {
            parser,
            writer: Mutex::new(writer),
            reader: Some(reader),
        })
    }

    /// Returns a copy of the currently parsed screen.
    ///
    /// For drawing, prefer [`PtyStream::with_screen`] to avoid cloning the
    /// screen on every frame.
    #[must_use]
    pub fn screen(&self) -> Screen {
        self.read_parser().screen().clone()
    }

    /// Calls `f` with the currently parsed screen without cloning it.
    pub fn with_screen<R>(&self, f: impl FnOnce(&Screen) -> R) -> R {
        f(self.read_parser().screen())
    }

    /// Calls `f` with mutable access to the parsed screen.
    ///
    /// This can be used for operations such as changing the scrollback
    /// position.
    pub fn with_screen_mut<R>(&self, f: impl FnOnce(&mut Screen) -> R) -> R {
        f(self.write_parser().screen_mut())
    }

    /// Returns the shared parser.
    ///
    /// This is an escape hatch for more advanced use cases. Most callers
    /// should use [`PtyStream::with_screen`] or
    /// [`PtyStream::with_screen_mut`] instead.
    #[must_use]
    pub fn parser(&self) -> &Arc<RwLock<Parser>> {
        &self.parser
    }

    /// Returns the current parser size as `(rows, cols)`.
    #[must_use]
    pub fn size(&self) -> (u16, u16) {
        self.read_parser().screen().size()
    }

    /// Resizes the parser.
    ///
    /// This does not resize the PTY. Resize the PTY through its own handle as
    /// well, otherwise the child process will keep using the old size.
    pub fn resize(&self, rows: u16, cols: u16) {
        self.write_parser().screen_mut().set_size(rows, cols);
    }

    /// Sets the parser's scrollback position.
    pub fn set_scrollback(&self, rows: usize) {
        self.write_parser().screen_mut().set_scrollback(rows);
    }

    /// Writes bytes to the PTY input stream and flushes the writer.
    ///
    /// # Errors
    ///
    /// Returns any error reported by the underlying writer.
    pub fn write_input(&self, bytes: &[u8]) -> io::Result<()> {
        let mut writer = self.writer();
        writer.write_all(bytes)?;
        writer.flush()
    }

    /// Locks and returns the underlying writer.
    ///
    /// The writer is behind a mutex so input can be sent with `&self`, even
    /// while the screen is being rendered.
    pub fn writer(&self) -> MutexGuard<'_, W> {
        self.writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns whether the background reader has finished.
    ///
    /// The reader finishes when the PTY output reaches end-of-file or the
    /// reader returns an error.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.reader.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Waits for the background reader to finish.
    ///
    /// # Errors
    ///
    /// Returns the error reported by the reader, or an error if the reader
    /// thread panicked.
    pub fn wait(&mut self) -> io::Result<()> {
        match self.reader.take() {
            Some(reader) => reader
                .join()
                .map_err(|_| io::Error::other("the tui-term PTY reader thread panicked"))?,
            None => Ok(()),
        }
    }

    fn read_parser(&self) -> RwLockReadGuard<'_, Parser> {
        self.parser
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write_parser(&self) -> RwLockWriteGuard<'_, Parser> {
        self.parser
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn read_loop<R: Read>(mut reader: R, parser: Arc<RwLock<Parser>>) -> io::Result<()> {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                let mut parser = parser
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                parser.process(&buf[..n]);
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn stream(input: &'static [u8]) -> PtyStream<Vec<u8>> {
        PtyStream::new(Cursor::new(input), Vec::new(), Size::new(3, 10)).unwrap()
    }

    fn wait_for<F: Fn() -> bool>(condition: F) {
        for _ in 0..1000 {
            if condition() {
                return;
            }
            std::thread::yield_now();
        }
        panic!("condition was not met");
    }

    #[test]
    fn parses_reader_output_in_background() {
        let mut stream = stream(b"hello\r\nworld");
        wait_for(|| stream.finished());
        let screen = stream.screen();
        assert!(screen.contents().contains("hello"));
        assert!(screen.contents().contains("world"));
        stream.wait().unwrap();
    }

    #[test]
    fn writes_input_to_writer() {
        let stream = stream(b"");
        stream.write_input(b"abc").unwrap();
        stream.write_input(b"def").unwrap();
        let writer = stream.writer();
        assert_eq!(&*writer, b"abcdef");
    }

    #[test]
    fn resizes_parser() {
        let stream = stream(b"");
        stream.resize(10, 20);
        assert_eq!(stream.size(), (10, 20));
        stream.with_screen_mut(|screen| screen.set_scrollback(0));
    }
}
