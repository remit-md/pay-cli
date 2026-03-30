// Error handling uses anyhow throughout.
// This module provides convenience helpers for structured CLI error output.

use std::io::{self, IsTerminal, Write};

/// Print a success message to stdout: "✓ message"
pub fn success(msg: &str) {
    println!("\u{2713} {msg}");
}

/// Print an error message to stderr: "✗ Error: message"
pub fn error(msg: &str) {
    eprintln!("\u{2717} Error: {msg}");
}

/// Print key-value pairs as a simple indented list.
pub fn print_kv(pairs: &[(&str, &str)]) {
    for (k, v) in pairs {
        println!("  {k}: {v}");
    }
}

/// Print JSON to stdout (pretty-printed).
pub fn print_json<T: serde::Serialize>(value: &T) {
    if let Ok(json) = serde_json::to_string_pretty(value) {
        println!("{json}");
    }
}

/// Check if stdout is a terminal (for color/formatting decisions).
pub fn is_terminal() -> bool {
    io::stdout().is_terminal()
}

/// Flush stdout (useful after printing without newline).
pub fn flush() {
    let _ = io::stdout().flush();
}
