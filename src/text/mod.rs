use crate::config::{ColorTheme, HighlightGroup};
use html_escape::decode_html_entities;
use once_cell::sync::Lazy;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use regex::{Regex, RegexBuilder};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

const BLOCK_BREAK: &str = "\u{1e}";

/// Normalizes various punctuation variants to ASCII to simplify downstream processing.
pub fn normalize_punctuation(input: &str) -> String {
    input
        .chars()
        .map(|character| match character {
            '“' | '”' | '„' | '«' | '»' => '"',
            '‘' | '’' | '‚' | '‹' | '›' | '′' | '‛' | '`' => '\'',
            '（' => '(',
            '）' => ')',
            '［' => '[',
            '］' => ']',
            '｛' => '{',
            '｝' => '}',
            '❲' => '[',
            '❳' => ']',
            '【' => '[',
            '】' => ']',
            '「' => '"',
            '」' => '"',
            '『' => '"',
            '』' => '"',
            '〈' => '<',
            '〉' => '>',
            '–' | '—' | '‒' | '―' => '-',
            _ => character,
        })
        .collect()
}

/// Converts basic HTML chapter content to formatted paragraph text suitable for the reader.
pub fn format_html_to_text(html: &str) -> String {
    let mut working = strip_html_noise(html);
    let reflow_continuations =
        HTML_BLOCK_TAG_RE.is_match(&working) || HTML_BR_TAG_RE.is_match(&working);

    working = HTML_BLOCK_TAG_RE
        .replace_all(&working, BLOCK_BREAK)
        .into_owned();
    working = HTML_BR_TAG_RE
        .replace_all(&working, BLOCK_BREAK)
        .into_owned();
    working = BLOCK_BREAK_RUN_RE.replace_all(&working, "\n").into_owned();
    working = HTML_TAG_RE.replace_all(&working, "").into_owned();

    let decoded = decode_html_entities(&working);
    let normalized: String = normalize_punctuation(&decoded);
    let cleaned = strip_lm_segments(&normalized);
    let fixed = fix_t_corruptions(&cleaned);
    let dewatermarked = strip_content_artifacts(&fixed);

    let body = normalize_paragraph_spacing(&dewatermarked, reflow_continuations);

    if body.is_empty() {
        body
    } else {
        format!("\n{}", body) // leading visual spacer before chapter text
    }
}

fn strip_html_noise(input: &str) -> String {
    let without_comments = HTML_COMMENT_RE.replace_all(input, " ").into_owned();
    HTML_SCRIPT_STYLE_RE
        .replace_all(&without_comments, " ")
        .into_owned()
}

fn strip_content_artifacts(input: &str) -> String {
    let dewatermarked = strip_watermarks(input);
    BROKEN_GLYPH_RE.replace_all(&dewatermarked, "").into_owned()
}

fn normalize_paragraph_spacing(input: &str, reflow_continuations: bool) -> String {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let mut paragraphs = Vec::new();
    let mut current = None;
    let mut can_skip_heading = true;

    for line in normalized.lines() {
        let trimmed = normalize_text_fragment(line);
        if trimmed.is_empty() {
            flush_current_paragraph(&mut paragraphs, &mut current);
            continue;
        }

        for fragment in split_quote_fragments(&trimmed) {
            let fragment = normalize_text_fragment(&fragment);
            if fragment.is_empty() {
                continue;
            }

            if can_skip_heading
                && paragraphs.is_empty()
                && current.is_none()
                && CHAPTER_HEADING_RE.is_match(&fragment)
            {
                continue;
            }

            can_skip_heading = false;
            push_paragraph_fragment(
                &mut paragraphs,
                &mut current,
                fragment,
                reflow_continuations,
            );
        }
    }

    flush_current_paragraph(&mut paragraphs, &mut current);
    paragraphs.join("\n\n")
}

fn normalize_text_fragment(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn flush_current_paragraph(paragraphs: &mut Vec<String>, current: &mut Option<String>) {
    if let Some(paragraph) = current.take() {
        if !paragraph.is_empty() {
            paragraphs.push(paragraph);
        }
    }
}

fn push_paragraph_fragment(
    paragraphs: &mut Vec<String>,
    current: &mut Option<String>,
    fragment: String,
    reflow_continuations: bool,
) {
    let Some(mut paragraph) = current.take() else {
        *current = Some(fragment);
        return;
    };

    if should_join_fragments(&paragraph, &fragment, reflow_continuations) {
        if needs_join_space(&paragraph, &fragment) {
            paragraph.push(' ');
        }
        paragraph.push_str(&fragment);
        *current = Some(paragraph);
    } else {
        paragraphs.push(paragraph);
        *current = Some(fragment);
    }
}

fn should_join_fragments(previous: &str, next: &str, reflow_continuations: bool) -> bool {
    if !reflow_continuations {
        return false;
    }
    if is_forced_standalone(previous) || is_forced_standalone(next) {
        return false;
    }
    if has_terminal_end(previous) {
        return false;
    }

    starts_with_lowercase(next)
        || ends_with_continuation_punctuation(previous)
        || ends_with_incomplete_word(previous)
        || previous.chars().count() >= 60
}

fn is_forced_standalone(fragment: &str) -> bool {
    is_standalone_quote(fragment)
        || EMPHASIS_LINE_RE.is_match(fragment)
        || STARS_RE.is_match(fragment)
}

fn is_standalone_quote(fragment: &str) -> bool {
    let trimmed = fragment.trim();
    if trimmed.len() < 2 {
        return false;
    }
    let first = trimmed.chars().next();
    let last = trimmed.chars().next_back();
    matches!(
        (first, last),
        (Some('\''), Some('\'')) | (Some('"'), Some('"'))
    )
}

fn needs_join_space(previous: &str, next: &str) -> bool {
    let previous_ends_with_space = previous
        .chars()
        .next_back()
        .is_some_and(char::is_whitespace);
    let next_starts_with_space = next.chars().next().is_some_and(char::is_whitespace);
    !previous_ends_with_space && !next_starts_with_space
}

fn has_terminal_end(fragment: &str) -> bool {
    let mut trimmed = fragment.trim();
    loop {
        let Some(character) = trimmed.chars().next_back() else {
            return false;
        };
        if matches!(character, '"' | '\'' | ')' | ']' | '}') {
            trimmed = &trimmed[..trimmed.len() - character.len_utf8()];
            continue;
        }
        return matches!(character, '.' | '!' | '?' | '…');
    }
}

fn starts_with_lowercase(fragment: &str) -> bool {
    fragment
        .chars()
        .find(|character| character.is_alphabetic())
        .is_some_and(char::is_lowercase)
}

fn ends_with_continuation_punctuation(fragment: &str) -> bool {
    fragment
        .trim()
        .chars()
        .next_back()
        .is_some_and(|character| matches!(character, ',' | ';' | ':' | '-'))
}

fn ends_with_incomplete_word(fragment: &str) -> bool {
    let Some(word) = fragment
        .split_whitespace()
        .next_back()
        .map(|word| word.trim_matches(|character: char| !character.is_alphanumeric()))
    else {
        return false;
    };
    matches!(
        word.to_ascii_lowercase().as_str(),
        "a" | "an"
            | "and"
            | "as"
            | "at"
            | "by"
            | "for"
            | "from"
            | "his"
            | "in"
            | "into"
            | "its"
            | "like"
            | "of"
            | "or"
            | "than"
            | "that"
            | "the"
            | "their"
            | "to"
            | "was"
            | "were"
            | "with"
    )
}

fn split_quote_fragments(line: &str) -> Vec<String> {
    let mut spans = isolatable_quote_spans(line);
    if spans.is_empty() {
        return vec![line.to_owned()];
    }

    spans.sort_by_key(|(start, _)| *start);
    let mut fragments = Vec::new();
    let mut cursor = 0;

    for (start, end) in spans {
        if start < cursor {
            continue;
        }
        if cursor < start {
            fragments.push(line[cursor..start].to_owned());
        }
        fragments.push(line[start..end].to_owned());
        cursor = end;
    }

    if cursor < line.len() {
        fragments.push(line[cursor..].to_owned());
    }

    fragments
}

fn isolatable_quote_spans(line: &str) -> Vec<(usize, usize)> {
    let mut spans = quote_spans_for_line(line, '"');
    spans.extend(quote_spans_for_line(line, '\''));
    spans
        .into_iter()
        .filter(|(start, end)| quote_span_should_stand_alone(line, *start, *end))
        .collect()
}

fn quote_spans_for_line(line: &str, quote: char) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut spans = Vec::new();
    let mut current_start = None;

    for (index, (byte_index, character)) in chars.iter().copied().enumerate() {
        if character != quote {
            continue;
        }
        if quote == '\'' && !is_single_quote_delimiter(&chars, index) {
            continue;
        }

        if let Some(start) = current_start.take() {
            let end = byte_index + character.len_utf8();
            if start < end {
                spans.push((start, end));
            }
        } else {
            current_start = Some(byte_index);
        }
    }

    spans
}

fn is_single_quote_delimiter(chars: &[(usize, char)], index: usize) -> bool {
    let previous = index
        .checked_sub(1)
        .and_then(|previous| chars.get(previous));
    let next = chars.get(index + 1);
    let previous_is_word = previous
        .map(|(_, character)| is_wordish(*character))
        .unwrap_or(false);
    let next_is_word = next
        .map(|(_, character)| is_wordish(*character))
        .unwrap_or(false);

    !(previous_is_word && next_is_word)
}

fn quote_span_should_stand_alone(line: &str, start: usize, end: usize) -> bool {
    if end <= start + 2 || !line.is_char_boundary(start) || !line.is_char_boundary(end) {
        return false;
    }

    let inner_start = start + 1;
    let inner_end = end - 1;
    if !line.is_char_boundary(inner_start) || !line.is_char_boundary(inner_end) {
        return false;
    }

    let inner = line[inner_start..inner_end].trim();
    !inner.is_empty() && has_terminal_end(inner)
}

fn strip_lm_segments(input: &str) -> String {
    LM_SEGMENT_RE.replace_all(input, "").into_owned()
}

fn fix_t_corruptions(input: &str) -> String {
    T_CORRUPTION_RE.replace_all(input, "T").into_owned()
}

fn strip_watermarks(input: &str) -> String {
    let mut output = input.to_string();
    for marker in WATERMARK_MARKERS {
        if !marker.is_empty() {
            output = output.replace(marker, "");
        }
    }
    let after_inline = INLINE_WATERMARK_RE.replace_all(&output, " ").into_owned();
    let after_watermarks = WATERMARK_REGEX.replace_all(&after_inline, "").into_owned();
    AD_SCRIPT_RE.replace_all(&after_watermarks, "").into_owned()
}

static BOTCHED_QUOTE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*['’'][^'’']*["“”]\s*$"#).unwrap());
static BOTCHED_QUOTE2_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*["“”][^“”"]*[‘’']\s*$"#).unwrap());
static TRIPLE_DOTS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\.\.\..*|^….+").unwrap());
static EMPHASIS_LINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*[A-Z][A-Za-z' -]{0,60}(?:\.{3}|…)\s*$").unwrap());
static STARS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\*\*.*").unwrap());
static LM_SEGMENT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bl[^\s]*[~/'\d][^\s]*m[\s]?\.").unwrap());
static T_CORRUPTION_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)'l'").unwrap());
static HTML_COMMENT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
static HTML_SCRIPT_STYLE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?is)<script\b[^>]*>.*?</script>|<style\b[^>]*>.*?</style>|<noscript\b[^>]*>.*?</noscript>",
    )
    .unwrap()
});
static HTML_BLOCK_TAG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)</?(?:p|div|h[1-6]|li|blockquote|section|article)(?:\s+[^>]*)?>").unwrap()
});
static HTML_BR_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)<br\s*/?>").unwrap());
static HTML_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<[^>]+>").unwrap());
static BLOCK_BREAK_RUN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?:[ \t\r\n]*\x1E[ \t\r\n]*)+").unwrap());
static CHAPTER_HEADING_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^chapter\s+\d+\b.*$").unwrap());
static BROKEN_GLYPH_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ɽ�]+").unwrap());
static WATERMARK_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?mi)^\s*[°•✪◆◈⊛➤✧#/_]*\s*N[оo0][v][eе][lI1][iіɪl][gɡ][hһ][tт][^\n]*$").unwrap()
});
static INLINE_WATERMARK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r###"(?i)[\s\{\}\[\]\(\)"'★❖°•✪◆◈⊛➤✧#/_~|-]*N[\s._•·-]*[оo0][\s._•·-]*v[\s._•·-]*[eе][\s._•·-]*[lI1Ⅰ][\s._•·-]*[iіɪιlI1𝕚𝚒][\s._•·-]*[gɡ][\s._•·-]*[hһ][\s._•·-]*[tт](?:[\s\{\}\[\]\(\)"'★❖°•✪◆◈⊛➤✧#/_~|-]*\([^)\n]{0,120}\))?[\s\{\}\[\]\(\)"'★❖°•✪◆◈⊛➤✧#/_~|-]*"###).unwrap()
});
static WATERMARK_MARKERS: &[&str] = &[
    "°• N 𝑜 v 𝑒 l i g h t •°",
    "✪ Nоvеlіgһt ✪ (Official version)",
    "/N_o_v_e_l_i_g_h_t/",
    "◆ Nоvеlіgһt ◆ (Only on Nоvеlіgһt)",
    "◈ Nоvеlіgһт ◈ (Continue reading)",
    "➤ NоvеⅠight ➤ (Read more on our source)",
    "✧ NоvеIight ✧ (Original source)",
    "°• N 𝑜 v 𝑒 l i g h t •° # Nоvеlight #",
    "⊛ Nоvеlιght ⊛ (Read the full story)",
    "# Nоvеlight #",
    "[N O V E L I G H T]",
    " N.O.V.E.L.I.G.H.T ",
    "\"N.o.v.e.l.i.g.h.t\"",
];

/// Matches injected ad/tracker script blocks that leak through HTML scraping.
/// Covers patterns such as:
///   - var adx_id_NNNNN = ...
///   - window.pubadxtag / window.pubfuturetag push calls
///   - aclib.runBanner({ ... })
static AD_SCRIPT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?s)var\s+adx_id_\d+\s*=.*?(?:\n|$)|adx_id_\w+\.id\s*=.*?(?:\n|$)|window\.pubadxtag.*?(?:\n|$)|window\.pubfuturetag.*?(?:\n|$)|aclib\.runBanner\(\s*\{[^}]*\}\s*\)\s*;?",
    )
    .unwrap()
});

fn square_overrides_for_line(line: &str, in_attr: &mut bool) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let bytes = line.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if !*in_attr {
            if let Some(relative_start) = line[index..].find('[') {
                let start = index + relative_start;
                if let Some(relative_end) = line[start + 1..].find(']') {
                    let end = start + 1 + relative_end + 1;
                    ranges.push((start, end));
                    index = end;
                } else {
                    ranges.push((start, line.len()));
                    *in_attr = true;
                    break;
                }
            } else {
                break;
            }
        } else if let Some(relative_end) = line[index..].find(']') {
            let end = index + relative_end + 1;
            ranges.push((0, end));
            *in_attr = false;
            index = end;
        } else {
            ranges.push((0, line.len()));
            break;
        }
    }

    ranges
}

fn overlaps(first: (usize, usize), second: (usize, usize)) -> bool {
    let (first_start, first_end) = first;
    let (second_start, second_end) = second;
    first_start < second_end && second_start < first_end
}

fn subtract_overlaps(range: (usize, usize), blockers: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let (mut current_start, end) = range;
    let mut segments = Vec::new();

    if current_start >= end {
        return segments;
    }

    let mut sorted_blockers = blockers.to_vec();
    sorted_blockers.sort_by_key(|(start, _)| *start);

    for (block_start, block_end) in sorted_blockers {
        if block_end <= current_start {
            continue;
        }
        if block_start >= end {
            break;
        }

        if current_start < block_start {
            segments.push((current_start, block_start.min(end)));
        }
        current_start = current_start.max(block_end);
        if current_start >= end {
            break;
        }
    }

    if current_start < end {
        segments.push((current_start, end));
    }

    segments.retain(|(start, finish)| start < finish);
    segments
}

#[derive(Debug)]
pub struct HighlightMatcher {
    matcher: Option<Regex>,
    colors: HashMap<String, Color>,
    fingerprint: u64,
}

impl HighlightMatcher {
    pub fn empty() -> Self {
        Self {
            matcher: None,
            colors: HashMap::new(),
            fingerprint: 0,
        }
    }

    pub fn from_groups(groups: &[HighlightGroup]) -> Self {
        let mut colors = HashMap::new();
        let mut terms = Vec::new();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        for group in groups {
            for term in &group.terms {
                let trimmed = term.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let key = trimmed.to_lowercase();
                if colors.contains_key(&key) {
                    continue;
                }

                trimmed.hash(&mut hasher);
                format!("{:?}", group.color).hash(&mut hasher);
                colors.insert(key, group.color);
                terms.push(trimmed.to_owned());
            }
        }

        terms.sort_by(|left, right| right.len().cmp(&left.len()));

        let matcher = if terms.is_empty() {
            None
        } else {
            let pattern = terms
                .iter()
                .map(|term| regex::escape(term))
                .collect::<Vec<_>>()
                .join("|");
            RegexBuilder::new(&pattern)
                .case_insensitive(true)
                .unicode(true)
                .build()
                .ok()
        };

        Self {
            matcher,
            colors,
            fingerprint: hasher.finish(),
        }
    }

    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    fn color_for(&self, matched: &str) -> Option<Color> {
        self.colors.get(&matched.to_lowercase()).copied()
    }
}

impl Default for HighlightMatcher {
    fn default() -> Self {
        Self::empty()
    }
}

fn is_wordish(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn has_wordish_boundaries(line: &str, start: usize, end: usize) -> bool {
    let before = line[..start].chars().next_back();
    let after = line[end..].chars().next();

    !before.is_some_and(is_wordish) && !after.is_some_and(is_wordish)
}

fn push_literal_highlight_overrides(
    overrides: &mut Vec<(usize, usize, Color)>,
    matcher: &HighlightMatcher,
    line: &str,
    blockers: &mut Vec<(usize, usize)>,
) {
    let Some(regex) = &matcher.matcher else {
        return;
    };

    for matched in regex.find_iter(line) {
        let span = (matched.start(), matched.end());
        if !has_wordish_boundaries(line, span.0, span.1) {
            continue;
        }
        if blockers.iter().any(|blocker| overlaps(span, *blocker)) {
            continue;
        }
        let Some(color) = matcher.color_for(matched.as_str()) else {
            continue;
        };
        blockers.push(span);
        overrides.push((span.0, span.1, color));
    }
}

fn is_single_quote(character: char) -> bool {
    matches!(character, '\'' | '’' | '‘' | '‚' | '‛')
}

fn is_double_quote(character: char) -> bool {
    matches!(character, '"' | '“' | '”')
}

fn first_non_space_is_double_quote(line: &str) -> bool {
    line.chars()
        .find(|character| !character.is_whitespace())
        .is_some_and(is_double_quote)
}

fn mixed_edge_quote_span(line: &str) -> Option<(usize, usize)> {
    let mut start_index = None;
    for (index, character) in line.char_indices() {
        if !character.is_whitespace() {
            start_index = Some((index, character));
            break;
        }
    }

    let (start_index, first_character) = start_index?;

    let mut end_index = None;
    for (index, character) in line.char_indices().rev() {
        if !character.is_whitespace() {
            end_index = Some((index + character.len_utf8(), character));
            break;
        }
    }

    let (end_index, last_character) = end_index?;

    let mixed = (is_single_quote(first_character) && is_double_quote(last_character))
        || (is_double_quote(first_character) && is_single_quote(last_character));

    if mixed {
        Some((start_index, end_index))
    } else {
        None
    }
}

fn single_quote_ranges_for_line(line: &str, in_single_quote: &mut bool) -> Vec<(usize, usize)> {
    // Single quotes only colorize when a full pair exists on the same line.
    // This avoids bleeding highlights when authors forget to close a leading ".
    *in_single_quote = false;

    let mut ranges = Vec::new();
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut i = 0;

    while i < chars.len() {
        let (start_idx, ch) = chars[i];
        if ch != '\'' {
            i += 1;
            continue;
        }

        let prev = if i == 0 { None } else { Some(chars[i - 1].1) };
        let next = if i + 1 < chars.len() {
            Some(chars[i + 1].1)
        } else {
            None
        };

        let prev_is_alnum = prev.map(|c| c.is_alphanumeric()).unwrap_or(false);
        let next_is_alnum = next.map(|c| c.is_alphanumeric()).unwrap_or(false);
        let prev_is_word = prev_is_alnum || prev.map(|c| c == '_').unwrap_or(false);
        let next_is_word = next_is_alnum || next.map(|c| c == '_').unwrap_or(false);

        // Treat inner apostrophes and possessive endings as non-delimiters.
        if prev_is_word && next_is_word {
            i += 1;
            continue;
        }

        // Search for a matching closing quote later in the line.
        let mut j = i + 1;
        let mut closing: Option<usize> = None;
        while j < chars.len() {
            let (end_idx, candidate) = chars[j];
            if candidate == '\'' {
                let pre = if j == 0 { None } else { Some(chars[j - 1].1) };
                let post = if j + 1 < chars.len() {
                    Some(chars[j + 1].1)
                } else {
                    None
                };

                let pre_is_alnum = pre.map(|c| c.is_alphanumeric()).unwrap_or(false);
                let post_is_alnum = post.map(|c| c.is_alphanumeric()).unwrap_or(false);
                let pre_is_word = pre_is_alnum || pre.map(|c| c == '_').unwrap_or(false);
                let post_is_word = post_is_alnum || post.map(|c| c == '_').unwrap_or(false);

                if pre_is_word && post_is_word {
                    j += 1;
                    continue;
                }

                closing = Some(end_idx + candidate.len_utf8());
                break;
            }
            j += 1;
        }

        if let Some(end) = closing {
            ranges.push((start_idx, end));
            i = j + 1;
        } else {
            // No closing quote on this line — ignore the opener.
            i += 1;
        }
    }

    ranges
}

fn double_quote_ranges_for_line(line: &str, in_double_quote: &mut bool) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut current_start = if *in_double_quote { Some(0) } else { None };
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut index = 0;

    while index < chars.len() {
        let (byte_index, character) = chars[index];
        if !is_double_quote(character) {
            index += 1;
            continue;
        }

        if let Some(start) = current_start {
            let end = byte_index + character.len_utf8();
            if start < end {
                ranges.push((start, end));
            }
            current_start = None;
            *in_double_quote = false;
            index += 1;
            continue;
        }

        // look for a closing quote on the same line
        let mut search_index = index + 1;
        let mut closing: Option<(usize, usize)> = None;
        while search_index < chars.len() {
            let (candidate_byte, candidate_char) = chars[search_index];
            if is_double_quote(candidate_char) {
                closing = Some((candidate_byte, candidate_char.len_utf8()));
                break;
            }
            search_index += 1;
        }

        if let Some((close_byte, close_len)) = closing {
            ranges.push((byte_index, close_byte + close_len));
            index = search_index + 1;
            continue;
        }

        // only treat as multi-line if the quote starts the sentence (ignoring leading whitespace)
        if line[..byte_index].trim().is_empty() {
            current_start = Some(byte_index);
            *in_double_quote = true;
        }

        index += 1;
    }

    if let Some(start) = current_start {
        if start < line.len() {
            ranges.push((start, line.len()));
        }
    }

    ranges
}

/// Applies syntax highlighting to the provided chapter text.
pub fn colorize_text(
    input: &str,
    theme: &ColorTheme,
    highlight_matcher: &HighlightMatcher,
) -> Text<'static> {
    let mut styled = Text::default();
    let mut in_attribute = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for line in input.lines() {
        // let base_color = if TRIPLE_DOTS_RE.is_match(line) {
        //     Color::Yellow
        // } else {
        //     Color::White
        // };

        let base_color = theme.base_text;

        if in_double_quote && line.trim().is_empty() || first_non_space_is_double_quote(line) {
            in_double_quote = false;
        }

        let mut overrides: Vec<(usize, usize, Color)> = Vec::new();
        let square_ranges = square_overrides_for_line(line, &mut in_attribute);
        for (start, end) in &square_ranges {
            overrides.push((*start, *end, theme.square_brackets));
        }

        'triple_dots: for matched in TRIPLE_DOTS_RE.find_iter(line) {
            let span = (matched.start(), matched.end());
            for bracket_range in &square_ranges {
                if overlaps(span, *bracket_range) {
                    continue 'triple_dots;
                }
            }
            overrides.push((span.0, span.1, theme.triple_dots));
        }
        let single_quote_ranges = single_quote_ranges_for_line(line, &mut in_single_quote);
        let mut protected_ranges = square_ranges.clone();
        for span in single_quote_ranges {
            let segments = subtract_overlaps(span, &square_ranges);
            for (start, end) in segments {
                protected_ranges.push((start, end));
                overrides.push((start, end, theme.single_quote));
            }
        }

        let double_quote_ranges = double_quote_ranges_for_line(line, &mut in_double_quote);
        for span in double_quote_ranges {
            let segments = subtract_overlaps(span, &square_ranges);
            for (start, end) in segments {
                protected_ranges.push((start, end));
                overrides.push((start, end, theme.double_quote));
            }
        }

        let mut botched = false;
        if let Some(matched) = BOTCHED_QUOTE_RE.find(line) {
            protected_ranges.push((matched.start(), matched.end()));
            overrides.push((matched.start(), matched.end(), theme.botched_quote));
            botched = true;
        }
        if let Some(matched) = BOTCHED_QUOTE2_RE.find(line) {
            protected_ranges.push((matched.start(), matched.end()));
            overrides.push((matched.start(), matched.end(), theme.botched_quote));
            botched = true;
        }
        if !botched {
            if let Some((start, end)) = mixed_edge_quote_span(line) {
                protected_ranges.push((start, end));
                overrides.push((start, end, theme.botched_quote));
                botched = true;
            }
        }
        for matched in STARS_RE.find_iter(line) {
            protected_ranges.push((matched.start(), matched.end()));
            overrides.push((matched.start(), matched.end(), theme.stars));
        }
        if EMPHASIS_LINE_RE.is_match(line) {
            overrides.push((0, line.len(), theme.emphasis_line));
        }

        push_literal_highlight_overrides(
            &mut overrides,
            highlight_matcher,
            line,
            &mut protected_ranges,
        );

        if botched {
            in_single_quote = false;
            in_double_quote = false;
        }

        if line.is_empty() {
            styled.lines.push(Line::from(Span::styled(
                "",
                Style::default().fg(base_color),
            )));
            continue;
        }

        let mut color_map: Vec<Option<Color>> = vec![None; line.len()];
        for (start, end, color) in overrides {
            let safe_start = start.min(line.len());
            let safe_end = end.min(line.len());
            if safe_start >= safe_end {
                continue;
            }
            for cell in color_map.iter_mut().take(safe_end).skip(safe_start) {
                *cell = Some(color);
            }
        }

        let mut spans = Vec::new();
        let mut segment_start = 0usize;
        let mut segment_color = color_map[0];
        let mut boundaries = line
            .char_indices()
            .map(|(index, _)| index)
            .skip(1)
            .collect::<Vec<usize>>();
        boundaries.push(line.len());

        for boundary in boundaries {
            let next_color = if boundary < line.len() {
                color_map[boundary]
            } else {
                None
            };

            if next_color != segment_color {
                if boundary > segment_start {
                    let color = segment_color.unwrap_or(base_color);
                    spans.push(Span::styled(
                        line[segment_start..boundary].to_owned(),
                        Style::default().fg(color),
                    ));
                }
                segment_start = boundary;
                segment_color = next_color;
            }
        }

        if segment_start < line.len() {
            let color = segment_color.unwrap_or(base_color);
            spans.push(Span::styled(
                line[segment_start..].to_owned(),
                Style::default().fg(color),
            ));
        }

        styled.lines.push(Line::from(spans));
    }

    styled
}

static WORDINGS_TO_FIX: &[(&str, &str)] = &[
    ("Nefis", "Nephis"),
    ("Nef", "Neph"),
    ("Netther", "Nether"),
    ("the Nether", "Nether"),
    ("the Weaver", "Weaver"),
    ("Night Garder", "Night Garden"),
    ("the Bastion", "Bastion"),
    ("the Night Garden", "Night Garden"),
    ("the Ravenheart", "Ravenheart"),
];

pub fn fix_wording(input: &str) -> String {
    let mut output = input.to_string();

    for (from, to) in WORDINGS_TO_FIX {
        output = output.replace(from, to);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> ColorTheme {
        ColorTheme {
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

    fn span_color_for(text: &Text<'static>, needle: &str) -> Option<Color> {
        text.lines.iter().find_map(|line| {
            line.spans.iter().find_map(|span| {
                span.content
                    .contains(needle)
                    .then_some(span.style.fg)
                    .flatten()
            })
        })
    }

    fn highlight_group(name: &str, color: Color, terms: &[&str]) -> HighlightGroup {
        HighlightGroup {
            name: name.to_string(),
            color,
            terms: terms.iter().map(|term| term.to_string()).collect(),
        }
    }

    #[test]
    fn plain_newline_paragraphs_keep_blank_visual_spacing() {
        assert_eq!(
            format_html_to_text("First paragraph\nSecond paragraph"),
            "\nFirst paragraph\n\nSecond paragraph"
        );
    }

    #[test]
    fn paragraph_and_br_tags_keep_blank_visual_spacing() {
        assert_eq!(
            format_html_to_text("<p>First paragraph</p><p>Second<br />line</p>"),
            "\nFirst paragraph\n\nSecond line"
        );
    }

    #[test]
    fn inline_novelight_watermark_fragments_are_removed() {
        let cleaned = format_html_to_text("Before ~Nоvеl𝕚ght~ after");
        assert!(!cleaned.contains("Nоv"));
        assert_eq!(cleaned, "\nBefore after");
    }

    #[test]
    fn div_blocks_restore_paragraph_breaks_without_gluing() {
        assert_eq!(
            format_html_to_text(
                "<div>Nephis held the shimmering crystals.</div><div>The remnants of shattered souls glowed softly.</div><div>Around them, the settlement hurried inside.</div>"
            ),
            "\nNephis held the shimmering crystals.\n\nThe remnants of shattered souls glowed softly.\n\nAround them, the settlement hurried inside."
        );
    }

    #[test]
    fn inline_single_quoted_thoughts_split_to_standalone_paragraphs() {
        assert_eq!(
            format_html_to_text(
                "<div>Rain froze for a heartbeat, her mind numb. 'N-no...'</div><div>Then, Tamar moved weakly.</div>"
            ),
            "\nRain froze for a heartbeat, her mind numb.\n\n'N-no...'\n\nThen, Tamar moved weakly."
        );
    }

    #[test]
    fn inline_double_quoted_dialogue_splits_without_bleeding_into_narration() {
        let formatted = format_html_to_text(
            "<div>He grimaced.\"What are you thinking about?\"Changing Star sighed.</div>",
        );
        assert_eq!(
            formatted,
            "\nHe grimaced.\n\n\"What are you thinking about?\"\n\nChanging Star sighed."
        );

        let styled = colorize_text(&formatted, &theme(), &HighlightMatcher::empty());
        let quote_line = styled
            .lines
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains("thinking"))
            })
            .unwrap();
        assert!(quote_line
            .spans
            .iter()
            .all(|span| span.style.fg == Some(Color::Green)));
        let narration_line = styled
            .lines
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains("Changing"))
            })
            .unwrap();
        assert!(narration_line
            .spans
            .iter()
            .all(|span| span.style.fg == Some(Color::White)));
    }

    #[test]
    fn hard_wrapped_div_blocks_reflow_into_full_paragraphs() {
        assert_eq!(
            format_html_to_text(
                "<div>The Burned Forest stretched beneath an ashen sky like a bleak and lifeless monument</div>
<div>to total destruction. Here and there, the blackened trunks rose into the</div>
<div>sky like broken towers.</div>
<div>Beneath them, an impenetrable labyrinth of charred deadfall soared hundreds of meters</div>
<div>above the ground, completely hiding it from view.</div>"
            ),
            "\nThe Burned Forest stretched beneath an ashen sky like a bleak and lifeless monument to total destruction. Here and there, the blackened trunks rose into the sky like broken towers.\n\nBeneath them, an impenetrable labyrinth of charred deadfall soared hundreds of meters above the ground, completely hiding it from view."
        );
    }

    #[test]
    fn headings_watermarks_ads_and_broken_glyphs_are_removed() {
        let cleaned = format_html_to_text(
            "<div><strong>Chapter 2000 &nbsp;Human Beasts</strong></div>
<div>The Burned Forest  {N•o•v•e•l•i•g•h•t}  stretched.</div>
<div>\"Done.\" ɽ�</div>
<div>it ❖ Nоvеl𝚒ght ❖ (Exclusive on Nоvеl𝚒ght) takes special eyes.</div>
<script>var adx_id_10065 = 1; window.pubadxtag.push({zoneid: 10065});</script>",
        );

        assert!(!cleaned.contains("Chapter 2000"));
        assert!(!cleaned.contains("Nоv"));
        assert!(!cleaned.contains('ɽ'));
        assert!(!cleaned.contains('�'));
        assert!(!cleaned.contains("adx_id"));
        assert_eq!(
            cleaned,
            "\nThe Burned Forest stretched.\n\n\"Done.\"\n\nit takes special eyes."
        );
    }

    #[test]
    fn configurable_literal_highlight_groups_are_colored() {
        let groups = vec![
            highlight_group("places", Color::LightBlue, &["Dark Sea"]),
            highlight_group("ranks", Color::LightYellow, &["Great Titan"]),
            highlight_group("lore", Color::LightCyan, &["Memory", "Nightmare Creature"]),
        ];
        let matcher = HighlightMatcher::from_groups(&groups);
        let styled = colorize_text(
            "The Dark Sea held a Great Titan, a Memory, and a Nightmare Creature.\nStill...",
            &theme(),
            &matcher,
        );

        assert_eq!(span_color_for(&styled, "Dark Sea"), Some(Color::LightBlue));
        assert_eq!(
            span_color_for(&styled, "Great Titan"),
            Some(Color::LightYellow)
        );
        assert_eq!(span_color_for(&styled, "Memory"), Some(Color::LightCyan));
        assert_eq!(
            span_color_for(&styled, "Nightmare Creature"),
            Some(Color::LightCyan)
        );
        assert_eq!(span_color_for(&styled, "Still..."), Some(Color::Magenta));
    }

    #[test]
    fn longest_literal_highlight_wins_over_shorter_overlap() {
        let groups = vec![
            highlight_group("short", Color::Red, &["Titan"]),
            highlight_group("long", Color::Yellow, &["Great Titan"]),
        ];
        let matcher = HighlightMatcher::from_groups(&groups);
        let styled = colorize_text("A Great Titan arrived.", &theme(), &matcher);

        assert_eq!(span_color_for(&styled, "Great Titan"), Some(Color::Yellow));
    }

    #[test]
    fn literal_highlights_do_not_match_inside_larger_words() {
        let groups = vec![highlight_group("class", Color::Yellow, &["Titan"])];
        let matcher = HighlightMatcher::from_groups(&groups);
        let styled = colorize_text("Titanic Titan", &theme(), &matcher);
        let spans = styled.lines[0]
            .spans
            .iter()
            .map(|span| (span.content.to_string(), span.style.fg))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec![
                ("Titanic ".to_string(), Some(Color::White)),
                ("Titan".to_string(), Some(Color::Yellow)),
            ]
        );
    }

    #[test]
    fn duplicate_literal_highlights_keep_first_configured_color() {
        let groups = vec![
            highlight_group("first", Color::Red, &["Memory"]),
            highlight_group("second", Color::Yellow, &["Memory"]),
        ];
        let matcher = HighlightMatcher::from_groups(&groups);
        let styled = colorize_text("Memory", &theme(), &matcher);

        assert_eq!(span_color_for(&styled, "Memory"), Some(Color::Red));
    }

    #[test]
    fn dialogue_color_takes_priority_over_configured_highlights() {
        let groups = vec![highlight_group(
            "rank",
            Color::LightYellow,
            &["Great Titan"],
        )];
        let matcher = HighlightMatcher::from_groups(&groups);
        let styled = colorize_text("\"Great Titan\"", &theme(), &matcher);
        let colors = styled.lines[0]
            .spans
            .iter()
            .map(|span| span.style.fg)
            .collect::<Vec<_>>();

        assert_eq!(colors, vec![Some(Color::Green)]);
    }
}
