use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// An enum saved as a plain string id in config.json. An unknown id (a typo,
/// a value from a newer version) reads as the default instead of failing the
/// whole file.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident (default $default:ident) {
            $($(#[$vmeta:meta])* $variant:ident = $id:literal $(| $alias:literal)*,)+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(from = "String", into = "String")]
        pub enum $name {
            $($(#[$vmeta])* $variant,)+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant,)+];

            pub fn id(self) -> &'static str {
                match self {
                    $($name::$variant => $id,)+
                }
            }

            pub fn parse(id: &str) -> Option<Self> {
                match id.trim() {
                    $($id $(| $alias)* => Some($name::$variant),)+
                    _ => None,
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                $name::$default
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                Self::parse(&id).unwrap_or_default()
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.id().to_string()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.id())
            }
        }
    };
}

string_enum! {
    /// What turns the speech into text.
    Engine (default Groq) {
        Local = "local",
        Groq = "groq",
        Polza = "polza",
        AssemblyAi = "assemblyai",
        /// OpenAI or any OpenAI-compatible endpoint
        OpenAi = "openai" | "openai-compatible",
    }
}

impl Engine {
    /// Name for logs, messages and the tray menu (English)
    pub fn label(self) -> &'static str {
        match self {
            Engine::Local => "Local",
            Engine::Groq => "Groq",
            Engine::Polza => "Polza",
            Engine::AssemblyAi => "AssemblyAI",
            Engine::OpenAi => "OpenAI Compatible",
        }
    }

    pub fn is_cloud(self) -> bool {
        self != Engine::Local
    }
}

string_enum! {
    RecordingMode (default Toggle) {
        /// The hotkey starts a dictation, the next press stops it
        Toggle = "toggle",
        /// Records while the hotkey is held
        PushToTalk = "push-to-talk",
    }
}

string_enum! {
    /// Where the recording indicator appears while dictating.
    IndicatorPosition (default TopRight) {
        /// Top-right corner, slides down from behind the top edge
        TopRight = "top-right",
        /// Bottom centre just above the taskbar, slides up from behind it
        BottomCenter = "bottom-center",
    }
}

string_enum! {
    /// Service that runs the post-processing model.
    PostProvider (default Gemini) {
        Gemini = "gemini",
        OpenRouter = "openrouter",
        Groq = "groq",
        Polza = "polza",
    }
}

string_enum! {
    /// Post-processing style.
    Preset (default Proper) {
        Chill = "chill",
        /// "official" is the old name of this preset
        Proper = "proper" | "official",
        Custom = "custom",
    }
}

/// Languages offered for recognition; empty means detect automatically.
pub const RECOGNITION_LANGUAGES: &[&str] = &["ru", "en", "uk", "de", "fr", "es", "it", "pt", "pl"];

fn default_true() -> bool {
    true
}

/// API key and chosen model of one post-processing provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PostProcessProvider {
    #[serde(rename = "apiKey")]
    pub api_key: String,
    pub model: String,
    /// Sub-provider that serves the model (Polza only); empty means automatic
    #[serde(rename = "providerId", skip_serializing_if = "String::is_empty")]
    pub provider_id: String,
}

/// A language model rewrites the transcribed text before it is pasted.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PostProcess {
    pub enabled: bool,
    pub provider: PostProvider,
    pub preset: Preset,
    /// The user's own instructions for the "custom" preset
    #[serde(rename = "customPrompt")]
    pub custom_prompt: String,
    pub gemini: PostProcessProvider,
    pub openrouter: PostProcessProvider,
    pub groq: PostProcessProvider,
    pub polza: PostProcessProvider,
}

impl PostProcess {
    pub fn provider_settings(&self, provider: PostProvider) -> &PostProcessProvider {
        match provider {
            PostProvider::Gemini => &self.gemini,
            PostProvider::OpenRouter => &self.openrouter,
            PostProvider::Groq => &self.groq,
            PostProvider::Polza => &self.polza,
        }
    }

    fn providers(&self) -> impl Iterator<Item = (PostProvider, &PostProcessProvider)> {
        PostProvider::ALL
            .iter()
            .map(move |&provider| (provider, self.provider_settings(provider)))
    }
}

/// Everything in config.json. A field missing from the file gets its value
/// from `Default` below.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub microphone: String,
    pub engine: Engine,
    #[serde(rename = "groqApiKey")]
    pub groq_api_key: String,
    /// "whisper-large-v3-turbo" or "whisper-large-v3"
    #[serde(rename = "groqModel")]
    pub groq_model: String,
    #[serde(rename = "openaiEndpoint")]
    pub openai_endpoint: String,
    #[serde(rename = "openaiModel")]
    pub openai_model: String,
    #[serde(rename = "openaiApiKey")]
    pub openai_api_key: String,
    #[serde(rename = "polzaApiKey")]
    pub polza_api_key: String,
    #[serde(rename = "polzaModel")]
    pub polza_model: String,
    #[serde(rename = "polzaProvider")]
    pub polza_provider: String,
    #[serde(rename = "assemblyaiApiKey")]
    pub assemblyai_api_key: String,
    /// "universal-3-5-pro" (no Russian) or "universal-2"
    #[serde(rename = "assemblyaiModel")]
    pub assemblyai_model: String,
    /// Language of the speech, e.g. "ru"; empty detects it
    #[serde(rename = "recognitionLanguage")]
    pub recognition_language: String,
    /// A failed cloud transcription is retried with a downloaded local model
    #[serde(rename = "fallbackLocal")]
    pub fallback_local: bool,
    /// Local engine: the model used for dictation (an id from local::catalog)
    #[serde(rename = "localModel")]
    pub local_model: String,
    /// Local engine: "vulkan" | "cpu", or empty for the one recommended
    /// for this computer
    #[serde(rename = "localBackend")]
    pub local_backend: String,
    /// Local engine: when an idle model leaves memory:
    /// "immediate" | "30s" | "5m" | "10m" | "never"
    #[serde(rename = "localUnload")]
    pub local_unload: String,
    #[serde(rename = "recordingMode")]
    pub recording_mode: RecordingMode,
    pub hotkey: String,
    /// Adds a space after the pasted text, so the next dictation doesn't stick to it
    #[serde(rename = "appendSpace")]
    pub append_space: bool,
    /// Recording indicator that slides in at the edge of the screen while dictating
    #[serde(rename = "showIndicator")]
    pub show_indicator: bool,
    #[serde(rename = "indicatorPosition")]
    pub indicator_position: IndicatorPosition,
    /// Interface language: "en", "ru", or empty to follow the system
    pub language: String,
    #[serde(rename = "postProcess")]
    pub post_process: PostProcess,
    /// Looks for a newer release on GitHub
    #[serde(rename = "checkUpdates")]
    pub check_updates: bool,
    /// The first-run setup was finished or skipped. A config file from before
    /// the setup existed counts as set up.
    #[serde(rename = "setupDone", default = "default_true")]
    pub setup_done: bool,
    /// Shows the Developer section and turns on log collection
    #[serde(rename = "developerMode")]
    pub developer_mode: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            microphone: "default".to_string(),
            engine: Engine::default(),
            groq_api_key: String::new(),
            groq_model: "whisper-large-v3-turbo".to_string(),
            openai_endpoint: "https://api.openai.com/v1".to_string(),
            openai_model: "whisper-1".to_string(),
            openai_api_key: String::new(),
            polza_api_key: String::new(),
            polza_model: "openai/whisper-large-v3".to_string(),
            polza_provider: String::new(),
            assemblyai_api_key: String::new(),
            assemblyai_model: "universal-3-5-pro".to_string(),
            recognition_language: String::new(),
            fallback_local: true,
            local_model: crate::local::catalog::DEFAULT_MODEL.to_string(),
            local_backend: String::new(),
            local_unload: "5m".to_string(),
            recording_mode: RecordingMode::default(),
            hotkey: "Ctrl+Shift+Space".to_string(),
            append_space: false,
            show_indicator: true,
            indicator_position: IndicatorPosition::default(),
            language: String::new(),
            post_process: PostProcess::default(),
            check_updates: true,
            setup_done: false,
            developer_mode: false,
        }
    }
}

fn key_state(key: &str) -> &'static str {
    if key.trim().is_empty() {
        "not set"
    } else {
        "set"
    }
}

fn or_auto(value: &str) -> &str {
    if value.is_empty() {
        "auto"
    } else {
        value
    }
}

impl Settings {
    pub fn config_path(app_dir: &PathBuf) -> PathBuf {
        app_dir.join("config.json")
    }

    pub fn load(app_dir: &PathBuf) -> Self {
        let path = Self::config_path(app_dir);
        match fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// The recognition language as an API expects it: `None` detects it.
    pub fn language_hint(&self) -> Option<&str> {
        let code = self.recognition_language.trim();
        (!code.is_empty()).then_some(code)
    }

    /// The Engine key of a provider that is also used for post-processing.
    pub fn engine_key(&self, provider: PostProvider) -> &str {
        match provider {
            PostProvider::Groq => self.groq_api_key.as_str(),
            PostProvider::Polza => self.polza_api_key.as_str(),
            _ => "",
        }
    }

    /// API key for post-processing with `provider`. Groq and Polza fall back
    /// to the key from Engine when no separate one is set.
    pub fn post_process_key(&self, provider: PostProvider) -> String {
        let own = self.post_process.provider_settings(provider).api_key.trim();
        if own.is_empty() {
            self.engine_key(provider).trim().to_string()
        } else {
            own.to_string()
        }
    }

    pub fn save(&self, app_dir: &PathBuf) -> Result<(), String> {
        let path = Self::config_path(app_dir);
        fs::create_dir_all(app_dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| e.to_string())
    }

    /// One-line description for logs. API keys are never included,
    /// only whether they are set.
    pub fn summary(&self) -> String {
        format!(
            "engine={}, mode={}, hotkey={}, appendSpace={}, mic='{}', recognitionLanguage={}, fallbackLocal={}, groqModel={}, groqKey={}, openaiEndpoint={}, openaiModel={}, openaiKey={}, polzaModel={}, polzaProvider={}, polzaKey={}, assemblyaiModel={}, assemblyaiKey={}, localModel={}, localBackend={}, localUnload={}, showIndicator={}, indicatorPosition={}, language={}, postProcess={}, checkUpdates={}, developerMode={}",
            self.engine,
            self.recording_mode,
            self.hotkey,
            self.append_space,
            self.microphone,
            or_auto(&self.recognition_language),
            self.fallback_local,
            self.groq_model,
            key_state(&self.groq_api_key),
            self.openai_endpoint,
            self.openai_model,
            key_state(&self.openai_api_key),
            self.polza_model,
            or_auto(&self.polza_provider),
            key_state(&self.polza_api_key),
            self.assemblyai_model,
            key_state(&self.assemblyai_api_key),
            self.local_model,
            or_auto(&self.local_backend),
            self.local_unload,
            self.show_indicator,
            self.indicator_position,
            if self.language.is_empty() { "system" } else { self.language.as_str() },
            self.post_process_summary(),
            self.check_updates,
            self.developer_mode
        )
    }

    fn post_process_summary(&self) -> String {
        let pp = &self.post_process;
        let model = pp.provider_settings(pp.provider).model.as_str();
        format!(
            "{} (provider {}, model {}, preset {}, key {})",
            if pp.enabled { "on" } else { "off" },
            pp.provider,
            if model.is_empty() { "none" } else { model },
            pp.preset,
            key_state(&self.post_process_key(pp.provider))
        )
    }

    /// Human-readable list of what differs from `old`, with API keys masked.
    pub fn describe_changes(&self, old: &Settings) -> Vec<String> {
        let mut changes = Vec::new();
        let mut changed = |name: &str, before: &dyn std::fmt::Display, after: &dyn std::fmt::Display| {
            let (before, after) = (before.to_string(), after.to_string());
            if before != after {
                changes.push(format!("{}: '{}' → '{}'", name, before, after));
            }
        };

        changed("engine", &old.engine, &self.engine);
        changed("microphone", &old.microphone, &self.microphone);
        changed("recordingMode", &old.recording_mode, &self.recording_mode);
        changed("hotkey", &old.hotkey, &self.hotkey);
        changed("recognitionLanguage", &old.recognition_language, &self.recognition_language);
        changed("fallbackLocal", &old.fallback_local, &self.fallback_local);
        changed("groqModel", &old.groq_model, &self.groq_model);
        changed("openaiEndpoint", &old.openai_endpoint, &self.openai_endpoint);
        changed("openaiModel", &old.openai_model, &self.openai_model);
        changed("polzaModel", &old.polza_model, &self.polza_model);
        changed("polzaProvider", &old.polza_provider, &self.polza_provider);
        changed("assemblyaiModel", &old.assemblyai_model, &self.assemblyai_model);
        changed("localModel", &old.local_model, &self.local_model);
        changed("localBackend", &old.local_backend, &self.local_backend);
        changed("localUnload", &old.local_unload, &self.local_unload);
        changed("language", &old.language, &self.language);
        changed("appendSpace", &old.append_space, &self.append_space);
        changed("showIndicator", &old.show_indicator, &self.show_indicator);
        changed("indicatorPosition", &old.indicator_position, &self.indicator_position);
        changed("checkUpdates", &old.check_updates, &self.check_updates);
        changed("setupDone", &old.setup_done, &self.setup_done);
        changed("developerMode", &old.developer_mode, &self.developer_mode);

        let pp_old = &old.post_process;
        let pp_new = &self.post_process;
        changed("postProcess.enabled", &pp_old.enabled, &pp_new.enabled);
        changed("postProcess.provider", &pp_old.provider, &pp_new.provider);
        changed("postProcess.preset", &pp_old.preset, &pp_new.preset);
        for ((provider, before), (_, after)) in pp_old.providers().zip(pp_new.providers()) {
            changed(&format!("postProcess.{}.model", provider), &before.model, &after.model);
            changed(
                &format!("postProcess.{}.providerId", provider),
                &before.provider_id,
                &after.provider_id,
            );
        }

        let secrets = [
            ("groqApiKey".to_string(), &old.groq_api_key, &self.groq_api_key),
            ("openaiApiKey".to_string(), &old.openai_api_key, &self.openai_api_key),
            ("polzaApiKey".to_string(), &old.polza_api_key, &self.polza_api_key),
            ("assemblyaiApiKey".to_string(), &old.assemblyai_api_key, &self.assemblyai_api_key),
        ];
        let post_secrets = pp_old.providers().zip(pp_new.providers()).map(|((provider, before), (_, after))| {
            (format!("postProcess.{}.apiKey", provider), &before.api_key, &after.api_key)
        });
        for (name, before, after) in secrets.into_iter().chain(post_secrets) {
            if before != after {
                changes.push(format!("{} updated ({})", name, key_state(after)));
            }
        }

        if pp_old.custom_prompt != pp_new.custom_prompt {
            changes.push(format!(
                "postProcess.customPrompt updated ({} characters)",
                pp_new.custom_prompt.chars().count()
            ));
        }

        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn test_default_settings() {
        let settings = Settings::default();
        assert_eq!(settings.microphone, "default");
        assert_eq!(settings.engine, Engine::Groq);
        assert_eq!(settings.groq_api_key, "");
        assert_eq!(settings.groq_model, "whisper-large-v3-turbo");
        assert!(!settings.append_space);
        assert!(!settings.post_process.enabled);
        assert_eq!(settings.post_process.provider, PostProvider::Gemini);
        assert_eq!(settings.post_process.preset, Preset::Proper);
        assert_eq!(settings.openai_endpoint, "https://api.openai.com/v1");
        assert_eq!(settings.openai_model, "whisper-1");
        assert_eq!(settings.polza_model, "openai/whisper-large-v3");
        assert_eq!(settings.assemblyai_model, "universal-3-5-pro");
        assert_eq!(settings.recognition_language, "");
        assert!(settings.fallback_local);
        assert_eq!(settings.local_model, "parakeet-tdt-0.6b-v3");
        assert_eq!(settings.local_backend, "");
        assert_eq!(settings.local_unload, "5m");
        assert_eq!(settings.language, "");
        assert_eq!(settings.recording_mode, RecordingMode::Toggle);
        assert_eq!(settings.hotkey, "Ctrl+Shift+Space");
        assert!(settings.show_indicator);
        assert_eq!(settings.indicator_position, IndicatorPosition::TopRight);
        assert!(settings.check_updates);
        assert!(!settings.setup_done);
        assert!(!settings.developer_mode);
    }

    #[test]
    fn test_missing_fields_get_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"engine":"local"}"#).unwrap();
        assert_eq!(settings.engine, Engine::Local);
        assert_eq!(settings.microphone, "default");
        assert_eq!(settings.local_model, "parakeet-tdt-0.6b-v3");
        assert_eq!(settings.local_unload, "5m");
        assert_eq!(settings.post_process, PostProcess::default());
        assert!(settings.show_indicator);
        assert_eq!(settings.indicator_position, IndicatorPosition::TopRight);
        assert!(settings.fallback_local);
        assert!(settings.check_updates);
        assert!(!settings.developer_mode);
    }

    #[test]
    fn test_a_config_from_before_the_setup_counts_as_set_up() {
        let settings: Settings = serde_json::from_str(r#"{"microphone":"default"}"#).unwrap();
        assert!(settings.setup_done);
    }

    #[test]
    fn test_ids_round_trip_and_unknown_ids_get_the_default() {
        for &engine in Engine::ALL {
            assert_eq!(Engine::parse(engine.id()), Some(engine));
        }
        assert_eq!(Engine::from("openai-compatible".to_string()), Engine::OpenAi);
        assert_eq!(Engine::from("whatever".to_string()), Engine::Groq);
        assert_eq!(Preset::from("official".to_string()), Preset::Proper);
        assert_eq!(RecordingMode::from("push-to-talk".to_string()), RecordingMode::PushToTalk);
        for &position in IndicatorPosition::ALL {
            assert_eq!(IndicatorPosition::parse(position.id()), Some(position));
        }

        let settings: Settings = serde_json::from_str(
            r#"{"engine":"nonsense","indicatorPosition":"bottom-center","postProcess":{"provider":"openrouter"}}"#,
        )
        .unwrap();
        assert_eq!(settings.engine, Engine::Groq);
        assert_eq!(settings.indicator_position, IndicatorPosition::BottomCenter);
        assert_eq!(settings.post_process.provider, PostProvider::OpenRouter);

        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(json.contains(r#""engine":"groq""#));
        assert!(json.contains(r#""recordingMode":"toggle""#));
        assert!(json.contains(r#""indicatorPosition":"top-right""#));
    }

    #[test]
    fn test_indicator_position_change_is_logged() {
        let old = Settings::default();
        let mut new = Settings::default();
        new.indicator_position = IndicatorPosition::BottomCenter;
        let changes = new.describe_changes(&old).join("; ");
        assert!(changes.contains("indicatorPosition: 'top-right' → 'bottom-center'"));
        assert!(new.summary().contains("indicatorPosition=bottom-center"));
    }

    #[test]
    fn test_logs_never_contain_api_keys() {
        let old = Settings::default();
        let mut new = Settings::default();
        new.groq_api_key = "gsk_secret".to_string();
        new.assemblyai_api_key = "aai_secret".to_string();
        new.post_process.gemini.api_key = "AIza_secret".to_string();
        new.post_process.enabled = true;
        new.engine = Engine::Polza;

        let summary = new.summary();
        let changes = new.describe_changes(&old).join("; ");
        for secret in ["gsk_secret", "aai_secret", "AIza_secret"] {
            assert!(!summary.contains(secret));
            assert!(!changes.contains(secret));
        }
        assert!(changes.contains("assemblyaiApiKey updated (set)"));
        assert!(changes.contains("groqApiKey updated (set)"));
        assert!(changes.contains("postProcess.gemini.apiKey updated (set)"));
        assert!(changes.contains("postProcess.enabled: 'false' → 'true'"));
        assert!(changes.contains("engine: 'groq' → 'polza'"));
    }

    #[test]
    fn test_official_preset_migrated_to_proper() {
        let dir = temp_dir().join("typr_test_preset");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("config.json"),
            r#"{"microphone":"default","postProcess":{"preset":"official"}}"#,
        )
        .unwrap();

        let loaded = Settings::load(&dir);
        assert_eq!(loaded.post_process.preset, Preset::Proper);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_empty_provider_id_is_not_saved() {
        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(!json.contains("providerId"));
    }

    #[test]
    fn test_post_process_key_falls_back_to_engine_key() {
        let mut settings = Settings::default();
        settings.groq_api_key = "gsk_engine".to_string();
        assert_eq!(settings.post_process_key(PostProvider::Groq), "gsk_engine");
        settings.post_process.groq.api_key = "gsk_own".to_string();
        assert_eq!(settings.post_process_key(PostProvider::Groq), "gsk_own");
        assert_eq!(settings.post_process_key(PostProvider::Gemini), "");
    }

    #[test]
    fn test_language_hint() {
        let mut settings = Settings::default();
        assert_eq!(settings.language_hint(), None);
        settings.recognition_language = "ru".to_string();
        assert_eq!(settings.language_hint(), Some("ru"));
    }

    #[test]
    fn test_local_changes_are_logged() {
        let old = Settings::default();
        let mut new = Settings::default();
        new.local_backend = "vulkan".to_string();
        new.local_unload = "never".to_string();
        let changes = new.describe_changes(&old).join("; ");
        assert!(changes.contains("localBackend: '' → 'vulkan'"));
        assert!(changes.contains("localUnload: '5m' → 'never'"));
        assert!(new.summary().contains("localBackend=vulkan"));
    }

    #[test]
    fn test_save_and_load() {
        let dir = temp_dir().join("typr_test_settings");
        let _ = fs::remove_dir_all(&dir);

        let mut settings = Settings::default();
        settings.engine = Engine::OpenAi;
        settings.openai_api_key = "test-key-123".to_string();
        settings.openai_model = "whisper-custom".to_string();
        settings.openai_endpoint = "https://custom.ai/v1".to_string();
        settings.recognition_language = "en".to_string();

        settings.save(&dir).unwrap();
        let loaded = Settings::load(&dir);
        assert_eq!(loaded, settings);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_missing_file_returns_default() {
        let dir = temp_dir().join("typr_test_missing");
        let _ = fs::remove_dir_all(&dir);
        let settings = Settings::load(&dir);
        assert_eq!(settings, Settings::default());
        assert!(!settings.setup_done);
    }

    #[test]
    fn test_load_corrupt_json_returns_default() {
        let dir = temp_dir().join("typr_test_corrupt");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "not json").unwrap();

        let settings = Settings::load(&dir);
        assert_eq!(settings, Settings::default());

        let _ = fs::remove_dir_all(&dir);
    }
}
