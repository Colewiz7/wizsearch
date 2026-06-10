//! Typed settings registry. Every user-facing behavior registers here with a
//! key, default, description, and schema; the Settings UI is generated from
//! these defs. Values persist in the SQLite settings table as JSON. Secrets are
//! a special kind: their values live ONLY in the OS keychain.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use serde::Serialize;
use serde_json::{json, Value};

use crate::sources::SourceDescriptor;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingKind {
    Bool {
        default: bool,
    },
    Int {
        default: i64,
        min: i64,
        max: i64,
    },
    Float {
        default: f64,
        min: f64,
        max: f64,
    },
    Text {
        default: String,
        placeholder: String,
    },
    Select {
        default: String,
        options: Vec<String>,
    },
    /// stored in the OS keychain, never in the DB. value over IPC is set/unset only
    Secret {
        help_url: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingDef {
    pub key: String,
    pub label: String,
    pub description: String,
    pub category: String,
    pub kind: SettingKind,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("unknown setting '{0}'")]
    UnknownKey(String),
    #[error("invalid value for '{key}': {reason}")]
    InvalidValue { key: String, reason: String },
    #[error("'{0}' is a secret; use secret_set so it goes to the OS keychain")]
    SecretViaSettings(String),
    #[error("storage: {0}")]
    Storage(String),
}

/// Persistence the store writes through to (implemented by collection::Db).
pub trait SettingsBackend: Send + Sync {
    fn load_all(&self) -> Result<HashMap<String, String>, String>;
    fn save(&self, key: &str, json_value: &str) -> Result<(), String>;
}

pub struct SettingsStore {
    defs: Vec<SettingDef>,
    values: RwLock<HashMap<String, Value>>,
    backend: Mutex<Option<Box<dyn SettingsBackend>>>,
}

impl SettingsStore {
    pub fn new(defs: Vec<SettingDef>) -> Self {
        SettingsStore {
            defs,
            values: RwLock::new(HashMap::new()),
            backend: Mutex::new(None),
        }
    }

    /// attach SQLite persistence and pull stored values into memory
    pub fn attach_backend(&self, backend: Box<dyn SettingsBackend>) -> Result<(), SettingsError> {
        let stored = backend.load_all().map_err(SettingsError::Storage)?;
        {
            let mut values = self.values.write().expect("settings lock");
            for (k, raw) in stored {
                // ignore rows for settings that no longer exist or won't parse
                let Some(def) = self.defs.iter().find(|d| d.key == k) else {
                    continue;
                };
                let Ok(v) = serde_json::from_str::<Value>(&raw) else {
                    continue;
                };
                if validate(def, &v).is_ok() {
                    values.insert(k, v);
                }
            }
        }
        *self.backend.lock().expect("settings backend lock") = Some(backend);
        Ok(())
    }

    pub fn defs(&self) -> &[SettingDef] {
        &self.defs
    }

    pub fn def(&self, key: &str) -> Option<&SettingDef> {
        self.defs.iter().find(|d| d.key == key)
    }

    pub fn default_value(def: &SettingDef) -> Value {
        match &def.kind {
            SettingKind::Bool { default } => json!(default),
            SettingKind::Int { default, .. } => json!(default),
            SettingKind::Float { default, .. } => json!(default),
            SettingKind::Text { default, .. } => json!(default),
            SettingKind::Select { default, .. } => json!(default),
            SettingKind::Secret { .. } => Value::Null,
        }
    }

    pub fn get(&self, key: &str) -> Result<Value, SettingsError> {
        let def = self
            .def(key)
            .ok_or_else(|| SettingsError::UnknownKey(key.to_string()))?;
        if let Some(v) = self.values.read().expect("settings lock").get(key) {
            return Ok(v.clone());
        }
        Ok(Self::default_value(def))
    }

    pub fn set(&self, key: &str, value: Value) -> Result<(), SettingsError> {
        let def = self
            .def(key)
            .ok_or_else(|| SettingsError::UnknownKey(key.to_string()))?;
        if matches!(def.kind, SettingKind::Secret { .. }) {
            return Err(SettingsError::SecretViaSettings(key.to_string()));
        }
        validate(def, &value)?;
        if let Some(b) = self.backend.lock().expect("settings backend lock").as_ref() {
            b.save(key, &value.to_string())
                .map_err(SettingsError::Storage)?;
        }
        self.values
            .write()
            .expect("settings lock")
            .insert(key.to_string(), value);
        Ok(())
    }

    /// every non-secret setting's effective value (stored or default)
    pub fn all_values(&self) -> HashMap<String, Value> {
        self.defs
            .iter()
            .filter(|d| !matches!(d.kind, SettingKind::Secret { .. }))
            .map(|d| {
                let v = self
                    .values
                    .read()
                    .expect("settings lock")
                    .get(&d.key)
                    .cloned()
                    .unwrap_or_else(|| Self::default_value(d));
                (d.key.clone(), v)
            })
            .collect()
    }

    // typed helpers for host code; fall back to the given default if the key
    // isn't registered (callers pass the registered default anyway)
    pub fn bool_or(&self, key: &str, fallback: bool) -> bool {
        self.get(key)
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(fallback)
    }

    pub fn i64_or(&self, key: &str, fallback: i64) -> i64 {
        self.get(key)
            .ok()
            .and_then(|v| v.as_i64())
            .unwrap_or(fallback)
    }

    pub fn string_or(&self, key: &str, fallback: &str) -> String {
        self.get(key)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| fallback.to_string())
    }
}

fn validate(def: &SettingDef, value: &Value) -> Result<(), SettingsError> {
    let bad = |reason: &str| {
        Err(SettingsError::InvalidValue {
            key: def.key.clone(),
            reason: reason.to_string(),
        })
    };
    match &def.kind {
        SettingKind::Bool { .. } => {
            if !value.is_boolean() {
                return bad("expected a boolean");
            }
        }
        SettingKind::Int { min, max, .. } => match value.as_i64() {
            Some(n) if n >= *min && n <= *max => {}
            Some(_) => return bad(&format!("must be between {min} and {max}")),
            None => return bad("expected an integer"),
        },
        SettingKind::Float { min, max, .. } => match value.as_f64() {
            Some(n) if n >= *min && n <= *max => {}
            Some(_) => return bad(&format!("must be between {min} and {max}")),
            None => return bad("expected a number"),
        },
        SettingKind::Text { .. } => {
            if !value.is_string() {
                return bad("expected a string");
            }
        }
        SettingKind::Select { options, .. } => match value.as_str() {
            Some(s) if options.iter().any(|o| o == s) => {}
            _ => return bad(&format!("must be one of: {}", options.join(", "))),
        },
        SettingKind::Secret { .. } => return bad("secrets are keychain-only"),
    }
    Ok(())
}

/// Build the full registry. Per-source settings are generated from descriptors
/// so adding a source automatically adds its settings.
pub fn build_registry(
    descriptors: &[&SourceDescriptor],
    default_collection_dir: &str,
) -> Vec<SettingDef> {
    let mut defs = vec![
        // --- search ---
        SettingDef {
            key: "search.timeout_ms".into(),
            label: "Source timeout (ms)".into(),
            description: "How long to wait for each source before showing results without it.".into(),
            category: "Search".into(),
            kind: SettingKind::Int { default: 8000, min: 1000, max: 60000 },
        },
        SettingDef {
            key: "search.page_size".into(),
            label: "Results per source".into(),
            description: "How many results to ask each source for per page.".into(),
            category: "Search".into(),
            kind: SettingKind::Int { default: 24, min: 4, max: 80 },
        },
        SettingDef {
            key: "search.merge_strategy".into(),
            label: "Result merging".into(),
            description: "round_robin interleaves sources into one grid; grouped keeps each source's results together.".into(),
            category: "Search".into(),
            kind: SettingKind::Select {
                default: "round_robin".into(),
                options: vec!["round_robin".into(), "grouped".into()],
            },
        },
        SettingDef {
            key: "search.search_as_you_type".into(),
            label: "Search as you type".into(),
            description: "Run the search automatically after you stop typing (otherwise press Enter).".into(),
            category: "Search".into(),
            kind: SettingKind::Bool { default: true },
        },
        SettingDef {
            key: "search.debounce_ms".into(),
            label: "Type-to-search delay (ms)".into(),
            description: "How long to wait after typing stops before searching.".into(),
            category: "Search".into(),
            kind: SettingKind::Int { default: 450, min: 100, max: 2000 },
        },
        // --- preview ---
        SettingDef {
            key: "preview.hover_to_play".into(),
            label: "Play previews on hover".into(),
            description: "Loop GIFs and videos when the pointer is over a tile.".into(),
            category: "Preview".into(),
            kind: SettingKind::Bool { default: true },
        },
        SettingDef {
            key: "preview.hover_delay_ms".into(),
            label: "Hover delay (ms)".into(),
            description: "Delay before a hovered tile starts playing.".into(),
            category: "Preview".into(),
            kind: SettingKind::Int { default: 120, min: 0, max: 1500 },
        },
        SettingDef {
            key: "preview.audio_volume".into(),
            label: "Preview volume".into(),
            description: "Volume for audio previews (0 to 1).".into(),
            category: "Preview".into(),
            kind: SettingKind::Float { default: 0.8, min: 0.0, max: 1.0 },
        },
        SettingDef {
            key: "preview.mute_video_previews".into(),
            label: "Mute video previews".into(),
            description: "Keep hover video loops silent.".into(),
            category: "Preview".into(),
            kind: SettingKind::Bool { default: true },
        },
        // --- grid ---
        SettingDef {
            key: "grid.tile_min_width".into(),
            label: "Tile size (px)".into(),
            description: "Minimum tile width in the result grid; fewer columns means bigger tiles.".into(),
            category: "Grid".into(),
            kind: SettingKind::Int { default: 220, min: 120, max: 480 },
        },
        SettingDef {
            key: "grid.show_source_badges".into(),
            label: "Show source badges".into(),
            description: "Show which source each result came from on its tile.".into(),
            category: "Grid".into(),
            kind: SettingKind::Bool { default: true },
        },
        // --- collection ---
        SettingDef {
            key: "collection.dir".into(),
            label: "Collection folder".into(),
            description: "Where collected assets are stored. Never committed to git.".into(),
            category: "Collection".into(),
            kind: SettingKind::Text {
                default: default_collection_dir.to_string(),
                placeholder: "~/WizSearch".into(),
            },
        },
        SettingDef {
            key: "collection.make_thumbnails".into(),
            label: "Generate thumbnails".into(),
            description: "Use ffmpeg to generate poster thumbnails for collected videos.".into(),
            category: "Collection".into(),
            kind: SettingKind::Bool { default: true },
        },
        // --- sidecars ---
        SettingDef {
            key: "sidecars.auto_download".into(),
            label: "Download tools on first run".into(),
            description: "Fetch ffmpeg/ffprobe/yt-dlp automatically (SHA-256 verified) when missing.".into(),
            category: "Tools".into(),
            kind: SettingKind::Bool { default: true },
        },
        SettingDef {
            key: "sidecars.ytdlp_self_update".into(),
            label: "Let yt-dlp self-update".into(),
            description: "yt-dlp breaks when sites change; allow it to update itself independently of its pinned install.".into(),
            category: "Tools".into(),
            kind: SettingKind::Bool { default: true },
        },
        // --- ui ---
        SettingDef {
            key: "ui.theme".into(),
            label: "Theme".into(),
            description: "App color theme.".into(),
            category: "Appearance".into(),
            kind: SettingKind::Select {
                default: "dark".into(),
                options: vec!["dark".into(), "light".into()],
            },
        },
    ];

    for d in descriptors {
        defs.push(SettingDef {
            key: format!("sources.{}.enabled", d.id),
            label: format!("Enable {}", d.name),
            description: format!("Include {} when searching.", d.name),
            category: "Sources".into(),
            kind: SettingKind::Bool { default: true },
        });
        defs.push(SettingDef {
            key: format!("sources.{}.rate_limit_per_min", d.id),
            label: format!("{} rate limit (req/min)", d.name),
            description: format!(
                "Max requests per minute sent to {}. Lower this if you hit limits.",
                d.name
            ),
            category: "Sources".into(),
            kind: SettingKind::Int {
                default: d.default_rate_limit_per_min as i64,
                min: 1,
                max: 600,
            },
        });
        if d.requires_key {
            defs.push(SettingDef {
                key: crate::security::credential_key(d.id),
                label: format!("{} API key", d.name),
                description: format!(
                    "Your personal free {} key. Stored in the OS keychain, never on disk.",
                    d.name
                ),
                category: "Sources".into(),
                kind: SettingKind::Secret {
                    help_url: d.key_help_url.to_string(),
                },
            });
        }
    }
    defs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SettingsStore {
        SettingsStore::new(build_registry(&[], "/tmp/c"))
    }

    #[test]
    fn defaults_come_back_without_backend() {
        let s = store();
        assert_eq!(s.i64_or("search.timeout_ms", 0), 8000);
        assert_eq!(s.string_or("search.merge_strategy", ""), "round_robin");
        assert!(s.bool_or("preview.hover_to_play", false));
    }

    #[test]
    fn validation_rejects_bad_values() {
        let s = store();
        assert!(s.set("search.timeout_ms", json!(50)).is_err()); // below min
        assert!(s.set("search.merge_strategy", json!("randomly")).is_err());
        assert!(s.set("search.merge_strategy", json!("grouped")).is_ok());
        assert!(s.set("nope.nope", json!(true)).is_err());
    }

    #[test]
    fn secrets_cannot_go_through_settings() {
        let descs: Vec<&'static crate::sources::SourceDescriptor> = vec![];
        let _ = descs;
        let s = SettingsStore::new(vec![SettingDef {
            key: "source.x.api_key".into(),
            label: "X key".into(),
            description: String::new(),
            category: "Sources".into(),
            kind: SettingKind::Secret {
                help_url: String::new(),
            },
        }]);
        assert!(matches!(
            s.set("source.x.api_key", json!("leaky")),
            Err(SettingsError::SecretViaSettings(_))
        ));
    }
}
