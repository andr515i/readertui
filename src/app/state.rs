use ratatui::widgets::ListState;

use crate::models::{Chapter, Novel};

/// Tracks whether the inline search is active and the current query.
pub struct StatefulSearch {
    pub active: bool,
    pub input: String,
    pub previous_selection: Option<usize>,
}

impl StatefulSearch {
    pub fn new() -> Self {
        Self {
            active: false,
            input: String::new(),
            previous_selection: None,
        }
    }

    pub fn begin(&mut self, previous_selection: Option<usize>) {
        self.active = true;
        self.input.clear();
        self.previous_selection = previous_selection;
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.input.clear();
        self.previous_selection = None;
    }
}

/// Maintains in-progress key combinations (e.g. `12j`, `gg`).
#[derive(Default)]
pub struct KeyState {
    pub pending_count: Option<usize>,
    pub seen_g: bool,
}

impl KeyState {
    pub fn push_digit(&mut self, digit: u8) {
        let value = (digit - b'0') as usize;
        self.pending_count = Some(self.pending_count.unwrap_or(0).saturating_mul(10) + value);
    }

    pub fn reset(&mut self) {
        self.pending_count = None;
        self.seen_g = false;
    }

    pub fn take_count(&mut self) -> usize {
        let count = self.pending_count.take().unwrap_or(1);
        self.seen_g = false;
        count.max(1)
    }
}

#[derive(Default)]
pub struct BookmarkOverlayState {
    pub active: bool,
    pub selected: Option<usize>,
}

impl BookmarkOverlayState {
    pub fn open(&mut self, bookmark_count: usize) {
        self.active = true;
        self.selected = if bookmark_count == 0 { None } else { Some(0) };
    }

    pub fn close(&mut self) {
        self.active = false;
        self.selected = None;
    }
}

pub fn move_selection_wrapping(
    selected: Option<usize>,
    len: usize,
    amount: usize,
    forward: bool,
) -> Option<usize> {
    if len == 0 {
        return None;
    }

    let Some(current) = selected else {
        return Some(0);
    };
    let current = current % len;
    let step = amount % len;
    if forward {
        Some((current + step) % len)
    } else {
        Some((current + len - step) % len)
    }
}

pub fn clamp_scroll(scroll: u16, max_scroll: u16) -> u16 {
    scroll.min(max_scroll)
}

pub fn max_scroll_for(rendered_lines: usize, visible_height: u16) -> u16 {
    rendered_lines
        .saturating_sub(visible_height as usize)
        .min(u16::MAX as usize) as u16
}

/// Represents the top-level navigation state for the application.
pub enum AppMode {
    ChapterState {
        novel: Novel,
        chapters: Vec<Chapter>,
        filtered: Vec<Chapter>,
        list_state: ListState,
    },
    ReaderState {
        novel: Novel,
        chapters: Vec<Chapter>,
        current_index: usize,
        scroll: u16,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_prefix_is_taken_once_and_resets_g() {
        let mut keys = KeyState::default();
        keys.push_digit(b'1');
        keys.push_digit(b'2');
        keys.seen_g = true;

        assert_eq!(keys.take_count(), 12);
        assert_eq!(keys.take_count(), 1);
        assert!(!keys.seen_g);
    }

    #[test]
    fn list_movement_wraps_for_large_counts() {
        assert_eq!(move_selection_wrapping(Some(1), 4, 6, true), Some(3));
        assert_eq!(move_selection_wrapping(Some(1), 4, 6, false), Some(3));
        assert_eq!(move_selection_wrapping(None, 4, 1, true), Some(0));
        assert_eq!(move_selection_wrapping(Some(0), 0, 3, true), None);
    }

    #[test]
    fn scroll_helpers_clamp_to_visible_range() {
        assert_eq!(max_scroll_for(100, 20), 80);
        assert_eq!(max_scroll_for(10, 20), 0);
        assert_eq!(clamp_scroll(90, 80), 80);
    }
}
