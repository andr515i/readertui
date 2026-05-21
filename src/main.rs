use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use rusqlite::Result;
use std::hash::{Hash, Hasher};
use std::io;
use std::time::{Duration, Instant};

mod app;
mod config;
mod db;
mod debug;
mod models;
mod progress;
mod text;
mod ui;

use app::{
    clamp_scroll, max_scroll_for, move_selection_wrapping, AppMode, BookmarkOverlayState, KeyState,
    StatefulSearch,
};
use config::{Config, ReaderConfig, ScrollConfig};
use db::{fetch_chapters, fetch_novels, Database};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use models::Chapter;
use progress::{
    add_bookmark, bookmarks_for, delete_bookmark, initial_index_for, initial_scroll_for,
    record_reader_position, seed_if_absent, Bookmark,
};
use text::{colorize_text, fix_wording, format_html_to_text, HighlightMatcher};

#[derive(Clone, Copy, Debug, Default)]
struct ReaderMetrics {
    rendered_lines: usize,
    visible_height: u16,
    max_scroll: u16,
    scroll: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PreparedChapterKey {
    novel_id: i32,
    chapter_id: i32,
    content_hash: u64,
    highlight_hash: u64,
}

#[derive(Debug, Default)]
struct PreparedChapterCache {
    key: Option<PreparedChapterKey>,
    content: String,
    text: Text<'static>,
    measured_width: u16,
    rendered_lines: usize,
}

#[derive(Clone, Copy, Debug)]
struct PendingReaderPosition {
    novel_id: i32,
    chapter_id: i32,
    scroll: u16,
    updated_at: Instant,
}

#[derive(Debug, Default)]
struct PendingReaderProgress {
    position: Option<PendingReaderPosition>,
}

fn delete_last_word(buffer: &mut String) {
    if buffer.is_empty() {
        return;
    }

    let trimmed_len = buffer.trim_end_matches(char::is_whitespace).len();
    if trimmed_len == 0 {
        buffer.clear();
        return;
    }

    let mut truncate_at = 0usize;
    let slice = &buffer[..trimmed_len];
    let mut found_boundary = false;
    for (idx, ch) in slice.char_indices().rev() {
        if ch.is_whitespace() {
            truncate_at = idx + ch.len_utf8();
            found_boundary = true;
            break;
        }
    }

    if !found_boundary {
        buffer.clear();
    } else {
        buffer.truncate(truncate_at);
    }
}

fn is_numeric_query(query: &str) -> bool {
    !query.is_empty() && query.chars().all(|character| character.is_ascii_digit())
}

fn leading_digits(value: &str) -> Option<&str> {
    let mut end = 0;
    for (index, character) in value.char_indices() {
        if character.is_ascii_digit() {
            end = index + character.len_utf8();
        } else {
            break;
        }
    }

    if end == 0 {
        None
    } else {
        Some(&value[..end])
    }
}

fn displayed_chapter_number(title: &str) -> Option<&str> {
    const CHAPTER_PREFIX: &str = "chapter";

    let trimmed = title.trim_start();
    if let Some(prefix) = trimmed.get(..CHAPTER_PREFIX.len()) {
        if prefix.eq_ignore_ascii_case(CHAPTER_PREFIX) {
            let rest = trimmed[CHAPTER_PREFIX.len()..].trim_start();
            if let Some(number) = leading_digits(rest) {
                return Some(number);
            }
        }
    }

    leading_digits(trimmed)
}

fn numeric_chapter_score(title: &str, query: &str) -> Option<(u8, usize, u64)> {
    let number = displayed_chapter_number(title)?;
    let parsed = number.parse::<u64>().unwrap_or(u64::MAX);

    if number == query {
        Some((0, number.len(), parsed))
    } else if number.starts_with(query) {
        Some((1, number.len(), parsed))
    } else if number.contains(query) {
        Some((2, number.len(), parsed))
    } else {
        None
    }
}

fn filter_chapters(chapters: &[Chapter], query: &str) -> Vec<Chapter> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return chapters.to_vec();
    }

    if is_numeric_query(trimmed) {
        let mut ranked: Vec<((u8, usize, u64, usize), &Chapter)> = chapters
            .iter()
            .enumerate()
            .filter_map(|(index, chapter)| {
                numeric_chapter_score(chapter.title.as_str(), trimmed)
                    .map(|(rank, len, number)| ((rank, len, number, index), chapter))
            })
            .collect();

        ranked.sort_by(|a, b| a.0.cmp(&b.0));
        return ranked
            .into_iter()
            .map(|(_, chapter)| chapter.clone())
            .collect();
    }

    let matcher = SkimMatcherV2::default();
    let lower = trimmed.to_lowercase();
    let numeric = trimmed.parse::<i32>().ok();

    let mut ranked: Vec<(i64, &Chapter)> = Vec::new();

    for chapter in chapters {
        let mut best_score: Option<i64> = None;

        if let Some(num) = numeric {
            if chapter.id == num {
                best_score = Some(2_000_000);
            }
        }

        let title_lower = chapter.title.to_lowercase();
        if title_lower.contains(&lower) {
            best_score = Some(best_score.map_or(1_000_000, |current| current.max(1_000_000)));
        }

        let id_str = chapter.id.to_string();
        if id_str.contains(trimmed) {
            best_score = Some(best_score.map_or(1_100_000, |current| current.max(1_100_000)));
        }

        if let Some(score) = matcher.fuzzy_match(chapter.title.as_str(), trimmed) {
            best_score = Some(best_score.map_or(score, |current| current.max(score)));
        }

        let title_with_id = format!("{} {}", chapter.id, chapter.title);
        if let Some(score) = matcher.fuzzy_match(title_with_id.as_str(), trimmed) {
            best_score = Some(best_score.map_or(score, |current| current.max(score)));
        }

        if let Some(score) = matcher.fuzzy_match(id_str.as_str(), trimmed) {
            best_score = Some(best_score.map_or(score, |current| current.max(score)));
        }

        if let Some(score) = best_score {
            ranked.push((score, chapter));
        }
    }

    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    ranked
        .into_iter()
        .map(|(_, chapter)| chapter.clone())
        .collect()
}

fn update_filtered_list(
    chapters: &[Chapter],
    filtered: &mut Vec<Chapter>,
    list_state: &mut ListState,
    query: &str,
) {
    let results = filter_chapters(chapters, query);
    filtered.clear();
    filtered.extend(results);

    if filtered.is_empty() {
        list_state.select(None);
    } else {
        let current = list_state.selected().unwrap_or(0).min(filtered.len() - 1);
        list_state.select(Some(current));
    }
}

fn chapter_content_hash(chapter: &Chapter) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    chapter.id.hash(&mut hasher);
    chapter.title.hash(&mut hasher);
    chapter.content.hash(&mut hasher);
    hasher.finish()
}

fn reader_text_width(area: Rect) -> u16 {
    area.width.saturating_sub(2).max(1)
}

fn reader_text_height(area: Rect) -> u16 {
    area.height.saturating_sub(2).max(1)
}

fn visible_line_range(rendered_lines: usize, visible_height: u16, scroll: u16) -> (usize, usize) {
    if rendered_lines == 0 {
        return (0, 0);
    }

    let first = scroll as usize + 1;
    let last = (scroll as usize + visible_height as usize).min(rendered_lines);
    (first, last)
}

impl PreparedChapterCache {
    fn ensure(
        &mut self,
        novel_id: i32,
        novel_title: &str,
        chapter: &Chapter,
        width: u16,
        color_theme: &config::ColorTheme,
        highlight_matcher: &HighlightMatcher,
    ) {
        let key = PreparedChapterKey {
            novel_id,
            chapter_id: chapter.id,
            content_hash: chapter_content_hash(chapter),
            highlight_hash: highlight_matcher.fingerprint(),
        };

        if self.key != Some(key) {
            let fixed_content = fix_wording(&chapter.content);
            self.content = format_html_to_text(&fixed_content);
            self.text = colorize_text(self.content.as_str(), color_theme, highlight_matcher);
            self.key = Some(key);
            self.measured_width = 0;
            self.rendered_lines = 0;

            debug::log_chapter_content(novel_title, &chapter.title, &self.content, &self.text);
        }

        if self.measured_width != width {
            self.rendered_lines = Paragraph::new(self.text.clone())
                .wrap(Wrap { trim: true })
                .line_count(width);
            self.measured_width = width;
        }
    }
}

impl PendingReaderProgress {
    fn schedule(
        &mut self,
        novel_id: i32,
        chapter_id: i32,
        scroll: u16,
        now: Instant,
        scroll_persist: bool,
    ) {
        self.position = if scroll_persist {
            Some(PendingReaderPosition {
                novel_id,
                chapter_id,
                scroll,
                updated_at: now,
            })
        } else {
            None
        };
    }

    fn flush_due(&mut self, now: Instant, delay: Duration, scroll_persist: bool) -> bool {
        if self.due_position(now, delay).is_none() {
            return false;
        }

        self.flush_now(scroll_persist)
    }

    fn due_position(&self, now: Instant, delay: Duration) -> Option<PendingReaderPosition> {
        let position = self.position?;
        if now.duration_since(position.updated_at) < delay {
            None
        } else {
            Some(position)
        }
    }

    fn flush_now(&mut self, scroll_persist: bool) -> bool {
        let Some(position) = self.position.take() else {
            return false;
        };

        record_reader_position(
            position.novel_id,
            position.chapter_id,
            position.scroll,
            scroll_persist,
        );
        true
    }

    fn clear(&mut self) {
        self.position = None;
    }
}

fn scaled_step(step: u16, count: usize) -> u16 {
    (step as usize)
        .saturating_mul(count.max(1))
        .min(u16::MAX as usize) as u16
}

fn page_step(scroll_settings: &ScrollConfig, visible_height: u16) -> u16 {
    scroll_settings
        .page_step
        .unwrap_or_else(|| visible_height.saturating_sub(2).max(1))
        .max(1)
}

fn scroll_down(scroll: u16, amount: u16, max_scroll: u16) -> u16 {
    clamp_scroll(scroll.saturating_add(amount), max_scroll)
}

fn scroll_up(scroll: u16, amount: u16) -> u16 {
    scroll.saturating_sub(amount)
}

fn persist_reader(
    novel_id: i32,
    chapters: &[Chapter],
    current_index: usize,
    scroll: u16,
    reader_settings: &ReaderConfig,
) {
    if let Some(chapter) = chapters.get(current_index) {
        record_reader_position(novel_id, chapter.id, scroll, reader_settings.scroll_persist);
    }
}

fn schedule_reader_progress(
    pending_progress: &mut PendingReaderProgress,
    novel_id: i32,
    chapters: &[Chapter],
    current_index: usize,
    scroll: u16,
    reader_settings: &ReaderConfig,
) {
    if let Some(chapter) = chapters.get(current_index) {
        pending_progress.schedule(
            novel_id,
            chapter.id,
            scroll,
            Instant::now(),
            reader_settings.scroll_persist,
        );
    }
}

fn chapter_index_for(chapters: &[Chapter], chapter_id: i32) -> Option<usize> {
    chapters.iter().position(|chapter| chapter.id == chapter_id)
}

fn bookmark_label(bookmark: &Bookmark, chapters: &[Chapter]) -> String {
    let title = chapters
        .iter()
        .find(|chapter| chapter.id == bookmark.chapter_id)
        .map(|chapter| chapter.title.as_str())
        .unwrap_or("Unknown chapter");
    format!("{} | line {}", title, bookmark.scroll.saturating_add(1))
}

fn popup_area(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let width = ((area.width as u32 * percent_x as u32) / 100)
        .max(32)
        .min(area.width as u32) as u16;
    let height = ((area.height as u32 * percent_y as u32) / 100)
        .max(7)
        .min(area.height as u32) as u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

fn footer_text(status_message: &Option<String>, fallback: String) -> String {
    status_message.clone().unwrap_or(fallback)
}

fn render_footer(frame: &mut ratatui::Frame<'_>, area: Rect, text: String) {
    let footer = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, area);
}

fn read_event_batch(first: Event) -> Vec<Event> {
    const MAX_EVENTS_PER_TICK: usize = 64;
    let mut events = vec![first];

    while events.len() < MAX_EVENTS_PER_TICK
        && event::poll(Duration::from_millis(0)).expect("Polling queued events failed")
    {
        events.push(event::read().expect("Reading queued event failed"));
    }

    events
}

fn render_bookmark_popup(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    chapters: &[Chapter],
    bookmarks: &[Bookmark],
    overlay: &mut BookmarkOverlayState,
) {
    let popup = popup_area(area, 70, 45);
    frame.render_widget(Clear, popup);

    if bookmarks.is_empty() {
        let empty = Paragraph::new("No bookmarks yet. Press m in the reader to save one.")
            .block(Block::default().borders(Borders::ALL).title("Bookmarks"))
            .wrap(Wrap { trim: true });
        frame.render_widget(empty, popup);
        overlay.selected = None;
        return;
    }

    let items = bookmarks
        .iter()
        .map(|bookmark| ListItem::new(bookmark_label(bookmark, chapters)))
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Bookmarks (Enter jump, d delete, Esc close)"),
        )
        .highlight_symbol(">> ")
        .highlight_style(Style::default().fg(Color::Yellow));

    let mut list_state = ListState::default();
    let selected = overlay
        .selected
        .unwrap_or(0)
        .min(bookmarks.len().saturating_sub(1));
    overlay.selected = Some(selected);
    list_state.select(Some(selected));
    frame.render_stateful_widget(list, popup, &mut list_state);
}

fn reader_footer(
    current_index: usize,
    chapter_count: usize,
    metrics: ReaderMetrics,
    overlay_active: bool,
) -> String {
    if overlay_active {
        return "Bookmarks | j/k move | Enter jump | d delete | Esc close".to_string();
    }

    let (first_visible, last_visible) = visible_line_range(
        metrics.rendered_lines,
        metrics.visible_height,
        metrics.scroll,
    );
    let percent = if metrics.rendered_lines == 0 {
        0
    } else {
        (last_visible.saturating_mul(100) / metrics.rendered_lines).min(100)
    };

    format!(
        "Chapter {}/{} | lines {}-{}/{} ({}%) | j/k wheel scroll | Space/Pg page | m mark | B bookmarks",
        current_index + 1,
        chapter_count,
        first_visible,
        last_visible,
        metrics.rendered_lines,
        percent
    )
}

fn chapter_footer(filtered: &[Chapter], selected: Option<usize>, search_active: bool) -> String {
    let position = selected.map(|index| index + 1).unwrap_or(0);
    let mode = if search_active {
        "Esc cancel | Enter keep | Up/Down move"
    } else {
        "/ search | Enter read | h novels | gg/G top/bottom | q quit"
    };
    format!("Chapters {}/{} | {}", position, filtered.len(), mode)
}

#[tokio::main]
async fn main() -> Result<()> {
    let Config {
        scroll: scroll_settings,
        reader: reader_settings,
        database: database_settings,
        colors: color_theme,
        highlights: highlight_settings,
        debug: debug_settings,
    } = Config::load();
    debug::init(debug_settings.enabled, &debug_settings.log_path);
    let database =
        Database::from_config(&database_settings).expect("Failed to initialize database");

    enable_raw_mode().expect("Cannot enable raw mode");

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .expect("Cannot enter alternate screen");

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).expect("Failed to create terminal");

    let selected_novel_id =
        ui::home::run_home_screen(&mut terminal, &database, scroll_settings.mouse_step)
            .expect("Home screen failed");
    if selected_novel_id.is_none() {
        disable_raw_mode().expect("Cannot disable raw mode");
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .expect("Cannot leave alternate screen");
        return Ok(());
    }
    let selected_novel_id = selected_novel_id.unwrap();

    let mut keys = KeyState::default();
    let mut reader_metrics = ReaderMetrics::default();
    let mut bookmark_overlay = BookmarkOverlayState::default();
    let mut prepared_chapter = PreparedChapterCache::default();
    let mut pending_progress = PendingReaderProgress::default();
    let mut status_message: Option<String> = None;
    let progress_flush_delay = Duration::from_millis(250);

    let novels = fetch_novels(&database)
        .await
        .expect("Failed to fetch novels");
    let selected_novel = novels
        .into_iter()
        .find(|novel| novel.id == selected_novel_id)
        .expect("Selected novel not found");
    let mut active_highlights = HighlightMatcher::from_groups(
        &highlight_settings.active_groups_for(selected_novel.id, &selected_novel.title),
    );

    let (chapters, most_recent_index) =
        fetch_chapters(&database, selected_novel.id).expect("Failed to fetch chapters");

    seed_if_absent(selected_novel.id, chapters.get(most_recent_index));
    let list_index = initial_index_for(selected_novel.id, &chapters, most_recent_index);

    let mut chapter_list_state = ListState::default();
    if !chapters.is_empty() {
        chapter_list_state.select(Some(list_index));
    }

    let mut mode = AppMode::ChapterState {
        novel: selected_novel,
        chapters: chapters.clone(),
        filtered: chapters,
        list_state: chapter_list_state,
    };

    let mut search = StatefulSearch::new();
    let mut needs_redraw = true;

    'app: loop {
        pending_progress.flush_due(
            Instant::now(),
            progress_flush_delay,
            reader_settings.scroll_persist,
        );

        if needs_redraw {
            terminal
                .draw(|frame| {
                    let size = frame.area();
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
                        .split(size);
                    let body_area = chunks[0];
                    let footer_area = chunks[1];

                    match &mut mode {
                        AppMode::ChapterState {
                            novel,
                            chapters: _,
                            filtered,
                            list_state,
                        } => {
                            let header = if search.active {
                                format!("Searching for '{}'", search.input)
                            } else {
                                format!("Chapters for '{}'", novel.title)
                            };

                            let items: Vec<ListItem> = filtered
                                .iter()
                                .map(|chapter| ListItem::new(chapter.title.to_string()))
                                .collect();
                            let list = List::new(items)
                                .block(Block::default().borders(Borders::ALL).title(header))
                                .highlight_symbol(">> ")
                                .highlight_style(Style::default().fg(Color::Yellow));

                            if search.active {
                                let search_chunks = Layout::default()
                                    .direction(Direction::Vertical)
                                    .constraints(
                                        [Constraint::Min(1), Constraint::Length(3)].as_ref(),
                                    )
                                    .split(body_area);

                                frame.render_stateful_widget(list, search_chunks[0], list_state);

                                let input = Paragraph::new(search.input.as_str())
                                    .block(Block::default().borders(Borders::ALL).title("Search"))
                                    .wrap(Wrap { trim: true });
                                frame.render_widget(input, search_chunks[1]);
                            } else {
                                frame.render_stateful_widget(list, body_area, list_state);
                            }

                            render_footer(
                                frame,
                                footer_area,
                                footer_text(
                                    &status_message,
                                    chapter_footer(filtered, list_state.selected(), search.active),
                                ),
                            );
                        }
                        AppMode::ReaderState {
                            novel,
                            chapters,
                            current_index,
                            scroll,
                        } => {
                            let chapter = &chapters[*current_index];
                            let header = format!("Reading '{}' - {}", novel.title, chapter.title);
                            let text_width = reader_text_width(body_area);
                            let visible_height = reader_text_height(body_area);
                            prepared_chapter.ensure(
                                novel.id,
                                &novel.title,
                                chapter,
                                text_width,
                                &color_theme,
                                &active_highlights,
                            );

                            let paragraph = Paragraph::new(prepared_chapter.text.clone())
                                .block(Block::default().borders(Borders::ALL).title(header))
                                .wrap(Wrap { trim: true });
                            let rendered_lines = prepared_chapter.rendered_lines;
                            let max_scroll = max_scroll_for(rendered_lines, visible_height);
                            *scroll = clamp_scroll(*scroll, max_scroll);
                            reader_metrics = ReaderMetrics {
                                rendered_lines,
                                visible_height,
                                max_scroll,
                                scroll: *scroll,
                            };

                            frame.render_widget(paragraph.scroll((*scroll, 0)), body_area);

                            render_footer(
                                frame,
                                footer_area,
                                footer_text(
                                    &status_message,
                                    reader_footer(
                                        *current_index,
                                        chapters.len(),
                                        reader_metrics,
                                        bookmark_overlay.active,
                                    ),
                                ),
                            );

                            if bookmark_overlay.active {
                                let bookmarks = bookmarks_for(novel.id);
                                render_bookmark_popup(
                                    frame,
                                    body_area,
                                    chapters,
                                    &bookmarks,
                                    &mut bookmark_overlay,
                                );
                            }
                        }
                    }
                })
                .expect("Failed drawing UI");
            needs_redraw = false;
        }

        if !event::poll(Duration::from_millis(100)).expect("Polling error") {
            continue;
        }

        let first_event = event::read().expect("Reading event error");

        for input_event in read_event_batch(first_event) {
            needs_redraw = true;
            if let Event::Key(key) = input_event {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                let search_active = matches!(&mode, AppMode::ChapterState { .. }) && search.active;
                let overlay_active =
                    matches!(&mode, AppMode::ReaderState { .. }) && bookmark_overlay.active;
                if !search_active && !overlay_active {
                    if let KeyCode::Char(character) = key.code {
                        if character.is_ascii_digit() {
                            let digit = character as u8;
                            if digit != b'0' || keys.pending_count.is_some() {
                                keys.push_digit(digit);
                                continue;
                            }
                        }
                        if character == 'g' && !keys.seen_g {
                            keys.seen_g = true;
                            continue;
                        }
                    }
                }

                mode = match mode {
                    AppMode::ChapterState {
                        novel,
                        chapters,
                        mut filtered,
                        mut list_state,
                    } => {
                        if search.active {
                            match key.code {
                                KeyCode::Char(character) => {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        if character == 'w' {
                                            delete_last_word(&mut search.input);
                                        }
                                    } else {
                                        search.input.push(character);
                                    }
                                    update_filtered_list(
                                        &chapters,
                                        &mut filtered,
                                        &mut list_state,
                                        &search.input,
                                    );
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                KeyCode::Backspace => {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        delete_last_word(&mut search.input);
                                    } else {
                                        search.input.pop();
                                    }
                                    update_filtered_list(
                                        &chapters,
                                        &mut filtered,
                                        &mut list_state,
                                        &search.input,
                                    );
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                KeyCode::Down => {
                                    list_state.select(move_selection_wrapping(
                                        list_state.selected(),
                                        filtered.len(),
                                        1,
                                        true,
                                    ));
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                KeyCode::Up => {
                                    list_state.select(move_selection_wrapping(
                                        list_state.selected(),
                                        filtered.len(),
                                        1,
                                        false,
                                    ));
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                KeyCode::Delete => {
                                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                                        delete_last_word(&mut search.input);
                                        update_filtered_list(
                                            &chapters,
                                            &mut filtered,
                                            &mut list_state,
                                            &search.input,
                                        );
                                    }
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                KeyCode::Esc => {
                                    let previous = search.previous_selection;
                                    search.reset();
                                    filtered.clear();
                                    filtered.extend_from_slice(&chapters);
                                    list_state
                                        .select(previous.filter(|index| *index < filtered.len()));
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                KeyCode::Enter => {
                                    search.reset();
                                    if !filtered.is_empty() {
                                        let idx = list_state.selected().unwrap_or(0);
                                        let clamped = idx.min(filtered.len() - 1);
                                        list_state.select(Some(clamped));
                                    }
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                _ => AppMode::ChapterState {
                                    novel,
                                    chapters,
                                    filtered,
                                    list_state,
                                },
                            }
                        } else {
                            match key.code {
                                KeyCode::Char('/') => {
                                    status_message = None;
                                    search.begin(list_state.selected());
                                    filtered.clear();
                                    filtered.extend_from_slice(&chapters);
                                    list_state.select(None);
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                KeyCode::Char('h') | KeyCode::Left => {
                                    keys.reset();
                                    search.reset();
                                    status_message = None;

                                    match ui::home::run_home_screen(
                                        &mut terminal,
                                        &database,
                                        scroll_settings.mouse_step,
                                    )
                                    .expect("Home screen failed")
                                    {
                                        Some(new_novel_id) => {
                                            let novels = fetch_novels(&database)
                                                .await
                                                .expect("Failed to fetch novels");
                                            let selected_novel = novels
                                                .into_iter()
                                                .find(|novel| novel.id == new_novel_id)
                                                .expect("Selected novel not found");
                                            active_highlights = HighlightMatcher::from_groups(
                                                &highlight_settings.active_groups_for(
                                                    selected_novel.id,
                                                    &selected_novel.title,
                                                ),
                                            );
                                            let (new_chapters, most_recent_index) =
                                                fetch_chapters(&database, selected_novel.id)
                                                    .expect("Failed to fetch chapters");
                                            seed_if_absent(
                                                selected_novel.id,
                                                new_chapters.get(most_recent_index),
                                            );
                                            let selection_index = initial_index_for(
                                                selected_novel.id,
                                                &new_chapters,
                                                most_recent_index,
                                            );
                                            let mut new_list_state = ListState::default();
                                            if !new_chapters.is_empty() {
                                                new_list_state.select(Some(selection_index));
                                            }
                                            let filtered_chapters = new_chapters.clone();
                                            AppMode::ChapterState {
                                                novel: selected_novel,
                                                chapters: new_chapters,
                                                filtered: filtered_chapters,
                                                list_state: new_list_state,
                                            }
                                        }
                                        None => break 'app,
                                    }
                                }
                                KeyCode::Char('j') | KeyCode::Down => {
                                    let count = keys.take_count();
                                    status_message = None;
                                    list_state.select(move_selection_wrapping(
                                        list_state.selected(),
                                        filtered.len(),
                                        count,
                                        true,
                                    ));
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        list_state,
                                        filtered,
                                    }
                                }
                                KeyCode::Char('k') | KeyCode::Up => {
                                    let count = keys.take_count();
                                    status_message = None;
                                    list_state.select(move_selection_wrapping(
                                        list_state.selected(),
                                        filtered.len(),
                                        count,
                                        false,
                                    ));
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                KeyCode::Home => {
                                    keys.reset();
                                    status_message = None;
                                    if !filtered.is_empty() {
                                        list_state.select(Some(0));
                                    }
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                KeyCode::Char('g') if keys.seen_g => {
                                    keys.reset();
                                    status_message = None;
                                    if !filtered.is_empty() {
                                        list_state.select(Some(0));
                                    }
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                KeyCode::End | KeyCode::Char('G') => {
                                    keys.reset();
                                    status_message = None;
                                    if !filtered.is_empty() {
                                        list_state.select(Some(filtered.len() - 1));
                                    }
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        filtered,
                                        list_state,
                                    }
                                }
                                KeyCode::Char('l') | KeyCode::Enter | KeyCode::Right => {
                                    let selected = list_state.selected();
                                    if let Some(chapter) =
                                        selected.and_then(|index| filtered.get(index))
                                    {
                                        if let Some(current_index) =
                                            chapter_index_for(&chapters, chapter.id)
                                        {
                                            keys.reset();
                                            status_message = None;
                                            let scroll = initial_scroll_for(
                                                novel.id,
                                                chapter.id,
                                                reader_settings.scroll_persist,
                                            );
                                            record_reader_position(
                                                novel.id,
                                                chapter.id,
                                                scroll,
                                                reader_settings.scroll_persist,
                                            );
                                            AppMode::ReaderState {
                                                novel,
                                                chapters,
                                                current_index,
                                                scroll,
                                            }
                                        } else {
                                            AppMode::ChapterState {
                                                novel,
                                                chapters,
                                                filtered,
                                                list_state,
                                            }
                                        }
                                    } else {
                                        AppMode::ChapterState {
                                            novel,
                                            chapters,
                                            filtered,
                                            list_state,
                                        }
                                    }
                                }
                                KeyCode::Char('q') => break 'app,
                                _ => {
                                    keys.reset();
                                    AppMode::ChapterState {
                                        novel,
                                        chapters,
                                        list_state,
                                        filtered,
                                    }
                                }
                            }
                        }
                    }
                    AppMode::ReaderState {
                        novel,
                        chapters,
                        mut current_index,
                        scroll,
                    } => {
                        if bookmark_overlay.active {
                            let bookmarks = bookmarks_for(novel.id);
                            match key.code {
                                KeyCode::Esc | KeyCode::Char('B') => {
                                    bookmark_overlay.close();
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll,
                                    }
                                }
                                KeyCode::Char('j') | KeyCode::Down => {
                                    bookmark_overlay.selected = move_selection_wrapping(
                                        bookmark_overlay.selected,
                                        bookmarks.len(),
                                        1,
                                        true,
                                    );
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll,
                                    }
                                }
                                KeyCode::Char('k') | KeyCode::Up => {
                                    bookmark_overlay.selected = move_selection_wrapping(
                                        bookmark_overlay.selected,
                                        bookmarks.len(),
                                        1,
                                        false,
                                    );
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll,
                                    }
                                }
                                KeyCode::Enter => {
                                    if let Some(bookmark) = bookmark_overlay
                                        .selected
                                        .and_then(|index| bookmarks.get(index))
                                    {
                                        if let Some(index) =
                                            chapter_index_for(&chapters, bookmark.chapter_id)
                                        {
                                            current_index = index;
                                            let new_scroll = bookmark.scroll;
                                            pending_progress.clear();
                                            persist_reader(
                                                novel.id,
                                                &chapters,
                                                current_index,
                                                new_scroll,
                                                &reader_settings,
                                            );
                                            bookmark_overlay.close();
                                            status_message =
                                                Some("Jumped to bookmark.".to_string());
                                            AppMode::ReaderState {
                                                novel,
                                                chapters,
                                                current_index,
                                                scroll: new_scroll,
                                            }
                                        } else {
                                            status_message =
                                                Some("Bookmark chapter is not loaded.".to_string());
                                            AppMode::ReaderState {
                                                novel,
                                                chapters,
                                                current_index,
                                                scroll,
                                            }
                                        }
                                    } else {
                                        AppMode::ReaderState {
                                            novel,
                                            chapters,
                                            current_index,
                                            scroll,
                                        }
                                    }
                                }
                                KeyCode::Char('d') => {
                                    if let Some(selected) = bookmark_overlay.selected {
                                        pending_progress.flush_now(reader_settings.scroll_persist);
                                        if delete_bookmark(novel.id, selected) {
                                            let remaining = bookmarks.len().saturating_sub(1);
                                            bookmark_overlay.selected = if remaining == 0 {
                                                None
                                            } else {
                                                Some(selected.min(remaining - 1))
                                            };
                                            status_message = Some("Bookmark deleted.".to_string());
                                        }
                                    }
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll,
                                    }
                                }
                                _ => AppMode::ReaderState {
                                    novel,
                                    chapters,
                                    current_index,
                                    scroll,
                                },
                            }
                        } else {
                            match key.code {
                                KeyCode::Char('l') | KeyCode::Right => {
                                    let count = keys.take_count();
                                    current_index = if chapters.is_empty() {
                                        0
                                    } else {
                                        (current_index + count) % chapters.len()
                                    };
                                    let new_scroll = 0;
                                    pending_progress.clear();
                                    persist_reader(
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                KeyCode::Char('h') | KeyCode::Left => {
                                    let count = keys.take_count();
                                    current_index = if chapters.is_empty() {
                                        0
                                    } else {
                                        (current_index + chapters.len() - (count % chapters.len()))
                                            % chapters.len()
                                    };
                                    let new_scroll = 0;
                                    pending_progress.clear();
                                    persist_reader(
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                KeyCode::Char('b') => {
                                    keys.reset();
                                    bookmark_overlay.close();
                                    pending_progress.clear();
                                    persist_reader(
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        scroll,
                                        &reader_settings,
                                    );
                                    let mut chapter_list_state = ListState::default();
                                    if !chapters.is_empty() {
                                        chapter_list_state.select(Some(current_index));
                                    }
                                    status_message = None;
                                    AppMode::ChapterState {
                                        novel,
                                        chapters: chapters.clone(),
                                        filtered: chapters.clone(),
                                        list_state: chapter_list_state,
                                    }
                                }
                                KeyCode::Char('j') | KeyCode::Down => {
                                    let amount =
                                        scaled_step(scroll_settings.line_step, keys.take_count());
                                    let new_scroll =
                                        scroll_down(scroll, amount, reader_metrics.max_scroll);
                                    schedule_reader_progress(
                                        &mut pending_progress,
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                KeyCode::Char('k') | KeyCode::Up => {
                                    let amount =
                                        scaled_step(scroll_settings.line_step, keys.take_count());
                                    let new_scroll = scroll_up(scroll, amount);
                                    schedule_reader_progress(
                                        &mut pending_progress,
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                KeyCode::Char('d') => {
                                    let amount = scaled_step(
                                        scroll_settings.half_page_step,
                                        keys.take_count(),
                                    );
                                    let new_scroll =
                                        scroll_down(scroll, amount, reader_metrics.max_scroll);
                                    schedule_reader_progress(
                                        &mut pending_progress,
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                KeyCode::Char('u') => {
                                    let amount = scaled_step(
                                        scroll_settings.half_page_step,
                                        keys.take_count(),
                                    );
                                    let new_scroll = scroll_up(scroll, amount);
                                    schedule_reader_progress(
                                        &mut pending_progress,
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                KeyCode::PageDown | KeyCode::Char(' ') => {
                                    let step =
                                        page_step(&scroll_settings, reader_metrics.visible_height);
                                    let amount = scaled_step(step, keys.take_count());
                                    let new_scroll =
                                        scroll_down(scroll, amount, reader_metrics.max_scroll);
                                    schedule_reader_progress(
                                        &mut pending_progress,
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                KeyCode::PageUp | KeyCode::Backspace => {
                                    let step =
                                        page_step(&scroll_settings, reader_metrics.visible_height);
                                    let amount = scaled_step(step, keys.take_count());
                                    let new_scroll = scroll_up(scroll, amount);
                                    schedule_reader_progress(
                                        &mut pending_progress,
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                KeyCode::Home => {
                                    keys.reset();
                                    let new_scroll = 0;
                                    pending_progress.clear();
                                    persist_reader(
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                KeyCode::Char('g') if keys.seen_g => {
                                    keys.reset();
                                    let new_scroll = 0;
                                    pending_progress.clear();
                                    persist_reader(
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                KeyCode::End | KeyCode::Char('G') => {
                                    keys.reset();
                                    let new_scroll = reader_metrics.max_scroll;
                                    pending_progress.clear();
                                    persist_reader(
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                KeyCode::Char('m') => {
                                    keys.reset();
                                    pending_progress.flush_now(reader_settings.scroll_persist);
                                    if let Some(chapter) = chapters.get(current_index) {
                                        if add_bookmark(novel.id, chapter.id, scroll) {
                                            status_message = Some("Bookmark saved.".to_string());
                                        } else {
                                            status_message =
                                                Some("Bookmark already exists here.".to_string());
                                        }
                                    }
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll,
                                    }
                                }
                                KeyCode::Char('B') => {
                                    keys.reset();
                                    bookmark_overlay.open(bookmarks_for(novel.id).len());
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll,
                                    }
                                }
                                KeyCode::Char('q') => {
                                    pending_progress.clear();
                                    persist_reader(
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        scroll,
                                        &reader_settings,
                                    );
                                    break 'app;
                                }
                                _ => {
                                    keys.reset();
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll,
                                    }
                                }
                            }
                        }
                    }
                };
            } else if let Event::Mouse(mouse) = input_event {
                mode = match mode {
                    AppMode::ChapterState {
                        novel,
                        chapters,
                        filtered,
                        mut list_state,
                    } => {
                        match mouse.kind {
                            MouseEventKind::ScrollDown => {
                                list_state.select(move_selection_wrapping(
                                    list_state.selected(),
                                    filtered.len(),
                                    scroll_settings.mouse_step as usize,
                                    true,
                                ));
                            }
                            MouseEventKind::ScrollUp => {
                                list_state.select(move_selection_wrapping(
                                    list_state.selected(),
                                    filtered.len(),
                                    scroll_settings.mouse_step as usize,
                                    false,
                                ));
                            }
                            _ => {}
                        }
                        AppMode::ChapterState {
                            novel,
                            chapters,
                            filtered,
                            list_state,
                        }
                    }
                    AppMode::ReaderState {
                        novel,
                        chapters,
                        mut current_index,
                        scroll,
                    } => {
                        if bookmark_overlay.active {
                            let bookmarks = bookmarks_for(novel.id);
                            match mouse.kind {
                                MouseEventKind::ScrollDown => {
                                    bookmark_overlay.selected = move_selection_wrapping(
                                        bookmark_overlay.selected,
                                        bookmarks.len(),
                                        scroll_settings.mouse_step as usize,
                                        true,
                                    );
                                }
                                MouseEventKind::ScrollUp => {
                                    bookmark_overlay.selected = move_selection_wrapping(
                                        bookmark_overlay.selected,
                                        bookmarks.len(),
                                        scroll_settings.mouse_step as usize,
                                        false,
                                    );
                                }
                                _ => {}
                            }
                            AppMode::ReaderState {
                                novel,
                                chapters,
                                current_index,
                                scroll,
                            }
                        } else {
                            match mouse.kind {
                                MouseEventKind::ScrollDown => {
                                    let new_scroll = scroll_down(
                                        scroll,
                                        scroll_settings.mouse_step,
                                        reader_metrics.max_scroll,
                                    );
                                    if new_scroll != scroll {
                                        schedule_reader_progress(
                                            &mut pending_progress,
                                            novel.id,
                                            &chapters,
                                            current_index,
                                            new_scroll,
                                            &reader_settings,
                                        );
                                        status_message = None;
                                    }
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                MouseEventKind::ScrollUp => {
                                    let new_scroll = scroll_up(scroll, scroll_settings.mouse_step);
                                    if new_scroll != scroll {
                                        schedule_reader_progress(
                                            &mut pending_progress,
                                            novel.id,
                                            &chapters,
                                            current_index,
                                            new_scroll,
                                            &reader_settings,
                                        );
                                        status_message = None;
                                    }
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                MouseEventKind::Down(MouseButton::Left) => {
                                    current_index = if chapters.is_empty() {
                                        0
                                    } else {
                                        (current_index + 1) % chapters.len()
                                    };
                                    let new_scroll = 0;
                                    pending_progress.clear();
                                    persist_reader(
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                MouseEventKind::Down(MouseButton::Right) => {
                                    current_index = if chapters.is_empty() {
                                        0
                                    } else {
                                        (current_index + chapters.len() - 1) % chapters.len()
                                    };
                                    let new_scroll = 0;
                                    pending_progress.clear();
                                    persist_reader(
                                        novel.id,
                                        &chapters,
                                        current_index,
                                        new_scroll,
                                        &reader_settings,
                                    );
                                    status_message = None;
                                    AppMode::ReaderState {
                                        novel,
                                        chapters,
                                        current_index,
                                        scroll: new_scroll,
                                    }
                                }
                                _ => AppMode::ReaderState {
                                    novel,
                                    chapters,
                                    current_index,
                                    scroll,
                                },
                            }
                        }
                    }
                };
            }
        }
    }

    pending_progress.flush_now(reader_settings.scroll_persist);

    disable_raw_mode().expect("Cannot disable raw mode");
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .expect("Cannot leave alternate screen");
    terminal.show_cursor().expect("Cannot show cursor");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn theme() -> config::ColorTheme {
        config::ColorTheme {
            base_text: Color::White,
            single_quote: Color::Red,
            double_quote: Color::Green,
            square_brackets: Color::LightMagenta,
            triple_dots: Color::Yellow,
            botched_quote: Color::LightRed,
            stars: Color::DarkGray,
            lore_term: Color::LightCyan,
            rank_term: Color::LightYellow,
            place_term: Color::LightBlue,
            emphasis_line: Color::Magenta,
        }
    }

    fn chapter(id: i32, content: &str) -> Chapter {
        chapter_with_title(id, format!("Chapter {}", id), content)
    }

    fn chapter_with_title(id: i32, title: impl Into<String>, content: &str) -> Chapter {
        Chapter {
            id,
            reading_now: 0,
            title: title.into(),
            content: content.to_string(),
        }
    }

    fn highlight_group(name: &str, color: Color, terms: &[&str]) -> config::HighlightGroup {
        config::HighlightGroup {
            name: name.to_string(),
            color,
            terms: terms.iter().map(|term| term.to_string()).collect(),
        }
    }

    #[test]
    fn numeric_search_uses_displayed_numbers_not_database_ids() {
        let chapters = vec![
            chapter_with_title(818, "Chapter 35 - A Shadow, a Star and an Oracle", ""),
            chapter_with_title(1601, "Chapter 818 - Irregulars, Assemble", ""),
            chapter_with_title(2601, "Chapter 1818 - Moment of Respite", ""),
            chapter_with_title(3081, "Chapter 2081 - Fragments of War (18)", ""),
        ];

        let results = filter_chapters(&chapters, "818");
        let ids: Vec<i32> = results.iter().map(|chapter| chapter.id).collect();

        assert_eq!(ids, vec![1601, 2601]);
    }

    #[test]
    fn numeric_search_ranks_exact_visible_number_before_prefix_and_contains_matches() {
        let chapters = vec![
            chapter_with_title(1, "Chapter 1292 - History of Time", ""),
            chapter_with_title(2, "Chapter 292 - Just Cause", ""),
            chapter_with_title(3, "Chapter 2292 - Name Given", ""),
            chapter_with_title(4, "Chapter 921 - New Pets", ""),
        ];

        let results = filter_chapters(&chapters, "292");
        let ids: Vec<i32> = results.iter().map(|chapter| chapter.id).collect();

        assert_eq!(ids, vec![2, 1, 3]);
    }

    #[test]
    fn numeric_search_does_not_stitch_digits_across_repeated_chapter_text() {
        let chapters = vec![
            chapter_with_title(2921, "Chapter 921 - New Pets", ""),
            chapter_with_title(1071, "Chapter 1071 - Familiar Role", ""),
        ];

        assert!(filter_chapters(&chapters, "2921").is_empty());
    }

    #[test]
    fn numeric_search_exact_visible_number_outranks_prefix_and_contains_matches() {
        let chapters = vec![
            chapter_with_title(1, "Chapter 2921 - Storm", ""),
            chapter_with_title(2, "Chapter 29210 - Echo", ""),
            chapter_with_title(3, "Chapter 12921 - Drift", ""),
        ];

        let results = filter_chapters(&chapters, "2921");
        let ids: Vec<i32> = results.iter().map(|chapter| chapter.id).collect();

        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn numeric_search_ignores_non_leading_title_numbers() {
        let chapters = vec![chapter_with_title(
            2081,
            "Chapter 2081 - Fragments of War (18)",
            "",
        )];

        assert!(filter_chapters(&chapters, "18").is_empty());
    }

    #[test]
    fn mixed_search_keeps_fuzzy_title_matching() {
        let chapters = vec![
            chapter_with_title(2068, "Chapter 2068 - Fragments of War (5)", ""),
            chapter_with_title(2078, "Chapter 2078 - Fragments of War (15)", ""),
            chapter_with_title(2088, "Chapter 2088 - Fragments of War (25)", ""),
            chapter_with_title(292, "Chapter 292 - Just Cause", ""),
        ];

        let results = filter_chapters(&chapters, "fragment 5");
        let ids: Vec<i32> = results.iter().map(|chapter| chapter.id).collect();

        assert_eq!(ids, vec![2068, 2078, 2088]);
    }

    #[test]
    fn mixed_search_can_still_fuzzily_match_chapter_number_text() {
        let chapters = vec![
            chapter_with_title(1601, "Chapter 818 - Irregulars, Assemble", ""),
            chapter_with_title(2601, "Chapter 1818 - Moment of Respite", ""),
        ];

        let results = filter_chapters(&chapters, "ch 818");
        let ids: Vec<i32> = results.iter().map(|chapter| chapter.id).collect();

        assert_eq!(ids, vec![1601, 2601]);
    }

    #[test]
    fn numeric_search_without_visible_number_match_returns_no_results() {
        let chapters = vec![
            chapter_with_title(921, "Chapter 921 - New Pets", ""),
            chapter_with_title(1071, "Chapter 1071 - Familiar Role", ""),
        ];

        assert!(filter_chapters(&chapters, "555").is_empty());
    }

    #[test]
    fn visible_range_reports_bottom_page_instead_of_first_line_only() {
        let max_scroll = max_scroll_for(70, 44);

        assert_eq!(max_scroll, 26);
        assert_eq!(visible_line_range(70, 44, max_scroll), (27, 70));
    }

    #[test]
    fn prepared_chapter_cache_formats_once_and_remeasures_by_width() {
        let mut cache = PreparedChapterCache::default();
        let first = chapter(10, "First paragraph\nSecond paragraph");
        let no_highlights = HighlightMatcher::empty();

        cache.ensure(7, "Novel", &first, 40, &theme(), &no_highlights);
        let first_key = cache.key;
        let first_content = cache.content.clone();
        let first_text = cache.text.clone();

        assert_eq!(first_content, "\nFirst paragraph\n\nSecond paragraph");

        cache.ensure(7, "Novel", &first, 20, &theme(), &no_highlights);
        assert_eq!(cache.key, first_key);
        assert_eq!(cache.content, first_content);
        assert_eq!(cache.text, first_text);
        assert_eq!(cache.measured_width, 20);

        let changed = chapter(10, "First paragraph\nChanged paragraph");
        cache.ensure(7, "Novel", &changed, 20, &theme(), &no_highlights);

        assert_ne!(cache.key, first_key);
        assert_eq!(cache.content, "\nFirst paragraph\n\nChanged paragraph");
    }

    #[test]
    fn prepared_chapter_cache_invalidates_when_highlights_change() {
        let mut cache = PreparedChapterCache::default();
        let chapter = chapter(10, "A Memory shimmered.");
        let first_groups = vec![highlight_group("lore", Color::LightCyan, &["Memory"])];
        let second_groups = vec![highlight_group("lore", Color::Yellow, &["Memory"])];
        let first_matcher = HighlightMatcher::from_groups(&first_groups);
        let second_matcher = HighlightMatcher::from_groups(&second_groups);

        cache.ensure(7, "Novel", &chapter, 40, &theme(), &first_matcher);
        let first_key = cache.key;
        cache.ensure(7, "Novel", &chapter, 40, &theme(), &second_matcher);

        assert_ne!(cache.key, first_key);
    }

    #[test]
    fn pending_progress_becomes_due_only_after_idle_delay() {
        let mut pending = PendingReaderProgress::default();
        let now = Instant::now();
        let delay = Duration::from_millis(250);

        pending.schedule(1, 2, 3, now, true);

        assert!(pending
            .due_position(now + Duration::from_millis(249), delay)
            .is_none());
        assert_eq!(
            pending
                .due_position(now + Duration::from_millis(250), delay)
                .unwrap()
                .scroll,
            3
        );

        pending.schedule(1, 2, 4, now, false);
        assert!(pending.position.is_none());
    }
}
