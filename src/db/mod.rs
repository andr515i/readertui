use directories::ProjectDirs;
use rusqlite::{ffi, params, Connection, Error, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::DatabaseConfig;
use crate::models::{Chapter, Novel};

pub const DB_ENV_VAR: &str = "READERTUI_DB_FILE";

const LEGACY_DB_PATH: &str = "../db/novel.db";
const DB_FILE_NAME: &str = "novel.db";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct Database {
    path: PathBuf,
}

impl Database {
    pub fn from_config(config: &DatabaseConfig) -> Result<Self> {
        let database = Self {
            path: resolve_database_path(config.path.as_deref()),
        };
        database.ensure_ready()?;
        Ok(database)
    }

    fn connect(&self) -> Result<Connection> {
        connect_at(&self.path)
    }

    fn ensure_ready(&self) -> Result<()> {
        self.connect().map(|_| ())
    }
}

/// Fetches all novels from the database.
pub async fn fetch_novels(database: &Database) -> Result<Vec<Novel>> {
    let database = database.clone();
    tokio::task::spawn_blocking(move || fetch_novels_sync(&database))
        .await
        .expect("Failed to load novels")
}

pub fn fetch_novels_sync(database: &Database) -> Result<Vec<Novel>> {
    let conn = database.connect()?;
    let mut stmt = conn.prepare("SELECT id, title FROM novels ORDER BY id")?;
    let novel_iter = stmt.query_map([], |row| {
        Ok(Novel {
            id: row.get(0)?,
            title: row.get(1)?,
        })
    })?;

    let mut novels = Vec::new();
    for novel in novel_iter {
        novels.push(novel?);
    }
    Ok(novels)
}

/// Fetches all chapters for a given novel id and returns them alongside the most recently read index.
pub fn fetch_chapters(database: &Database, novel_id: i32) -> Result<(Vec<Chapter>, usize)> {
    let conn = database.connect()?;
    let mut stmt = conn.prepare(
        "SELECT id, title, content, reading_now FROM chapters WHERE novel_id = ? ORDER BY id",
    )?;

    let chapter_iter = stmt.query_map(params![novel_id], |row| {
        Ok(Chapter {
            id: row.get(0)?,
            title: row.get(1)?,
            content: row.get(2)?,
            reading_now: row.get(3)?,
        })
    })?;

    let mut chapters = Vec::new();
    for chapter in chapter_iter {
        chapters.push(chapter?);
    }
    let most_recent_index = chapters
        .iter()
        .position(|chapter| chapter.reading_now == 1)
        .unwrap_or(0);
    Ok((chapters, most_recent_index))
}

fn resolve_database_path(configured_path: Option<&Path>) -> PathBuf {
    let env_path = env::var_os(DB_ENV_VAR)
        .filter(|path| !path.to_string_lossy().trim().is_empty())
        .map(PathBuf::from);
    let legacy_path = PathBuf::from(LEGACY_DB_PATH);
    let app_data_path = app_data_database_path();

    resolve_database_path_from(
        env_path.as_deref(),
        configured_path,
        &legacy_path,
        &app_data_path,
    )
}

fn resolve_database_path_from(
    env_path: Option<&Path>,
    configured_path: Option<&Path>,
    legacy_path: &Path,
    app_data_path: &Path,
) -> PathBuf {
    if let Some(path) = env_path {
        return path.to_path_buf();
    }

    if let Some(path) = configured_path {
        return path.to_path_buf();
    }

    if legacy_path.exists() {
        return legacy_path.to_path_buf();
    }

    app_data_path.to_path_buf()
}

fn app_data_database_path() -> PathBuf {
    if let Some(project_dirs) = ProjectDirs::from("com", "novel-scraper", "readertui") {
        return project_dirs.data_dir().join(DB_FILE_NAME);
    }

    PathBuf::from(DB_FILE_NAME)
}

fn connect_at(path: &Path) -> Result<Connection> {
    ensure_parent_dir(path)?;
    let conn = Connection::open(path)?;
    migrate(&conn)?;
    Ok(conn)
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(io_to_sqlite_error)?;
    }
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version > SCHEMA_VERSION {
        return Err(Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_ERROR),
            Some(format!(
                "Unsupported database schema version {version}; this build supports {SCHEMA_VERSION}"
            )),
        ));
    }

    if version == 0 {
        conn.execute_batch(
            r#"
            BEGIN;
            CREATE TABLE IF NOT EXISTS novels (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS chapters (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                novel_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                reading_now INTEGER DEFAULT NULL,
                FOREIGN KEY(novel_id) REFERENCES novels(id)
            );
            PRAGMA user_version = 1;
            COMMIT;
            "#,
        )?;
    }

    Ok(())
}

fn io_to_sqlite_error(error: std::io::Error) -> Error {
    Error::ToSqlConversionFailure(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        let unique = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let dir = env::temp_dir().join(format!("readertui-{name}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn env_database_path_wins_over_config() {
        let dir = temp_dir("env-wins");
        let env_path = dir.join("env.db");
        let config_path = dir.join("config.db");
        let legacy_path = dir.join("legacy.db");
        let app_data_path = dir.join("app.db");

        assert_eq!(
            resolve_database_path_from(
                Some(&env_path),
                Some(&config_path),
                &legacy_path,
                &app_data_path,
            ),
            env_path
        );
    }

    #[test]
    fn config_database_path_wins_over_defaults() {
        let dir = temp_dir("config-wins");
        let config_path = dir.join("config.db");
        let legacy_path = dir.join("legacy.db");
        let app_data_path = dir.join("app.db");
        fs::write(&legacy_path, "").unwrap();

        assert_eq!(
            resolve_database_path_from(None, Some(&config_path), &legacy_path, &app_data_path),
            config_path
        );
    }

    #[test]
    fn existing_legacy_database_is_auto_detected() {
        let dir = temp_dir("legacy");
        let legacy_path = dir.join("legacy.db");
        let app_data_path = dir.join("app.db");
        fs::write(&legacy_path, "").unwrap();

        assert_eq!(
            resolve_database_path_from(None, None, &legacy_path, &app_data_path),
            legacy_path
        );
    }

    #[test]
    fn app_data_database_path_is_used_without_overrides_or_legacy() {
        let dir = temp_dir("app-data");
        let legacy_path = dir.join("missing-legacy.db");
        let app_data_path = dir.join("app.db");

        assert_eq!(
            resolve_database_path_from(None, None, &legacy_path, &app_data_path),
            app_data_path
        );
    }

    #[test]
    fn new_database_is_migrated_to_version_one() {
        let dir = temp_dir("migration");
        let database = Database {
            path: dir.join("new.db"),
        };

        let conn = database.connect().unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let novel_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM novels", [], |row| row.get(0))
            .unwrap();
        let chapter_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chapters", [], |row| row.get(0))
            .unwrap();

        assert_eq!(version, 1);
        assert_eq!(novel_count, 0);
        assert_eq!(chapter_count, 0);
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = temp_dir("idempotent");
        let database = Database {
            path: dir.join("repeat.db"),
        };

        database.connect().unwrap();
        let conn = database.connect().unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();

        assert_eq!(version, 1);
    }

    #[test]
    fn existing_legacy_schema_is_not_destroyed() {
        let dir = temp_dir("legacy-schema");
        let path = dir.join("legacy.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE novels (id integer primary key autoincrement, title text);
                CREATE TABLE chapters (
                    id integer primary key autoincrement,
                    novel_id integer,
                    title text,
                    content text,
                    reading_now INTEGER DEFAULT NULL,
                    foreign key(novel_id) references novels(id)
                );
                INSERT INTO novels (title) VALUES ('Existing Novel');
                "#,
            )
            .unwrap();
        }

        let database = Database { path };
        let conn = database.connect().unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        let title: String = conn
            .query_row("SELECT title FROM novels WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert_eq!(version, 1);
        assert_eq!(title, "Existing Novel");
    }
}
