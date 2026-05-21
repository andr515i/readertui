use crossterm::event::{self, Event, KeyCode, MouseEventKind};
use ratatui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use rusqlite::Result as SqlResult;
use std::{io, time::Duration};

use crate::app::move_selection_wrapping;
use crate::db::{fetch_novels_sync, Database};

pub type NovelId = i32;

fn load_home_novels(database: &Database) -> SqlResult<Vec<crate::models::Novel>> {
    fetch_novels_sync(database)
}

pub fn run_home_screen<B: Backend>(
    terminal: &mut Terminal<B>,
    database: &Database,
    mouse_step: u16,
) -> io::Result<Option<NovelId>> {
    let novels = load_home_novels(database).map_err(std::io::Error::other)?;
    let mut list_state = ListState::default();
    let mut page_step = 1usize;
    if !novels.is_empty() {
        list_state.select(Some(0));
    }

    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(1)])
                .split(f.area());

            let title = Paragraph::new(Line::from(vec![ratatui::text::Span::styled(
                "Novel Reader - Home",
                Style::default().fg(Color::Cyan),
            )]))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));

            f.render_widget(title, chunks[0]);

            let items: Vec<ListItem> = if novels.is_empty() {
                vec![ListItem::new("No novels found")]
            } else {
                novels
                    .iter()
                    .map(|novel| ListItem::new(novel.title.as_str()))
                    .collect()
            };

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Select a Novel (j/k, wheel, PgUp/PgDn, Home/End, Enter, q)"),
                )
                .highlight_symbol(">> ")
                .highlight_style(Style::default().fg(Color::Yellow));

            f.render_stateful_widget(list, chunks[1], &mut list_state);
            page_step = chunks[1].height.saturating_sub(2).max(1) as usize;
        })?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == event::KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('q') => return Ok(None),
                            KeyCode::Char('j') | KeyCode::Down => {
                                list_state.select(move_selection_wrapping(
                                    list_state.selected(),
                                    novels.len(),
                                    1,
                                    true,
                                ));
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                list_state.select(move_selection_wrapping(
                                    list_state.selected(),
                                    novels.len(),
                                    1,
                                    false,
                                ));
                            }
                            KeyCode::PageDown => {
                                list_state.select(move_selection_wrapping(
                                    list_state.selected(),
                                    novels.len(),
                                    page_step,
                                    true,
                                ));
                            }
                            KeyCode::PageUp => {
                                list_state.select(move_selection_wrapping(
                                    list_state.selected(),
                                    novels.len(),
                                    page_step,
                                    false,
                                ));
                            }
                            KeyCode::Home => {
                                if !novels.is_empty() {
                                    list_state.select(Some(0));
                                }
                            }
                            KeyCode::End => {
                                if !novels.is_empty() {
                                    list_state.select(Some(novels.len() - 1));
                                }
                            }
                            KeyCode::Enter | KeyCode::Char('l') => {
                                if let Some(selected) = list_state.selected() {
                                    if let Some(novel) = novels.get(selected) {
                                        return Ok(Some(novel.id));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        list_state.select(move_selection_wrapping(
                            list_state.selected(),
                            novels.len(),
                            mouse_step as usize,
                            true,
                        ));
                    }
                    MouseEventKind::ScrollUp => {
                        list_state.select(move_selection_wrapping(
                            list_state.selected(),
                            novels.len(),
                            mouse_step as usize,
                            false,
                        ));
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}
