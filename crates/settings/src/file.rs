//! The file on disk: `settings.jsonc` beside `settings.schema.json`.
//! Reads go through a JSONC parser; writes edit the concrete syntax tree so
//! the user's comments and layout survive.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use jsonc_parser::{ParseOptions, parse_to_serde_value};
use serde_json::Value;

use crate::model::{SCHEMA_FILE, Settings, schema_json};

pub const FILE_NAME: &str = "settings.jsonc";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Invalid { path: PathBuf, message: String },
}

/// `MODLOBBY_CONFIG_DIR`, else the platform's per-user config dir (`%APPDATA%\modlobby\config` on Windows).
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MODLOBBY_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    directories::ProjectDirs::from("", "", "modlobby")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The settings file plus the in-memory copy the app works from.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
    current: Arc<Mutex<Settings>>,
    /// Hash of what we wrote last, so the watcher can tell our writes from the user's.
    last_written: Arc<Mutex<Option<u64>>>,
}

impl Store {
    /// Creates the directory, the template and the schema on first run, then loads.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, Error> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
            path: dir.clone(),
            source,
        })?;
        let path = dir.join(FILE_NAME);
        if !path.exists() {
            write_atomic(&path, &template())?;
        }
        let schema_path = dir.join(SCHEMA_FILE);
        let schema = schema_json();
        if std::fs::read_to_string(&schema_path).ok().as_deref() != Some(schema.as_str()) {
            write_atomic(&schema_path, &schema)?;
        }
        let settings = load(&path)?;
        Ok(Self {
            dir,
            current: Arc::new(Mutex::new(settings)),
            last_written: Arc::new(Mutex::new(None)),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    pub fn get(&self) -> Settings {
        self.current.lock().expect("settings lock").clone()
    }

    /// Applies `change` to the file as minimal edits — comments, order and
    /// unrelated keys stay as the user left them — and to the in-memory copy.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<Settings, Error> {
        let path = self.path();
        let before = self.get();
        let mut after = before.clone();
        change(&mut after);
        if after == before {
            return Ok(after);
        }
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let root =
            CstRootNode::parse(&text, &ParseOptions::default()).map_err(|err| Error::Invalid {
                path: path.clone(),
                message: err.to_string(),
            })?;
        let object = root.object_value_or_set();
        apply_changes(&object, &to_value(&before), &to_value(&after));
        let edited = root.to_string();
        // A backup of the last user-visible version before we touch it.
        let _ = std::fs::copy(&path, self.dir.join(format!("{FILE_NAME}.bak")));
        write_atomic(&path, &edited)?;
        *self.last_written.lock().expect("hash lock") = Some(hash(&edited));
        *self.current.lock().expect("settings lock") = after.clone();
        Ok(after)
    }

    /// Re-reads the file after an external change. `Ok(None)` means it was our own write.
    pub(crate) fn reload(&self) -> Result<Option<Settings>, Error> {
        let path = self.path();
        let text = std::fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        if *self.last_written.lock().expect("hash lock") == Some(hash(&text)) {
            return Ok(None);
        }
        let settings = parse(&path, &text)?;
        *self.current.lock().expect("settings lock") = settings.clone();
        Ok(Some(settings))
    }
}

pub fn load(path: &Path) -> Result<Settings, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse(path, &text)
}

fn parse(path: &Path, text: &str) -> Result<Settings, Error> {
    parse_to_serde_value(text, &ParseOptions::default()).map_err(|err| Error::Invalid {
        path: path.to_path_buf(),
        message: err.to_string(),
    })
}

/// The first-run file: a header for humans, then the defaults.
pub fn template() -> String {
    let body = serde_json::to_string_pretty(&Settings::initial()).expect("settings serialise");
    format!(
        "// modlobby settings. Edit freely: the app reloads this file when it changes and\n\
         // keeps your comments when it writes. Passwords never live here; they are in\n\
         // the OS keyring. \"$schema\" gives your editor completion and documentation.\n\
         {body}\n"
    )
}

/// Walks both trees; a leaf that differs is set in place, a key that vanished is removed.
fn apply_changes(object: &CstObject, before: &Value, after: &Value) {
    let (Value::Object(before), Value::Object(after)) = (before, after) else {
        return;
    };
    for (key, new) in after {
        let old = before.get(key);
        if old == Some(new) {
            continue;
        }
        match (old, new) {
            (Some(Value::Object(_)), Value::Object(_)) => {
                let child = object.object_value_or_set(key);
                apply_changes(&child, old.expect("checked"), new);
            }
            _ => match object.get(key) {
                Some(prop) => prop.set_value(input_value(new)),
                None => {
                    object.append(key, input_value(new));
                }
            },
        }
    }
    for key in before.keys() {
        if !after.contains_key(key)
            && let Some(prop) = object.get(key)
        {
            prop.remove();
        }
    }
}

fn input_value(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(input_value).collect()),
        Value::Object(map) => CstInputValue::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), input_value(v)))
                .collect(),
        ),
    }
}

fn to_value(settings: &Settings) -> Value {
    serde_json::to_value(settings).expect("settings serialise")
}

fn hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Temp file + rename, so a crash never leaves a half-written settings file.
fn write_atomic(path: &Path, text: &str) -> Result<(), Error> {
    let tmp = path.with_extension("tmp");
    let io = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    std::fs::write(&tmp, text).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_writes_template_and_schema_then_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert!(dir.path().join(FILE_NAME).is_file());
        assert!(dir.path().join(SCHEMA_FILE).is_file());
        assert_eq!(store.get(), Settings::initial());

        let updated = store.update(|s| s.chat.max_lines = 42).unwrap();
        assert_eq!(updated.chat.max_lines, 42);
        assert_eq!(load(&store.path()).unwrap().chat.max_lines, 42);
        assert_eq!(Store::open(dir.path()).unwrap().get().chat.max_lines, 42);
    }

    #[test]
    fn user_comments_and_order_survive_app_writes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let hand_edited = "// my notes\n{\n  // trailing commas are fine,\n  \"chat\": { \"maxLines\": 9 }, // keep me\n  \"server\": { \"port\": 8200, \"tls\": false },\n}\n";
        std::fs::write(store.path(), hand_edited).unwrap();
        let reloaded = store.reload().unwrap().unwrap();
        assert_eq!(reloaded.server.port, 8200);

        store
            .update(|s| {
                s.chat.max_lines = 10;
                s.account.username = "alice".into();
            })
            .unwrap();
        let text = std::fs::read_to_string(store.path()).unwrap();
        assert!(text.starts_with("// my notes"));
        assert!(text.contains("// keep me"));
        assert!(text.contains("\"maxLines\": 10"));
        assert!(text.contains("\"username\": \"alice\""));
        assert!(text.contains("\"port\": 8200"));
        let parsed = load(&store.path()).unwrap();
        assert_eq!(parsed.account.username, "alice");
        assert!(!parsed.server.tls);
    }

    #[test]
    fn own_writes_are_recognised_and_bad_files_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store.update(|s| s.chat.max_lines = 3).unwrap();
        assert!(store.reload().unwrap().is_none(), "our own write");

        std::fs::write(store.path(), "{ not json").unwrap();
        assert!(matches!(store.reload(), Err(Error::Invalid { .. })));
        assert_eq!(
            store.get().chat.max_lines,
            3,
            "invalid input never clobbers"
        );
    }
}
