//! Surgery on `LuaUI/Config/BYAR.lua`, the file BAR keeps widget state in.
//!
//! The file is Lua returning one table:
//!
//! ```lua
//! return {
//!     allowUserWidgets = true,
//!     data = { AdvPlayersList = { ... }, ["Attack AoE"] = { ... } },
//!     order = { AdvPlayersList = 51, ["Attack AoE"] = 61, ["Auto Cloak Units"] = 0 },
//! }
//! ```
//!
//! `order` decides what runs — **`0` means disabled** — and `data` holds each
//! widget's saved settings. Both are keyed by `GetInfo().name`, which is the
//! same string the replay telemetry reports, so a widget's row on the usage
//! page and its entry here are the same widget without any mapping.
//!
//! **Edits are textual and local, on purpose.** A real file here is a hundred
//! and sixty kilobytes of somebody's accumulated settings — window positions,
//! keybinds, colour choices — and this is a lobby, not a Lua interpreter. Parsing
//! the whole thing to re-emit it would risk every one of those settings to
//! change one integer. So the table is located by brace matching and only the
//! lines that must change are touched; everything else stays byte for byte.
//!
//! **It refuses rather than guesses.** If the structure is not what is expected
//! — no `order` block, unbalanced braces, a key that appears twice — the edit is
//! declined and the file is left alone. A widget that will not disable is a
//! nuisance; a corrupted config loses settings the player cannot get back.
//!
//! **Delete removes both halves.** BAR's own `SaveConfigData` only ever adds, so
//! removing a widget's `order` entry while leaving its `data` behind produces
//! exactly the zombie entries already visible in real files: settings for
//! widgets that are no longer installed, restored the moment one is reinstalled.
//! Reversibility means both go.
//!
//! **Never while a game is running.** `widgetHandler:Shutdown` rewrites this file
//! wholesale from memory on exit, so an edit made mid-game is overwritten
//! without a word. Callers gate on that; this module only refuses to guess.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `order = 0` is how BAR records a widget the player switched off.
pub const DISABLED: i64 = 0;

/// Where the file lives under a write directory.
pub const CONFIG_PATH: &str = "LuaUI/Config/BYAR.lua";

/// Widgets BAR forces back on at load, whatever `order` says.
///
/// `selector.lua` is the widget list itself, and anything with
/// `GetInfo().hidden` is infrastructure. Writing `0` for these is accepted by
/// the file and then ignored by the game, so the honest thing is to say so
/// rather than report a disable that will not happen.
pub const ALWAYS_ENABLED: [&str; 1] = ["Widget Selector"];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("widget config has no `{0}` table")]
    MissingTable(&'static str),
    #[error("widget config has unbalanced braces in `{0}`")]
    Unbalanced(&'static str),
    #[error("widget config lists `{0}` more than once")]
    Duplicate(String),
    #[error("`{0}` is forced on by the game and cannot be disabled here")]
    ForcedOn(String),
}

/// What a widget's presence in the config looks like.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WidgetState {
    /// `GetInfo().name`, the same key the replay telemetry reports.
    pub name: String,
    /// Load order. Zero means the player switched it off.
    pub order: i64,
    /// Whether the widget has saved settings in `data`.
    pub has_settings: bool,
}

impl WidgetState {
    pub fn enabled(&self) -> bool {
        self.order != DISABLED
    }
}

/// The config file as text, edited in place.
#[derive(Debug, Clone)]
pub struct WidgetConfig {
    text: String,
}

/// Where a table's body sits in the text, exclusive of its braces.
#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}

impl WidgetConfig {
    pub fn parse(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Every widget the config knows about, enabled or not.
    pub fn states(&self) -> Result<Vec<WidgetState>, ConfigError> {
        let orders = self.entries("order")?;
        let data = self.entries("data").unwrap_or_default();
        Ok(orders
            .into_iter()
            .map(|(name, order)| WidgetState {
                has_settings: data.contains_key(&name),
                name,
                order,
            })
            .collect())
    }

    pub fn order_of(&self, name: &str) -> Option<i64> {
        self.entries("order").ok()?.get(name).copied()
    }

    /// Switch a widget off, leaving its settings alone.
    ///
    /// Disabling is not deleting: the player keeps their configuration and the
    /// widget comes back where they left it.
    pub fn disable(&mut self, name: &str) -> Result<bool, ConfigError> {
        if ALWAYS_ENABLED.contains(&name) {
            return Err(ConfigError::ForcedOn(name.to_owned()));
        }
        self.set_order(name, DISABLED)
    }

    /// Switch a widget on, adding its entry if BAR has never recorded one.
    ///
    /// Restored to the end of the load order rather than to wherever it used to
    /// sit: `0` erased that, and inventing a position it never had would reorder
    /// somebody else's widgets to hide the fact.
    ///
    /// **A freshly installed widget has no entry, and is off.** `barwidgets.lua`
    /// only gives a default order to widgets loaded from the game archive; a
    /// user widget with no `order` entry is recorded as `0` the first time it
    /// loads. So "Enable" on something just installed has to write the entry
    /// itself, or the player installs a widget and then finds it doing nothing.
    pub fn enable(&mut self, name: &str) -> Result<bool, ConfigError> {
        let order = self.entries("order")?;
        if order.get(name).is_some_and(|order| *order != DISABLED) {
            return Ok(false);
        }
        let next = order.values().copied().max().unwrap_or(0).saturating_add(1);
        if order.contains_key(name) {
            return self.set_order(name, next);
        }
        self.insert_order(name, next)?;
        Ok(true)
    }

    /// Add a new `order` entry at the end of the table.
    fn insert_order(&mut self, name: &str, value: i64) -> Result<(), ConfigError> {
        let span = self.table("order")?;
        let body = &self.text[span.start..span.end];
        // Placed before the closing brace's own line, indented like its
        // neighbours, so the file reads as if BAR had written it.
        let close_line = body
            .rfind('\n')
            .map_or(span.start, |at| span.start + at + 1);
        let entry = format!("\t\t[{}] = {value},\n", quote(name));
        self.text.insert_str(close_line, &entry);
        Ok(())
    }

    /// Remove a widget from the config entirely: order *and* settings.
    ///
    /// Returns what was actually removed, so a caller can report honestly
    /// rather than claiming a clean removal it did not make.
    pub fn remove(&mut self, name: &str) -> Result<Removed, ConfigError> {
        let order = self.remove_entry("order", name)?;
        let settings = self.remove_entry("data", name).unwrap_or(false);
        Ok(Removed {
            order_entry: order,
            settings,
        })
    }

    fn set_order(&mut self, name: &str, value: i64) -> Result<bool, ConfigError> {
        let span = self.table("order")?;
        let body = &self.text[span.start..span.end];
        let Some(found) = find_entry(body, name)? else {
            return Ok(false);
        };
        let line = &body[found.start..found.end];
        let Some(equals) = line.find('=') else {
            return Ok(false);
        };
        let tail = if line[equals + 1..].trim_end().ends_with(',') {
            ","
        } else {
            ""
        };
        let replacement = format!("{} = {value}{tail}", line[..equals].trim_end());
        let at = span.start + found.start;
        self.text
            .replace_range(at..span.start + found.end, &replacement);
        Ok(true)
    }

    fn remove_entry(&mut self, table: &'static str, name: &str) -> Result<bool, ConfigError> {
        let span = self.table(table)?;
        let body = &self.text[span.start..span.end];
        let Some(found) = find_entry(body, name)? else {
            return Ok(false);
        };
        // Take the trailing newline with it, so removal does not leave a blank
        // line where the entry was.
        let mut end = span.start + found.end;
        if self.text[end..].starts_with('\n') {
            end += 1;
        }
        self.text.replace_range(span.start + found.start..end, "");
        Ok(true)
    }

    fn entries(&self, table: &'static str) -> Result<BTreeMap<String, i64>, ConfigError> {
        let span = self.table(table)?;
        let body = &self.text[span.start..span.end];
        let mut found = BTreeMap::new();
        for (name, value) in top_level_keys(body) {
            if found.insert(name.clone(), value).is_some() {
                return Err(ConfigError::Duplicate(name));
            }
        }
        Ok(found)
    }

    /// The body of a top-level `<name> = { ... }` table.
    fn table(&self, name: &'static str) -> Result<Span, ConfigError> {
        let needle = format!("\n\t{name} = {{");
        let at = self
            .text
            .find(&needle)
            .ok_or(ConfigError::MissingTable(name))?;
        let start = at + needle.len();
        let end = match_brace(&self.text, start).ok_or(ConfigError::Unbalanced(name))?;
        Ok(Span { start, end })
    }
}

/// A Lua string literal for a table key, escaped.
fn quote(name: &str) -> String {
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// What a delete actually took out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Removed {
    /// Whether the widget was listed in `order`.
    pub order_entry: bool,
    /// Whether it had saved settings in `data`.
    pub settings: bool,
}

/// Where one entry's text begins and ends within a table body.
#[derive(Debug, Clone, Copy)]
struct Found {
    start: usize,
    end: usize,
}

/// Find one key's whole entry — including a nested table value — in a body.
fn find_entry(body: &str, name: &str) -> Result<Option<Found>, ConfigError> {
    let mut found: Option<Found> = None;
    for (start, key, after_equals) in scan_keys(body) {
        if key != name {
            continue;
        }
        if found.is_some() {
            return Err(ConfigError::Duplicate(name.to_owned()));
        }
        let end = entry_end(body, after_equals).ok_or(ConfigError::Unbalanced("entry"))?;
        found = Some(Found { start, end });
    }
    Ok(found)
}

/// Where an entry's value ends: past a nested table, or at the end of its line.
fn entry_end(body: &str, after_equals: usize) -> Option<usize> {
    let rest = body.get(after_equals..)?;
    let offset = rest.len() - rest.trim_start().len();
    if rest.trim_start().starts_with('{') {
        let open = after_equals + offset + 1;
        let close = match_brace(body, open)?;
        let mut end = close + 1;
        if body[end..].starts_with(',') {
            end += 1;
        }
        return Some(end);
    }
    Some(
        body[after_equals..]
            .find('\n')
            .map(|at| after_equals + at)
            .unwrap_or(body.len()),
    )
}

/// Top-level `key = <integer>` pairs, ignoring anything nested.
fn top_level_keys(body: &str) -> Vec<(String, i64)> {
    scan_keys(body)
        .into_iter()
        .filter_map(|(_, key, after_equals)| {
            let value = body[after_equals..]
                .trim_start()
                .split([',', '\n'])
                .next()?
                .trim()
                .parse::<i64>()
                .unwrap_or(DISABLED);
            Some((key, value))
        })
        .collect()
}

/// Every top-level key in a table body, as `(entry start, key, index after `=`)`.
///
/// Depth-aware and string-aware, so a `}` inside a quoted setting cannot end a
/// table early and a nested widget's own keys are never mistaken for top-level
/// ones.
fn scan_keys(body: &str) -> Vec<(usize, String, usize)> {
    let bytes = body.as_bytes();
    let mut keys = Vec::new();
    let mut depth = 0usize;
    let mut at = 0usize;
    let mut line_start = 0usize;
    while at < bytes.len() {
        match bytes[at] {
            b'-' if bytes.get(at + 1) == Some(&b'-') => {
                at = skip_to_newline(bytes, at);
                line_start = at;
                continue;
            }
            b'"' | b'\'' if depth > 0 => {
                at = skip_string(bytes, at);
                continue;
            }
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b'\n' => line_start = at + 1,
            b'"' | b'\'' | b'[' if depth == 0 => {
                if let Some((key, after)) = read_key(body, at) {
                    keys.push((line_start.max(entry_start(body, line_start)), key, after));
                    at = after;
                    continue;
                }
            }
            c if depth == 0 && (c.is_ascii_alphabetic() || c == b'_') => {
                if let Some((key, after)) = read_bare_key(body, at) {
                    keys.push((entry_start(body, line_start), key, after));
                    at = after;
                    continue;
                }
            }
            _ => {}
        }
        at += 1;
    }
    keys
}

/// The start of the entry's own text: its line, indentation included.
fn entry_start(body: &str, line_start: usize) -> usize {
    line_start.min(body.len())
}

fn read_key(body: &str, at: usize) -> Option<(String, usize)> {
    let bytes = body.as_bytes();
    let (quote, open) = if bytes[at] == b'[' {
        let next = *bytes.get(at + 1)?;
        if next != b'"' && next != b'\'' {
            return None;
        }
        (next, at + 1)
    } else {
        return None;
    };
    let close = find_quote_end(bytes, open, quote)?;
    let key = body.get(open + 1..close)?.to_owned();
    let mut after = close + 1;
    if bytes.get(after) != Some(&b']') {
        return None;
    }
    after += 1;
    after = skip_spaces(bytes, after);
    if bytes.get(after) != Some(&b'=') {
        return None;
    }
    Some((unescape(&key), after + 1))
}

fn read_bare_key(body: &str, at: usize) -> Option<(String, usize)> {
    let bytes = body.as_bytes();
    let mut end = at;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    let key = body.get(at..end)?.to_owned();
    let after = skip_spaces(bytes, end);
    if bytes.get(after) != Some(&b'=') {
        return None;
    }
    Some((key, after + 1))
}

fn find_quote_end(bytes: &[u8], open: usize, quote: u8) -> Option<usize> {
    let mut at = open + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            c if c == quote => return Some(at),
            _ => at += 1,
        }
    }
    None
}

fn unescape(value: &str) -> String {
    value.replace("\\\"", "\"").replace("\\\\", "\\")
}

fn skip_spaces(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && (bytes[at] == b' ' || bytes[at] == b'\t') {
        at += 1;
    }
    at
}

fn skip_to_newline(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && bytes[at] != b'\n' {
        at += 1;
    }
    at
}

fn skip_string(bytes: &[u8], at: usize) -> usize {
    find_quote_end(bytes, at, bytes[at]).map_or(bytes.len(), |end| end + 1)
}

/// The index of the `}` closing a `{` whose body starts at `start`.
fn match_brace(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 1usize;
    let mut at = start;
    while at < bytes.len() {
        match bytes[at] {
            b'-' if bytes.get(at + 1) == Some(&b'-') => {
                at = skip_to_newline(bytes, at);
                continue;
            }
            b'"' | b'\'' => {
                at = skip_string(bytes, at);
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
        at += 1;
    }
    None
}
