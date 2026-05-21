use once_cell::sync::Lazy;
use ratatui::style::Color;
use ratatui::text::Text;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

struct LogState {
    file: Option<File>,
    last_content_hash: Option<u64>,
}

static DEBUG_STATE: Lazy<Mutex<LogState>> = Lazy::new(|| {
    Mutex::new(LogState {
        file: None,
        last_content_hash: None,
    })
});

/// Initializes the debug log when explicitly enabled.
pub fn init(enabled: bool, configured_path: &str) {
    let mut state = DEBUG_STATE.lock().expect("debug log mutex poisoned");
    state.file = None;
    state.last_content_hash = None;

    let configured_path = if configured_path.trim().is_empty() {
        "readertui-debug.log"
    } else {
        configured_path
    };
    let env_path = std::env::var("READERTUI_DEBUG_LOG").ok();
    let Some(path) = env_path.or_else(|| enabled.then(|| configured_path.to_owned())) else {
        return;
    };

    let file = match File::create(Path::new(&path)) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("Failed to create debug log '{}': {}", path, error);
            return;
        }
    };
    state.file = Some(file);
}

/// Logs the supplied chapter content if it differs from the previously captured content.
///
/// The output wraps coloured segments using XML-like tags such as `<RED>...</RED>`.
/// Each invocation overwrites the existing file so that it only contains the
/// latest captured chapter.
pub fn log_chapter_content(novel_title: &str, chapter_title: &str, raw: &str, coloured: &Text<'_>) {
    let mut state = DEBUG_STATE.lock().expect("debug log mutex poisoned");
    if state.file.is_none() {
        return;
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut hasher);
    let fingerprint = hasher.finish();

    if state.last_content_hash == Some(fingerprint) {
        return;
    }
    state.last_content_hash = Some(fingerprint);

    let file = match state.file.as_mut() {
        Some(file) => file,
        None => return,
    };

    if let Err(error) = file.set_len(0) {
        eprintln!("Failed to truncate debug log: {}", error);
        return;
    }
    if let Err(error) = file.seek(SeekFrom::Start(0)) {
        eprintln!("Failed to rewind debug log: {}", error);
        return;
    }

    if let Err(error) = writeln!(file, "Novel: {}", novel_title) {
        eprintln!("Failed writing to debug log: {}", error);
        return;
    }
    if let Err(error) = writeln!(file, "Chapter: {}", chapter_title) {
        eprintln!("Failed writing to debug log: {}", error);
        return;
    }
    if let Err(error) = writeln!(file) {
        eprintln!("Failed writing to debug log: {}", error);
        return;
    }

    if let Err(error) = writeln!(file, "--- Normalized Text ---") {
        eprintln!("Failed writing to debug log: {}", error);
        return;
    }
    if let Err(error) = writeln!(file, "{}", raw) {
        eprintln!("Failed writing to debug log: {}", error);
        return;
    }
    if !raw.ends_with('\n') {
        if let Err(error) = writeln!(file) {
            eprintln!("Failed writing to debug log: {}", error);
            return;
        }
    }
    if let Err(error) = writeln!(file, "--- Coloured Output ---") {
        eprintln!("Failed writing to debug log: {}", error);
        return;
    }

    for line in &coloured.lines {
        let mut buffer = String::new();
        for span in &line.spans {
            let color = span.style.fg.unwrap_or(Color::White);
            let colour_tag = colour_tag_name(color);
            buffer.push_str(&format!(
                "<{tag}>{content}</{tag}>",
                tag = colour_tag,
                content = span.content.as_ref()
            ));
        }
        if let Err(error) = writeln!(file, "{}", buffer) {
            eprintln!("Failed writing to debug log: {}", error);
            return;
        }
    }

    if let Err(error) = file.flush() {
        eprintln!("Failed to flush debug log: {}", error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn disabled_debug_init_does_not_create_log_file() {
        let path = "/tmp/readertui-debug-disabled-test.log";
        let _ = fs::remove_file(path);

        init(false, path);

        assert!(!Path::new(path).exists());
    }

    #[test]
    fn enabled_debug_init_creates_log_file() {
        let path = "/tmp/readertui-debug-enabled-test.log";
        let _ = fs::remove_file(path);

        init(true, path);

        assert!(Path::new(path).exists());
        let _ = fs::remove_file(path);
        init(false, path);
    }
}
fn colour_tag_name(colour: Color) -> String {
    match colour {
        Color::Reset => "RESET".into(),
        Color::Black => "BLACK".into(),
        Color::Red => "RED".into(),
        Color::Green => "GREEN".into(),
        Color::Yellow => "YELLOW".into(),
        Color::Blue => "BLUE".into(),
        Color::Magenta => "MAGENTA".into(),
        Color::Cyan => "CYAN".into(),
        Color::Gray => "GRAY".into(),
        Color::DarkGray => "DARK_GRAY".into(),
        Color::LightRed => "LIGHT_RED".into(),
        Color::LightGreen => "LIGHT_GREEN".into(),
        Color::LightYellow => "LIGHT_YELLOW".into(),
        Color::LightBlue => "LIGHT_BLUE".into(),
        Color::LightMagenta => "LIGHT_MAGENTA".into(),
        Color::LightCyan => "LIGHT_CYAN".into(),
        Color::White => "WHITE".into(),
        Color::Indexed(idx) => format!("INDEXED({})", idx),
        Color::Rgb(r, g, b) => format!("RGB({},{},{})", r, g, b),
    }
}
