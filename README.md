# readertui

`readertui` is a terminal reader for a user-provided SQLite database of novels
and chapters. It focuses on fast keyboard navigation, chapter search, scroll
position persistence, bookmarks, and configurable text highlighting.

The project does not ship novel data, scraped content, or a scraper. Bring your
own lawful content database.

## Features

- Terminal UI built with `ratatui` and `crossterm`
- Home screen for selecting novels
- Chapter list with fuzzy search
- Reader view with keyboard, paging, and mouse-wheel scrolling
- Per-user reading progress and bookmarks
- Configurable colors and keyword highlighting
- Optional debug logging for text rendering work

## Install And Run

Requirements:

- Rust toolchain
- A terminal that supports raw mode and the alternate screen

Run from the project directory:

```sh
cargo run
```

Run tests:

```sh
cargo test
```

Build a release binary:

```sh
cargo build --release
```

## Database

`readertui` uses SQLite. On startup it opens and migrates the database using
this path order:

1. `READERTUI_DB_FILE`
2. `[database] path = "..."`
   in `config.toml`
3. Existing legacy `../db/novel.db`
4. A new empty `novel.db` in the platform app data directory

The app creates an empty database automatically when none exists. An empty
database is valid; the home screen will show `No novels found`.

Schema:

```sql
CREATE TABLE novels (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL
);

CREATE TABLE chapters (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    novel_id INTEGER NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    reading_now INTEGER DEFAULT NULL,
    FOREIGN KEY(novel_id) REFERENCES novels(id)
);
```

The `reading_now` column is kept for compatibility with older local databases.
Current reading progress is stored per OS user in a JSON file, not in the
content database.

## Configuration

Copy `config.example.toml` to `config.toml` for local customization. The local
`config.toml` is intentionally ignored by Git.

Useful environment variables:

- `READERTUI_DB_FILE`: override the SQLite content database path
- `READERTUI_PROGRESS_FILE`: override the reading progress JSON path
- `READERTUI_DEBUG_LOG`: enable debug logging and choose the log file path

## Repository Layout

This public repository is intended to contain only the reader application. If
you also maintain private scraper, data, or tooling projects, keep those in
separate private repositories. A private umbrella repository can include this
repo and the private repos as Git submodules for one-command updates:

```sh
git submodule update --init --recursive
git pull --recurse-submodules
```

## Legal And Content Note

This project is a reader for user-provided data. Do not commit or distribute
copyrighted novel text, scraped chapters, private databases, logs containing
chapter text, or personal reading data unless you have the rights and consent to
do so.

## License

MIT. See `LICENSE`.
