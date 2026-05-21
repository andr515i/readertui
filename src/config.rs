use std::fs;
use std::path::{Path, PathBuf};

use ratatui::style::Color;
use serde::Deserialize;

const CONFIG_PATH: &str = "config.toml";

#[derive(Debug, Clone)]
pub struct Config {
    pub scroll: ScrollConfig,
    pub reader: ReaderConfig,
    pub database: DatabaseConfig,
    pub colors: ColorTheme,
    pub highlights: HighlightConfig,
    pub debug: DebugConfig,
}

#[derive(Debug, Clone, Copy)]
pub struct ScrollConfig {
    pub line_step: u16,
    pub half_page_step: u16,
    pub mouse_step: u16,
    pub page_step: Option<u16>,
}

#[derive(Debug, Clone, Copy)]
pub struct ReaderConfig {
    pub scroll_persist: bool,
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct DebugConfig {
    pub enabled: bool,
    pub log_path: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ColorTheme {
    pub base_text: Color,
    pub single_quote: Color,
    pub double_quote: Color,
    pub square_brackets: Color,
    pub triple_dots: Color,
    pub botched_quote: Color,
    pub stars: Color,
    #[allow(dead_code)]
    pub lore_term: Color,
    #[allow(dead_code)]
    pub rank_term: Color,
    #[allow(dead_code)]
    pub place_term: Color,
    pub emphasis_line: Color,
}

#[derive(Debug, Clone)]
pub struct HighlightConfig {
    pub enabled: bool,
    pub global_groups: Vec<HighlightGroup>,
    pub profiles: Vec<HighlightProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightGroup {
    pub name: String,
    pub color: Color,
    pub terms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightProfile {
    pub name: String,
    pub novel_id: Option<i32>,
    pub novel_title_contains: Option<String>,
    pub groups: Vec<HighlightGroup>,
}

impl Config {
    pub fn load() -> Self {
        let raw = load_raw_config();
        Self::from_raw(raw)
    }

    fn from_raw(raw: RawConfig) -> Self {
        let colors = ColorTheme::from_raw(&raw.colors);
        Self {
            scroll: ScrollConfig {
                line_step: raw.scroll.line_step,
                half_page_step: raw.scroll.half_page_step,
                mouse_step: raw.scroll.mouse_step,
                page_step: raw.scroll.page_step,
            },
            reader: ReaderConfig {
                scroll_persist: raw.reader.scroll_persist,
            },
            database: DatabaseConfig::from_raw(&raw.database),
            colors,
            highlights: HighlightConfig::from_raw(&raw.highlights),
            debug: DebugConfig::from_raw(&raw.debug),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawConfig {
    scroll: RawScrollConfig,
    reader: RawReaderConfig,
    database: RawDatabaseConfig,
    colors: RawColorConfig,
    highlights: RawHighlightConfig,
    debug: RawDebugConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct RawScrollConfig {
    line_step: u16,
    half_page_step: u16,
    mouse_step: u16,
    page_step: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct RawReaderConfig {
    scroll_persist: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawDatabaseConfig {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct RawDebugConfig {
    enabled: bool,
    log_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct RawColorConfig {
    base_text: String,
    single_quote: String,
    double_quote: String,
    square_brackets: String,
    triple_dots: String,
    botched_quote: String,
    stars: String,
    lore_term: String,
    rank_term: String,
    place_term: String,
    emphasis_line: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct RawHighlightConfig {
    enabled: bool,
    global_groups: Vec<RawHighlightGroup>,
    profiles: Vec<RawHighlightProfile>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawHighlightGroup {
    name: String,
    color: String,
    terms: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawHighlightProfile {
    name: String,
    novel_id: Option<i32>,
    novel_title_contains: Option<String>,
    groups: Vec<RawHighlightGroup>,
}

impl Default for RawScrollConfig {
    fn default() -> Self {
        Self {
            line_step: 3,
            half_page_step: 22,
            mouse_step: 3,
            page_step: None,
        }
    }
}

impl Default for RawReaderConfig {
    fn default() -> Self {
        Self {
            scroll_persist: true,
        }
    }
}

impl Default for RawDebugConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            log_path: "readertui-debug.log".into(),
        }
    }
}

impl Default for RawColorConfig {
    fn default() -> Self {
        Self {
            base_text: "white".into(),
            single_quote: "red".into(),
            double_quote: "green".into(),
            square_brackets: "light_magenta".into(),
            triple_dots: "yellow".into(),
            botched_quote: "light_red".into(),
            stars: "dark_gray".into(),
            lore_term: "light_cyan".into(),
            rank_term: "light_yellow".into(),
            place_term: "light_blue".into(),
            emphasis_line: "magenta".into(),
        }
    }
}

impl Default for RawHighlightConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            global_groups: Vec::new(),
            profiles: Vec::new(),
        }
    }
}

impl DatabaseConfig {
    fn from_raw(raw: &RawDatabaseConfig) -> Self {
        Self {
            path: raw
                .path
                .as_ref()
                .map(|path| path.trim())
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        }
    }
}

impl ColorTheme {
    fn from_raw(raw: &RawColorConfig) -> Self {
        Self {
            base_text: parse_color_or_default(&raw.base_text, Color::White, "colors.base_text"),
            single_quote: parse_color_or_default(
                &raw.single_quote,
                Color::Red,
                "colors.single_quote",
            ),
            double_quote: parse_color_or_default(
                &raw.double_quote,
                Color::Green,
                "colors.double_quote",
            ),
            square_brackets: parse_color_or_default(
                &raw.square_brackets,
                Color::LightMagenta,
                "colors.square_brackets",
            ),
            triple_dots: parse_color_or_default(
                &raw.triple_dots,
                Color::Yellow,
                "colors.triple_dots",
            ),
            botched_quote: parse_color_or_default(
                &raw.botched_quote,
                Color::LightRed,
                "colors.botched_quote",
            ),
            stars: parse_color_or_default(&raw.stars, Color::DarkGray, "colors.stars"),
            lore_term: parse_color_or_default(&raw.lore_term, Color::LightCyan, "colors.lore_term"),
            rank_term: parse_color_or_default(
                &raw.rank_term,
                Color::LightYellow,
                "colors.rank_term",
            ),
            place_term: parse_color_or_default(
                &raw.place_term,
                Color::LightBlue,
                "colors.place_term",
            ),
            emphasis_line: parse_color_or_default(
                &raw.emphasis_line,
                Color::Magenta,
                "colors.emphasis_line",
            ),
        }
    }
}

impl HighlightConfig {
    fn from_raw(raw: &RawHighlightConfig) -> Self {
        Self {
            enabled: raw.enabled,
            global_groups: raw
                .global_groups
                .iter()
                .enumerate()
                .filter_map(|(index, group)| {
                    HighlightGroup::from_raw(group, format!("highlights.global_groups[{index}]"))
                })
                .collect(),
            profiles: raw
                .profiles
                .iter()
                .enumerate()
                .map(|(index, profile)| HighlightProfile::from_raw(profile, index))
                .collect(),
        }
    }

    pub fn active_groups_for(&self, novel_id: i32, novel_title: &str) -> Vec<HighlightGroup> {
        if !self.enabled {
            return Vec::new();
        }

        let mut groups = self.global_groups.clone();
        groups.extend(
            self.profiles
                .iter()
                .filter(|profile| profile.matches(novel_id, novel_title))
                .flat_map(|profile| profile.groups.clone()),
        );
        groups
    }
}

impl DebugConfig {
    fn from_raw(raw: &RawDebugConfig) -> Self {
        Self {
            enabled: raw.enabled,
            log_path: if raw.log_path.trim().is_empty() {
                "readertui-debug.log".into()
            } else {
                raw.log_path.trim().to_owned()
            },
        }
    }
}

impl HighlightGroup {
    fn from_raw(raw: &RawHighlightGroup, field: String) -> Option<Self> {
        let terms = raw
            .terms
            .iter()
            .map(|term| term.trim())
            .filter(|term| !term.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if terms.is_empty() {
            return None;
        }

        Some(Self {
            name: raw.name.trim().to_owned(),
            color: parse_color_or_default(&raw.color, Color::LightCyan, &format!("{field}.color")),
            terms,
        })
    }
}

impl HighlightProfile {
    fn from_raw(raw: &RawHighlightProfile, index: usize) -> Self {
        Self {
            name: raw.name.trim().to_owned(),
            novel_id: raw.novel_id,
            novel_title_contains: raw
                .novel_title_contains
                .as_ref()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            groups: raw
                .groups
                .iter()
                .enumerate()
                .filter_map(|(group_index, group)| {
                    HighlightGroup::from_raw(
                        group,
                        format!("highlights.profiles[{index}].groups[{group_index}]"),
                    )
                })
                .collect(),
        }
    }

    fn matches(&self, novel_id: i32, novel_title: &str) -> bool {
        let id_matches = self.novel_id == Some(novel_id);
        let title_matches = self.novel_title_contains.as_ref().is_some_and(|needle| {
            novel_title
                .to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase())
        });

        id_matches || title_matches
    }
}

fn load_raw_config() -> RawConfig {
    let path = Path::new(CONFIG_PATH);
    match fs::read_to_string(path) {
        Ok(contents) => match toml::from_str::<RawConfig>(&contents) {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!(
                    "Failed to parse {}: {}. Falling back to defaults.",
                    path.display(),
                    error
                );
                RawConfig::default()
            }
        },
        Err(error) => {
            if path.exists() {
                eprintln!(
                    "Failed to read {}: {}. Falling back to defaults.",
                    path.display(),
                    error
                );
            }
            RawConfig::default()
        }
    }
}

fn parse_color_or_default(value: &str, default: Color, field: &str) -> Color {
    if let Some(parsed) = parse_color(value) {
        parsed
    } else {
        if !value.trim().is_empty() {
            eprintln!(
                "Unknown color '{}' for {}. Using default {:?}.",
                value.trim(),
                field,
                default
            );
        }
        default
    }
}

fn parse_color(value: &str) -> Option<Color> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    let color = match lower.as_str() {
        "reset" => Some(Color::Reset),
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" => Some(Color::Gray),
        "grey" => Some(Color::Gray),
        "dark_gray" | "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "light_red" => Some(Color::LightRed),
        "light_green" => Some(Color::LightGreen),
        "light_yellow" => Some(Color::LightYellow),
        "light_blue" => Some(Color::LightBlue),
        "light_magenta" => Some(Color::LightMagenta),
        "light_cyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => None,
    };

    if color.is_some() {
        return color;
    }

    if let Some(rgb) = parse_rgb(lower.as_str()) {
        return Some(rgb);
    }

    if let Some(indexed) = parse_indexed(lower.as_str()) {
        return Some(indexed);
    }

    if let Some(hex) = parse_hex(lower.as_str()) {
        return Some(hex);
    }

    None
}

fn parse_hex(value: &str) -> Option<Color> {
    if !value.starts_with('#') || value.len() != 7 {
        return None;
    }

    let r = u8::from_str_radix(&value[1..3], 16).ok()?;
    let g = u8::from_str_radix(&value[3..5], 16).ok()?;
    let b = u8::from_str_radix(&value[5..7], 16).ok()?;

    Some(Color::Rgb(r, g, b))
}

fn parse_rgb(value: &str) -> Option<Color> {
    if !value.starts_with("rgb(") || !value.ends_with(')') {
        return None;
    }

    let inner = &value[4..value.len() - 1];
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() != 3 {
        return None;
    }

    let r = parts.first()?.trim().parse::<u8>().ok()?;
    let g = parts.get(1)?.trim().parse::<u8>().ok()?;
    let b = parts.get(2)?.trim().parse::<u8>().ok()?;

    Some(Color::Rgb(r, g, b))
}

fn parse_indexed(value: &str) -> Option<Color> {
    if !value.starts_with("indexed(") || !value.ends_with(')') {
        return None;
    }

    let inner = &value[8..value.len() - 1];
    let index = inner.trim().parse::<u8>().ok()?;
    Some(Color::Indexed(index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_highlight_groups_and_profiles() {
        let raw: RawConfig = toml::from_str(
            r##"
            [highlights]
            enabled = true

            [[highlights.global_groups]]
            name = "Global"
            color = "#112233"
            terms = ["Memory", " Spell "]

            [[highlights.profiles]]
            name = "Shadow Slave"
            novel_title_contains = "shadow slave"

            [[highlights.profiles.groups]]
            name = "Places"
            color = "light_blue"
            terms = ["Dark Sea"]
            "##,
        )
        .unwrap();
        let config = Config::from_raw(raw);

        assert!(config.highlights.enabled);
        assert_eq!(
            config.highlights.global_groups[0].terms,
            vec!["Memory", "Spell"]
        );
        assert_eq!(
            config.highlights.global_groups[0].color,
            Color::Rgb(0x11, 0x22, 0x33)
        );
        assert_eq!(
            config.highlights.profiles[0].groups[0].terms,
            vec!["Dark Sea"]
        );
    }

    #[test]
    fn active_groups_include_globals_and_matching_profiles() {
        let raw: RawConfig = toml::from_str(
            r#"
            [highlights]
            enabled = true

            [[highlights.global_groups]]
            name = "Global"
            color = "cyan"
            terms = ["Memory"]

            [[highlights.profiles]]
            name = "Shadow Slave"
            novel_title_contains = "Shadow Slave"

            [[highlights.profiles.groups]]
            name = "Shadow Places"
            color = "blue"
            terms = ["Dark Sea"]
            "#,
        )
        .unwrap();
        let config = Config::from_raw(raw);

        let shadow_groups = config.highlights.active_groups_for(7, "Shadow Slave");
        assert_eq!(shadow_groups.len(), 2);

        let other_groups = config.highlights.active_groups_for(8, "Other Novel");
        assert_eq!(other_groups.len(), 1);
        assert_eq!(other_groups[0].terms, vec!["Memory"]);
    }

    #[test]
    fn disabled_highlights_return_no_active_groups() {
        let raw: RawConfig = toml::from_str(
            r#"
            [highlights]
            enabled = false

            [[highlights.global_groups]]
            name = "Global"
            color = "cyan"
            terms = ["Memory"]
            "#,
        )
        .unwrap();
        let config = Config::from_raw(raw);

        assert!(config
            .highlights
            .active_groups_for(7, "Shadow Slave")
            .is_empty());
    }

    #[test]
    fn parses_debug_config_with_safe_defaults() {
        let default_config = Config::from_raw(toml::from_str("").unwrap());
        assert!(!default_config.debug.enabled);
        assert_eq!(default_config.debug.log_path, "readertui-debug.log");

        let configured = Config::from_raw(
            toml::from_str(
                r#"
                [debug]
                enabled = true
                log_path = "custom-debug.log"
                "#,
            )
            .unwrap(),
        );
        assert!(configured.debug.enabled);
        assert_eq!(configured.debug.log_path, "custom-debug.log");
    }

    #[test]
    fn parses_database_config_path() {
        let default_config = Config::from_raw(toml::from_str("").unwrap());
        assert_eq!(default_config.database.path, None);

        let configured = Config::from_raw(
            toml::from_str(
                r#"
                [database]
                path = "../db/novel.db"
                "#,
            )
            .unwrap(),
        );
        assert_eq!(
            configured.database.path,
            Some(PathBuf::from("../db/novel.db"))
        );
    }
}
