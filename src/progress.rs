use directories::ProjectDirs;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::models::Chapter;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub chapter_id: i32,
    pub scroll: u16,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct NovelProgress {
    chapter_id: Option<i32>,
    #[serde(default)]
    scroll: u16,
    #[serde(default)]
    bookmarks: Vec<Bookmark>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct ProgressFile {
    #[serde(default)]
    novels: HashMap<i32, NovelProgress>,
}

#[derive(Deserialize)]
struct LegacyProgressFile {
    #[serde(default)]
    chapters: HashMap<i32, i32>,
}

struct ProgressState {
    path: PathBuf,
    data: ProgressFile,
}

static PROGRESS_STATE: Lazy<Mutex<ProgressState>> = Lazy::new(|| {
    let path = progress_path();
    let data = load_from_disk(&path).unwrap_or_default();
    Mutex::new(ProgressState { path, data })
});

/// Returns the chapter index that should be selected for the supplied novel.
pub fn initial_index_for(novel_id: i32, chapters: &[Chapter], fallback_index: usize) -> usize {
    if chapters.is_empty() {
        return 0;
    }

    if let Some(chapter_id) = current_chapter_id(novel_id) {
        if let Some(index) = chapters.iter().position(|chapter| chapter.id == chapter_id) {
            return index;
        }
    }

    fallback_index.min(chapters.len().saturating_sub(1))
}

/// Returns the saved scroll offset for a chapter when scroll persistence is enabled.
pub fn initial_scroll_for(novel_id: i32, chapter_id: i32, scroll_persist: bool) -> u16 {
    if !scroll_persist {
        return 0;
    }

    let state = PROGRESS_STATE
        .lock()
        .expect("progress state mutex poisoned");
    let Some(progress) = state.data.novels.get(&novel_id) else {
        return 0;
    };

    if progress.chapter_id == Some(chapter_id) {
        progress.scroll
    } else {
        0
    }
}

/// Records the current reader position for a novel.
pub fn record_reader_position(novel_id: i32, chapter_id: i32, scroll: u16, scroll_persist: bool) {
    if let Err(error) = record_position(novel_id, chapter_id, scroll, scroll_persist) {
        eprintln!("Failed to persist reading progress: {}", error);
    }
}

pub fn bookmarks_for(novel_id: i32) -> Vec<Bookmark> {
    let state = PROGRESS_STATE
        .lock()
        .expect("progress state mutex poisoned");
    state
        .data
        .novels
        .get(&novel_id)
        .map(|progress| progress.bookmarks.clone())
        .unwrap_or_default()
}

pub fn add_bookmark(novel_id: i32, chapter_id: i32, scroll: u16) -> bool {
    let mut state = PROGRESS_STATE
        .lock()
        .expect("progress state mutex poisoned");
    match add_bookmark_inner(&mut state, novel_id, chapter_id, scroll) {
        Ok(added) => added,
        Err(error) => {
            eprintln!("Failed to persist bookmark: {}", error);
            false
        }
    }
}

pub fn delete_bookmark(novel_id: i32, index: usize) -> bool {
    let mut state = PROGRESS_STATE
        .lock()
        .expect("progress state mutex poisoned");
    let Some(progress) = state.data.novels.get_mut(&novel_id) else {
        return false;
    };
    if index >= progress.bookmarks.len() {
        return false;
    }

    progress.bookmarks.remove(index);
    if let Err(error) = persist(&state) {
        eprintln!("Failed to delete bookmark: {}", error);
        false
    } else {
        true
    }
}

fn current_chapter_id(novel_id: i32) -> Option<i32> {
    let state = PROGRESS_STATE
        .lock()
        .expect("progress state mutex poisoned");
    state
        .data
        .novels
        .get(&novel_id)
        .and_then(|progress| progress.chapter_id)
}

fn record_position(
    novel_id: i32,
    chapter_id: i32,
    scroll: u16,
    scroll_persist: bool,
) -> io::Result<()> {
    let mut state = PROGRESS_STATE
        .lock()
        .expect("progress state mutex poisoned");
    let progress = state.data.novels.entry(novel_id).or_default();
    progress.chapter_id = Some(chapter_id);
    progress.scroll = if scroll_persist { scroll } else { 0 };
    persist(&state)
}

fn add_bookmark_inner(
    state: &mut ProgressState,
    novel_id: i32,
    chapter_id: i32,
    scroll: u16,
) -> io::Result<bool> {
    let progress = state.data.novels.entry(novel_id).or_default();
    let bookmark = Bookmark { chapter_id, scroll };
    if progress.bookmarks.contains(&bookmark) {
        return Ok(false);
    }

    progress.bookmarks.push(bookmark);
    persist(state)?;
    Ok(true)
}

fn progress_path() -> PathBuf {
    if let Ok(custom) = env::var("READERTUI_PROGRESS_FILE") {
        return PathBuf::from(custom);
    }

    if let Some(project_dirs) = ProjectDirs::from("com", "novel-scraper", "readertui") {
        return project_dirs.data_dir().join("reading-progress.json");
    }

    PathBuf::from("reading-progress.json")
}

fn load_from_disk(path: &Path) -> io::Result<ProgressFile> {
    let content = fs::read_to_string(path)?;
    parse_progress(&content).map_err(std::io::Error::other)
}

fn parse_progress(content: &str) -> serde_json::Result<ProgressFile> {
    let value: serde_json::Value = serde_json::from_str(content)?;
    if value.get("novels").is_some() {
        serde_json::from_value(value)
    } else {
        let legacy = serde_json::from_value::<LegacyProgressFile>(value)?;
        Ok(ProgressFile {
            novels: legacy
                .chapters
                .into_iter()
                .map(|(novel_id, chapter_id)| {
                    (
                        novel_id,
                        NovelProgress {
                            chapter_id: Some(chapter_id),
                            scroll: 0,
                            bookmarks: Vec::new(),
                        },
                    )
                })
                .collect(),
        })
    }
}

fn persist(state: &ProgressState) -> io::Result<()> {
    if let Some(parent) = state.path.parent() {
        fs::create_dir_all(parent)?;
    }

    let serialized = serde_json::to_string_pretty(&state.data).map_err(std::io::Error::other)?;

    fs::write(&state.path, serialized)
}

/// Seeds the progress file using the supplied fallback when no entry exists yet.
pub fn seed_if_absent(novel_id: i32, chapter: Option<&Chapter>) {
    let has_entry = {
        let state = PROGRESS_STATE
            .lock()
            .expect("progress state mutex poisoned");
        state.data.novels.contains_key(&novel_id)
    };

    if has_entry {
        return;
    }

    if let Some(chapter) = chapter {
        if let Err(error) = record_position(novel_id, chapter.id, 0, true) {
            eprintln!("Failed to seed progress file: {}", error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_chapter_map() {
        let progress = parse_progress(r#"{"chapters":{"7":42}}"#).unwrap();
        let novel = progress.novels.get(&7).unwrap();
        assert_eq!(novel.chapter_id, Some(42));
        assert_eq!(novel.scroll, 0);
        assert!(novel.bookmarks.is_empty());
    }

    #[test]
    fn parses_current_progress_shape() {
        let progress = parse_progress(
            r#"{"novels":{"7":{"chapter_id":42,"scroll":13,"bookmarks":[{"chapter_id":42,"scroll":8}]}}}"#,
        )
        .unwrap();
        let novel = progress.novels.get(&7).unwrap();
        assert_eq!(novel.chapter_id, Some(42));
        assert_eq!(novel.scroll, 13);
        assert_eq!(
            novel.bookmarks,
            vec![Bookmark {
                chapter_id: 42,
                scroll: 8
            }]
        );

        let serialized = serde_json::to_string(&progress).unwrap();
        assert_eq!(parse_progress(&serialized).unwrap(), progress);
    }

    #[test]
    fn rejects_corrupt_progress_for_safe_fallback() {
        assert!(parse_progress("{not json").is_err());
    }

    #[test]
    fn bookmark_add_delete_and_deduplicate() {
        let path = env::temp_dir().join("readertui-test-progress.json");
        let mut state = ProgressState {
            path: path.clone(),
            data: ProgressFile::default(),
        };

        assert!(add_bookmark_inner(&mut state, 1, 2, 3).unwrap());
        assert!(!add_bookmark_inner(&mut state, 1, 2, 3).unwrap());
        assert!(add_bookmark_inner(&mut state, 1, 2, 4).unwrap());

        let progress = state.data.novels.get_mut(&1).unwrap();
        progress.bookmarks.remove(0);
        assert_eq!(
            progress.bookmarks,
            vec![Bookmark {
                chapter_id: 2,
                scroll: 4
            }]
        );

        let _ = fs::remove_file(path);
    }
}
