//! Wiring the dashboard to the real Loop inference engine.
//!
//! The dashboard builds a [`Models`] collection exactly the way the CLI's
//! runtime does, from the same on-disk configuration:
//! * `~/.loop/agent/auth.json`        → stored API keys (read-only)
//! * `~/.loop/agent/settings.json`    → custom providers added with `/login`
//! * `~/.loop/agent/models.json`      → pinned model metadata
//! * `~/.loop/agent/models-store.json`→ cached dynamic catalogs
//!
//! Inference then goes through `Models::stream_simple`, the same code path the
//! agent loop uses. No second engine is introduced: this module only loads
//! configuration and calls `loop-ai`.
//!
//! Keys never leave this process: they are loaded into an in-memory
//! credential store and used by `loop-ai` for outbound requests only.

pub mod params;
pub mod trace;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use loop_ai::auth::{Credential, CredentialStore, InMemoryCredentialStore};
use loop_ai::models::{CreateModelsOptions, Models};
use loop_ai::providers::{custom_provider, CustomModelSpec, CustomProviderConfig};
use loop_ai::types::Model;
use serde::Deserialize;

/// Where the dashboard reads Loop's configuration from.
#[derive(Debug, Clone)]
pub struct EnginePaths {
    /// Agent config dir (`~/.loop/agent`).
    pub agent_dir: PathBuf,
}

impl EnginePaths {
    /// Resolve the agent dir like `loop-app-core` does, honoring
    /// `LOOP_CODING_AGENT_DIR`.
    pub fn resolve() -> Self {
        let agent_dir = match std::env::var_os("LOOP_CODING_AGENT_DIR") {
            Some(v) if !v.is_empty() => PathBuf::from(v),
            _ => dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".loop")
                .join("agent"),
        };
        Self { agent_dir }
    }

    /// auth.json path.
    pub fn auth_path(&self) -> PathBuf {
        self.agent_dir.join("auth.json")
    }

    /// settings.json path.
    pub fn settings_path(&self) -> PathBuf {
        self.agent_dir.join("settings.json")
    }

    /// models.json path.
    pub fn models_json_path(&self) -> PathBuf {
        self.agent_dir.join("models.json")
    }

    /// models-store.json path.
    pub fn models_store_path(&self) -> PathBuf {
        self.agent_dir.join("models-store.json")
    }
}

/// A custom provider entry as stored in settings.json.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CustomProviderEntry {
    id: String,
    name: Option<String>,
    base_url: String,
}

/// Minimal mirror of settings.json (only what engine wiring needs).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SettingsFile {
    providers: Vec<CustomProviderEntry>,
}

/// Minimal mirror of auth.json.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AuthFile {
    providers: HashMap<String, StoredCred>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum StoredCred {
    ApiKey { key: String },
}

/// The wired inference engine.
#[derive(Clone)]
pub struct Engine {
    models: Arc<Models>,
    credentials: Arc<dyn CredentialStore>,
}

impl Engine {
    /// Build the engine from parts of a **live** application runtime.
    ///
    /// Used when the console is embedded in the running Loop process: the
    /// console then streams through the exact `Models` collection and
    /// credential store the app itself uses, instead of a re-read copy of
    /// its on-disk configuration.
    pub fn from_parts(models: Arc<Models>, credentials: Arc<dyn CredentialStore>) -> Self {
        Self {
            models,
            credentials,
        }
    }

    /// Build the engine from Loop's on-disk configuration.
    ///
    /// Never fails hard: unreadable files degrade to empty configuration so
    /// the dashboard can still explain what is missing.
    pub fn load(paths: EnginePaths) -> Self {
        let credentials = Arc::new(InMemoryCredentialStore::new());
        let settings = read_settings(&paths);
        load_credentials(&paths.auth_path(), credentials.as_ref());

        // The catalog cache: FileModelsStore would write back refreshes; the
        // dashboard must not mutate the agent's directory, so we seed each
        // provider's static catalog from the cache file instead.
        let cached_catalogs = read_models_store(&paths.models_store_path());

        let models = Models::create(CreateModelsOptions {
            credentials: Some(credentials.clone()),
            models_store: None,
        });

        // Built-in presets first (soket, openrouter, openai) — same order as
        // `register_providers`.
        for preset in loop_ai::providers::PROVIDER_PRESETS {
            models.set_provider(preset.provider());
        }
        // Custom OpenAI-compatible endpoints from settings.json.
        for entry in &settings.providers {
            models.set_provider(custom_provider(CustomProviderConfig {
                id: entry.id.clone(),
                name: entry.name.clone(),
                base_url: entry.base_url.clone(),
                api_key_env: vec![],
                models: vec![],
                headers: None,
            }));
        }
        // Pinned models from models.json.
        load_custom_models_json(&paths.models_json_path(), &models);

        // Seed dynamic catalogs from the cache so the selector shows the same
        // models the CLI showed at its last refresh.
        for (provider_id, cached) in &cached_catalogs {
            if let Some(provider) = models.get_provider(provider_id) {
                if provider.get_models().is_empty() || provider.is_dynamic() {
                    // Only enrich when the live catalog would otherwise be a
                    // fallback seed; a successful listing wins later.
                    let current = provider.get_models();
                    let is_seed_only = current.len() <= 1;
                    if is_seed_only && !cached.is_empty() {
                        provider.set_models(cached.clone());
                    }
                }
            }
        }

        let _ = &paths; // paths are read above; kept for future diagnostics
        Self {
            models: Arc::new(models),
            credentials,
        }
    }

    /// Refresh dynamic catalogs over the network (best-effort, bounded).
    pub async fn refresh_catalogs(&self) -> HashMap<String, String> {
        let result = self
            .models
            .refresh(loop_ai::ModelsRefreshOptions {
                allow_network: Some(true),
                force: false,
                ..Default::default()
            })
            .await;
        result.errors
    }

    /// All models whose provider is configured (has a usable key or is keyless).
    pub async fn available_models(&self) -> Vec<Model> {
        self.models.get_available().await
    }

    /// Look up one model by provider + id.
    pub fn get_model(&self, provider: &str, id: &str) -> Option<Model> {
        self.models.get_model(provider, id)
    }

    /// Underlying collection (for streaming).
    pub fn models(&self) -> &Arc<Models> {
        &self.models
    }

    /// Provider ids that currently hold a credential.
    pub fn configured_providers(&self) -> Vec<String> {
        self.credentials.list()
    }

    /// Number of registered providers (keyless ones included).
    pub fn provider_count(&self) -> usize {
        self.models.get_providers().len()
    }
}

fn read_settings(paths: &EnginePaths) -> SettingsFile {
    read_json_or_default(&paths.settings_path())
}

fn read_json_or_default<T: Default + for<'de> Deserialize<'de>>(path: &Path) -> T {
    match std::fs::read_to_string(path) {
        Ok(raw) if !raw.trim().is_empty() => serde_json::from_str(&raw).unwrap_or_default(),
        _ => T::default(),
    }
}

fn load_credentials(path: &Path, store: &dyn CredentialStore) {
    let file: AuthFile = read_json_or_default(path);
    for (provider_id, cred) in file.providers {
        let credential = match cred {
            StoredCred::ApiKey { key } => Credential::api_key(key),
        };
        store.set(&provider_id, credential);
    }
}

/// Read the CLI's model-catalog cache (read-only; never written back).
fn read_models_store(path: &Path) -> HashMap<String, Vec<Model>> {
    #[derive(Deserialize)]
    struct Entry {
        models: Vec<Model>,
    }
    // The file shape is one entry per provider: {"soket": {"models": [...]}}.
    #[derive(Deserialize)]
    #[serde(transparent)]
    struct Wrapper(HashMap<String, Entry>);

    let raw = match std::fs::read_to_string(path) {
        Ok(raw) if !raw.trim().is_empty() => raw,
        _ => return HashMap::new(),
    };
    let parsed: Result<Wrapper, _> = serde_json::from_str(&raw);
    if let Ok(wrapper) = parsed {
        return wrapper.0.into_iter().map(|(k, v)| (k, v.models)).collect();
    }
    HashMap::new()
}

/// Load pinned models from models.json (same shape as the CLI's loader).
fn load_custom_models_json(path: &Path, models: &Models) {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) if !raw.trim().is_empty() => raw,
        _ => return,
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let Some(arr) = value.get("providers").and_then(|v| v.as_array()) else {
        return;
    };
    for p in arr {
        let Some(id) = p.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let base_url = p
            .get("baseUrl")
            .or_else(|| p.get("base_url"))
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:11434/v1");
        let api_key_env = p
            .get("apiKeyEnv")
            .or_else(|| p.get("api_key_env"))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let specs = p
            .get("models")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        if let Some(s) = m.as_str() {
                            return Some(CustomModelSpec::new(s));
                        }
                        let mid = m.get("id")?.as_str()?;
                        let mut spec = CustomModelSpec::new(mid)
                            .with_reasoning(
                                m.get("reasoning")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false),
                            )
                            .with_name(m.get("name").and_then(|v| v.as_str()));
                        if let Some(ctx) = m.get("contextWindow").and_then(|v| v.as_u64()) {
                            spec = spec.with_context_window(Some(ctx));
                        }
                        if let Some(mt) = m.get("maxTokens").and_then(|v| v.as_u64()) {
                            spec = spec.with_max_tokens(Some(mt));
                        }
                        Some(spec)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if specs.is_empty() {
            continue;
        }
        models.set_provider(custom_provider(CustomProviderConfig {
            id: id.into(),
            name: p.get("name").and_then(|v| v.as_str()).map(String::from),
            base_url: base_url.into(),
            api_key_env,
            models: specs,
            headers: None,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_loads_from_real_config_dir() {
        // Uses whatever ~/.loop/agent contains on this machine; must not panic
        // even when files are absent.
        let engine = Engine::load(EnginePaths::resolve());
        let _ = engine.available_models_blocking_for_test();
    }

    impl Engine {
        fn available_models_blocking_for_test(&self) -> Vec<Model> {
            futures::executor::block_on(self.available_models())
        }
    }
}
