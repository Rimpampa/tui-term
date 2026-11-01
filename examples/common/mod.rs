//! Shared helpers for the examples.

/// Queries the host terminal for its background color.
///
/// Returns `None` when the terminal does not support the query, for example
/// when the example is run without an interactive terminal. Callers should
/// fall back to a sensible default in that case.
pub fn host_background_color() -> Option<(u8, u8, u8)> {
    terminal_colorsaurus::background_color(terminal_colorsaurus::QueryOptions::default())
        .ok()
        .map(|color| color.scale_to_8bit())
}
