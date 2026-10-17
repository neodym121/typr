use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
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
    RecordingMode (default PushToTalk) {
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

string_enum! {
    /// Language the dictation is translated into (Post-processing →
    /// Translation); names and detection in translate.rs.
    TranslationLanguage (default English) {
        English = "en",
        Russian = "ru",
        Ukrainian = "uk",
        Spanish = "es",
        French = "fr",
        German = "de",
        Chinese = "zh",
        Japanese = "ja",
        Hindi = "hi",
        Hebrew = "he",
        Persian = "fa",
    }
}

/// Languages offered for recognition; empty means detect automatically.
pub const RECOGNITION_LANGUAGES: &[&str] = &["ru", "en", "uk", "de", "fr", "es", "it", "pt", "pl"];

fn default_true() -> bool {
    true
}

/// The API key of every service, in one place (the API keys section). The
/// other sections only choose a service and a model.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ApiKeys {
    pub groq: String,
    /// OpenAI, or the OpenAI-compatible server set in Engine
    pub openai: String,
    pub polza: String,
    pub assemblyai: String,
    pub gemini: String,
    pub openrouter: String,
    /// Web search for quick ask
    pub exa: String,
}

impl ApiKeys {
    /// Every key with its name in config.json
    pub fn all(&self) -> [(&'static str, &str); 7] {
        [
            ("groq", &self.groq),
            ("openai", &self.openai),
            ("polza", &self.polza),
            ("assemblyai", &self.assemblyai),
            ("gemini", &self.gemini),
            ("openrouter", &self.openrouter),
            ("exa", &self.exa),
        ]
    }

    /// The key of a language model provider (post-processing, quick ask).
    pub fn model(&self, provider: PostProvider) -> &str {
        match provider {
            PostProvider::Gemini => self.gemini.trim(),
            PostProvider::OpenRouter => self.openrouter.trim(),
            PostProvider::Groq => self.groq.trim(),
            PostProvider::Polza => self.polza.trim(),
        }
    }

    /// The key of a cloud engine; the local one has none.
    pub fn engine(&self, engine: Engine) -> &str {
        match engine {
            Engine::Groq => self.groq.trim(),
            Engine::OpenAi => self.openai.trim(),
            Engine::Polza => self.polza.trim(),
            Engine::AssemblyAi => self.assemblyai.trim(),
            Engine::Local => "",
        }
    }

    pub fn exa(&self) -> &str {
        self.exa.trim()
    }
}

/// Where the keys were kept before the API keys section, by their new name.
/// The first one set becomes the shared key.
const OLD_KEY_PLACES: &[(&str, &[&[&str]])] = &[
    ("groq", &[&["groqApiKey"], &["postProcess", "groq", "apiKey"], &["quickAsk", "groq", "apiKey"]]),
    ("openai", &[&["openaiApiKey"]]),
    ("polza", &[&["polzaApiKey"], &["postProcess", "polza", "apiKey"], &["quickAsk", "polza", "apiKey"]]),
    ("assemblyai", &[&["assemblyaiApiKey"]]),
    ("gemini", &[&["postProcess", "gemini", "apiKey"], &["quickAsk", "gemini", "apiKey"]]),
    ("openrouter", &[&["postProcess", "openrouter", "apiKey"], &["quickAsk", "openrouter", "apiKey"]]),
    ("exa", &[&["quickAsk", "exaApiKey"]]),
];

/// Takes the string at `path` out of the config, if it is there.
fn take_string(root: &mut Map<String, Value>, path: &[&str]) -> Option<String> {
    let (last, parents) = path.split_last()?;
    let mut object = root;
    for name in parents {
        object = object.get_mut(*name)?.as_object_mut()?;
    }
    match object.remove(*last)? {
        Value::String(text) => Some(text),
        _ => None,
    }
}

/// Moves the keys of a config from before the API keys section (each engine
/// and section had its own) into `apiKeys`. A key already there stays.
fn migrate_keys(config: &mut Value) {
    let Some(root) = config.as_object_mut() else {
        return;
    };
    let mut keys = root
        .get("apiKeys")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut moved = Vec::new();
    for (name, places) in OLD_KEY_PLACES {
        let found: Vec<String> = places
            .iter()
            .filter_map(|path| take_string(root, path))
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .collect();
        let Some(first) = found.first() else {
            continue;
        };
        let current = keys.get(*name).and_then(Value::as_str).unwrap_or("").trim().to_string();
        let kept = if current.is_empty() {
            keys.insert(name.to_string(), Value::String(first.clone()));
            moved.push(*name);
            first.clone()
        } else {
            current
        };
        if found.iter().any(|key| *key != kept) {
            log::warn!(
                "The settings had different {} keys in different sections; API keys keeps one of them",
                name
            );
        }
    }
    if !moved.is_empty() {
        log::info!("API keys moved to the API keys section: {}", moved.join(", "));
    }
    root.insert("apiKeys".to_string(), Value::Object(keys));
}

/// The model chosen with one language model provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProviderModel {
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
    /// The same model translates the dictation into `translate_to`, with
    /// post-processing off too; a dictation already in that language isn't sent
    pub translate: bool,
    #[serde(rename = "translateTo")]
    pub translate_to: TranslationLanguage,
    pub gemini: ProviderModel,
    pub openrouter: ProviderModel,
    pub groq: ProviderModel,
    pub polza: ProviderModel,
}

impl PostProcess {
    /// Whether dictations go to the model at all (edited, translated or both)
    pub fn uses_model(&self) -> bool {
        self.enabled || self.translate
    }

    pub fn provider_settings(&self, provider: PostProvider) -> &ProviderModel {
        match provider {
            PostProvider::Gemini => &self.gemini,
            PostProvider::OpenRouter => &self.openrouter,
            PostProvider::Groq => &self.groq,
            PostProvider::Polza => &self.polza,
        }
    }

    fn providers(&self) -> impl Iterator<Item = (PostProvider, &ProviderModel)> {
        PostProvider::ALL
            .iter()
            .map(move |&provider| (provider, self.provider_settings(provider)))
    }
}

/// Quick ask: with text selected anywhere, the hotkey records a question
/// about it instead of a dictation, and a language model answers in a small
/// window that grows out of the recording indicator. While that window is
/// open, the hotkey asks follow-up questions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct QuickAsk {
    pub enabled: bool,
    /// Quick chat: `Settings::chat_hotkey` opens the same window as a chat
    /// with the same model, the questions shown too
    pub chat: bool,
    pub provider: PostProvider,
    pub gemini: ProviderModel,
    pub openrouter: ProviderModel,
    pub groq: ProviderModel,
    pub polza: ProviderModel,
    /// The model may search the web (Exa) when a question needs it
    #[serde(rename = "webSearch")]
    pub web_search: bool,
}

impl QuickAsk {
    pub fn provider_settings(&self, provider: PostProvider) -> &ProviderModel {
        match provider {
            PostProvider::Gemini => &self.gemini,
            PostProvider::OpenRouter => &self.openrouter,
            PostProvider::Groq => &self.groq,
            PostProvider::Polza => &self.polza,
        }
    }

    fn providers(&self) -> impl Iterator<Item = (PostProvider, &ProviderModel)> {
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
    #[serde(rename = "apiKeys")]
    pub api_keys: ApiKeys,
    /// "whisper-large-v3-turbo" or "whisper-large-v3"
    #[serde(rename = "groqModel")]
    pub groq_model: String,
    #[serde(rename = "openaiEndpoint")]
    pub openai_endpoint: String,
    #[serde(rename = "openaiModel")]
    pub openai_model: String,
    #[serde(rename = "polzaModel")]
    pub polza_model: String,
    #[serde(rename = "polzaProvider")]
    pub polza_provider: String,
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
    /// Opens and closes quick chat: keys, two or three modifiers alone
    /// (e.g. "Ctrl+Alt") or a mouse button
    #[serde(rename = "chatHotkey")]
    pub chat_hotkey: String,
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
    #[serde(rename = "quickAsk")]
    pub quick_ask: QuickAsk,
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
    /// Developer section: a hidden quick ask window stays loaded, so quick ask
    /// and quick chat open faster; WebView2 then keeps running while idle
    #[serde(rename = "keepAskWindow")]
    pub keep_ask_window: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            microphone: "default".to_string(),
            engine: Engine::default(),
            api_keys: ApiKeys::default(),
            groq_model: "whisper-large-v3-turbo".to_string(),
            openai_endpoint: "https://api.openai.com/v1".to_string(),
            openai_model: "whisper-1".to_string(),
            polza_model: "openai/whisper-large-v3".to_string(),
            polza_provider: String::new(),
            assemblyai_model: "universal-3-5-pro".to_string(),
            recognition_language: String::new(),
            fallback_local: true,
            local_model: crate::local::catalog::DEFAULT_MODEL.to_string(),
            local_backend: String::new(),
            local_unload: "5m".to_string(),
            recording_mode: RecordingMode::default(),
            hotkey: "Ctrl+Shift+Space".to_string(),
            chat_hotkey: "Ctrl+Alt".to_string(),
            append_space: false,
            show_indicator: true,
            indicator_position: IndicatorPosition::default(),
            language: String::new(),
            post_process: PostProcess::default(),
            quick_ask: QuickAsk::default(),
            check_updates: true,
            setup_done: false,
            developer_mode: false,
            keep_ask_window: false,
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
            Ok(contents) => Self::from_json(&contents),
            Err(_) => Self::default(),
        }
    }

    /// Reads config.json, moving the keys of an older one into `apiKeys`.
    /// A file that isn't JSON gives the defaults.
    pub fn from_json(contents: &str) -> Self {
        match serde_json::from_str::<Value>(contents) {
            Ok(mut config) => {
                migrate_keys(&mut config);
                serde_json::from_value(config).unwrap_or_default()
            }
            Err(_) => Self::default(),
        }
    }

    /// The recognition language as an API expects it: `None` detects it.
    pub fn language_hint(&self) -> Option<&str> {
        let code = self.recognition_language.trim();
        (!code.is_empty()).then_some(code)
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
            "engine={}, mode={}, hotkey={}, chatHotkey={}, appendSpace={}, mic='{}', recognitionLanguage={}, fallbackLocal={}, apiKeys={}, groqModel={}, openaiEndpoint={}, openaiModel={}, polzaModel={}, polzaProvider={}, assemblyaiModel={}, localModel={}, localBackend={}, localUnload={}, showIndicator={}, indicatorPosition={}, language={}, postProcess={}, quickAsk={}, checkUpdates={}, developerMode={}, keepAskWindow={}",
            self.engine,
            self.recording_mode,
            self.hotkey,
            self.chat_hotkey,
            self.append_space,
            self.microphone,
            or_auto(&self.recognition_language),
            self.fallback_local,
            self.keys_summary(),
            self.groq_model,
            self.openai_endpoint,
            self.openai_model,
            self.polza_model,
            or_auto(&self.polza_provider),
            self.assemblyai_model,
            self.local_model,
            or_auto(&self.local_backend),
            self.local_unload,
            self.show_indicator,
            self.indicator_position,
            if self.language.is_empty() { "system" } else { self.language.as_str() },
            self.post_process_summary(),
            self.quick_ask_summary(),
            self.check_updates,
            self.developer_mode,
            self.keep_ask_window
        )
    }

    /// The services that have a key, e.g. "groq+gemini"
    fn keys_summary(&self) -> String {
        let set: Vec<&str> = self
            .api_keys
            .all()
            .iter()
            .filter(|(_, key)| !key.trim().is_empty())
            .map(|(name, _)| *name)
            .collect();
        if set.is_empty() {
            "none".to_string()
        } else {
            set.join("+")
        }
    }

    fn post_process_summary(&self) -> String {
        let pp = &self.post_process;
        let model = pp.provider_settings(pp.provider).model.as_str();
        format!(
            "{} (provider {}, model {}, preset {}, translate {}, key {})",
            if pp.enabled { "on" } else { "off" },
            pp.provider,
            if model.is_empty() { "none" } else { model },
            pp.preset,
            if pp.translate { format!("to {}", pp.translate_to) } else { "off".to_string() },
            key_state(self.api_keys.model(pp.provider))
        )
    }

    fn quick_ask_summary(&self) -> String {
        let qa = &self.quick_ask;
        let model = qa.provider_settings(qa.provider).model.as_str();
        format!(
            "{} (chat {}, provider {}, model {}, key {}, web search {}, Exa key {})",
            if qa.enabled { "on" } else { "off" },
            if qa.chat { "on" } else { "off" },
            qa.provider,
            if model.is_empty() { "none" } else { model },
            key_state(self.api_keys.model(qa.provider)),
            if qa.web_search { "on" } else { "off" },
            key_state(self.api_keys.exa())
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
        changed("chatHotkey", &old.chat_hotkey, &self.chat_hotkey);
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
        changed("keepAskWindow", &old.keep_ask_window, &self.keep_ask_window);

        let pp_old = &old.post_process;
        let pp_new = &self.post_process;
        changed("postProcess.enabled", &pp_old.enabled, &pp_new.enabled);
        changed("postProcess.provider", &pp_old.provider, &pp_new.provider);
        changed("postProcess.preset", &pp_old.preset, &pp_new.preset);
        changed("postProcess.translate", &pp_old.translate, &pp_new.translate);
        changed("postProcess.translateTo", &pp_old.translate_to, &pp_new.translate_to);
        for ((provider, before), (_, after)) in pp_old.providers().zip(pp_new.providers()) {
            changed(&format!("postProcess.{}.model", provider), &before.model, &after.model);
            changed(
                &format!("postProcess.{}.providerId", provider),
                &before.provider_id,
                &after.provider_id,
            );
        }

        let qa_old = &old.quick_ask;
        let qa_new = &self.quick_ask;
        changed("quickAsk.enabled", &qa_old.enabled, &qa_new.enabled);
        changed("quickAsk.chat", &qa_old.chat, &qa_new.chat);
        changed("quickAsk.provider", &qa_old.provider, &qa_new.provider);
        changed("quickAsk.webSearch", &qa_old.web_search, &qa_new.web_search);
        for ((provider, before), (_, after)) in qa_old.providers().zip(qa_new.providers()) {
            changed(&format!("quickAsk.{}.model", provider), &before.model, &after.model);
            changed(
                &format!("quickAsk.{}.providerId", provider),
                &before.provider_id,
                &after.provider_id,
            );
        }

        for ((name, before), (_, after)) in old.api_keys.all().into_iter().zip(self.api_keys.all()) {
            if before != after {
                changes.push(format!("apiKeys.{} updated ({})", name, key_state(after)));
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
        assert_eq!(settings.api_keys, ApiKeys::default());
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
        assert_eq!(settings.recording_mode, RecordingMode::PushToTalk);
        assert!(!settings.post_process.translate);
        assert_eq!(settings.post_process.translate_to, TranslationLanguage::English);
        assert_eq!(settings.hotkey, "Ctrl+Shift+Space");
        assert_eq!(settings.chat_hotkey, "Ctrl+Alt");
        assert!(settings.show_indicator);
        assert_eq!(settings.indicator_position, IndicatorPosition::TopRight);
        assert!(settings.check_updates);
        assert!(!settings.setup_done);
        assert!(!settings.developer_mode);
        assert!(!settings.keep_ask_window);
    }

    #[test]
    fn test_missing_fields_get_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"engine":"local"}"#).unwrap();
        assert_eq!(settings.engine, Engine::Local);
        assert_eq!(settings.microphone, "default");
        assert_eq!(settings.local_model, "parakeet-tdt-0.6b-v3");
        assert_eq!(settings.local_unload, "5m");
        assert_eq!(settings.post_process, PostProcess::default());
        assert_eq!(settings.quick_ask, QuickAsk::default());
        assert!(settings.show_indicator);
        assert_eq!(settings.indicator_position, IndicatorPosition::TopRight);
        assert!(settings.fallback_local);
        assert!(settings.check_updates);
        assert!(!settings.developer_mode);
        assert!(!settings.keep_ask_window);
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
        assert_eq!(RecordingMode::from("toggle".to_string()), RecordingMode::Toggle);
        for &position in IndicatorPosition::ALL {
            assert_eq!(IndicatorPosition::parse(position.id()), Some(position));
        }
        for &language in TranslationLanguage::ALL {
            assert_eq!(TranslationLanguage::parse(language.id()), Some(language));
        }
        assert_eq!(TranslationLanguage::from("xx".to_string()), TranslationLanguage::English);

        let settings: Settings = serde_json::from_str(
            r#"{"engine":"nonsense","indicatorPosition":"bottom-center","postProcess":{"provider":"openrouter"}}"#,
        )
        .unwrap();
        assert_eq!(settings.engine, Engine::Groq);
        assert_eq!(settings.indicator_position, IndicatorPosition::BottomCenter);
        assert_eq!(settings.post_process.provider, PostProvider::OpenRouter);

        let json = serde_json::to_string(&Settings::default()).unwrap();
        assert!(json.contains(r#""engine":"groq""#));
        assert!(json.contains(r#""recordingMode":"push-to-talk""#));
        assert!(json.contains(r#""indicatorPosition":"top-right""#));
        assert!(json.contains(r#""translate":false,"translateTo":"en""#));
    }

    #[test]
    fn test_translation_reads_from_the_file_and_is_logged() {
        let settings: Settings =
            serde_json::from_str(r#"{"postProcess":{"translate":true,"translateTo":"es"}}"#).unwrap();
        assert!(settings.post_process.translate);
        assert_eq!(settings.post_process.translate_to, TranslationLanguage::Spanish);
        assert!(!settings.post_process.enabled);
        assert!(settings.post_process.uses_model());
        assert!(!Settings::default().post_process.uses_model());

        let changes = settings.describe_changes(&Settings::default()).join("; ");
        assert!(changes.contains("postProcess.translate: 'false' → 'true'"));
        assert!(changes.contains("postProcess.translateTo: 'en' → 'es'"));
        assert!(settings.summary().contains("translate to es"));
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
        new.api_keys.groq = "gsk_secret".to_string();
        new.api_keys.assemblyai = "aai_secret".to_string();
        new.api_keys.gemini = "AIza_secret".to_string();
        new.post_process.enabled = true;
        new.engine = Engine::Polza;

        let summary = new.summary();
        let changes = new.describe_changes(&old).join("; ");
        for secret in ["gsk_secret", "aai_secret", "AIza_secret"] {
            assert!(!summary.contains(secret));
            assert!(!changes.contains(secret));
        }
        assert!(changes.contains("apiKeys.assemblyai updated (set)"));
        assert!(changes.contains("apiKeys.groq updated (set)"));
        assert!(changes.contains("apiKeys.gemini updated (set)"));
        assert!(summary.contains("apiKeys=groq+assemblyai+gemini"));
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
    fn test_one_key_per_service() {
        let mut settings = Settings::default();
        settings.api_keys.groq = " gsk_key ".to_string();
        settings.api_keys.gemini = "AIza_key".to_string();
        assert_eq!(settings.api_keys.engine(Engine::Groq), "gsk_key");
        assert_eq!(settings.api_keys.model(PostProvider::Groq), "gsk_key");
        assert_eq!(settings.api_keys.model(PostProvider::Gemini), "AIza_key");
        assert_eq!(settings.api_keys.model(PostProvider::Polza), "");
        assert_eq!(settings.api_keys.engine(Engine::Local), "");
    }

    #[test]
    fn test_old_keys_move_to_api_keys() {
        let old = r#"{
            "engine": "groq",
            "groqApiKey": "gsk_engine",
            "openaiApiKey": "sk-openai",
            "postProcess": {"provider": "gemini", "gemini": {"apiKey": "AIza_post", "model": "gemini-2.5-flash"}, "groq": {"apiKey": "gsk_post"}},
            "quickAsk": {"enabled": true, "openrouter": {"apiKey": "or_ask", "model": "m"}, "polza": {"apiKey": "pk_ask"}, "exaApiKey": "exa_key"}
        }"#;
        let settings = Settings::from_json(old);
        assert_eq!(settings.api_keys.groq, "gsk_engine");
        assert_eq!(settings.api_keys.openai, "sk-openai");
        assert_eq!(settings.api_keys.gemini, "AIza_post");
        assert_eq!(settings.api_keys.openrouter, "or_ask");
        assert_eq!(settings.api_keys.polza, "pk_ask");
        assert_eq!(settings.api_keys.exa, "exa_key");
        assert_eq!(settings.api_keys.assemblyai, "");
        // The models stay where they were
        assert_eq!(settings.post_process.gemini.model, "gemini-2.5-flash");
        assert_eq!(settings.quick_ask.openrouter.model, "m");
        assert!(settings.quick_ask.enabled);

        let saved = serde_json::to_string(&settings).unwrap();
        for old_field in [r#""groqApiKey""#, r#""apiKey""#, r#""exaApiKey""#] {
            assert!(!saved.contains(old_field), "{} is still saved", old_field);
        }
        assert_eq!(Settings::from_json(&saved), settings);
    }

    #[test]
    fn test_a_key_in_api_keys_wins_over_an_old_one() {
        let settings = Settings::from_json(r#"{"apiKeys": {"groq": "gsk_new"}, "groqApiKey": "gsk_old"}"#);
        assert_eq!(settings.api_keys.groq, "gsk_new");
    }

    #[test]
    fn test_quick_ask_is_off_by_default_and_reads_from_the_file() {
        let settings = Settings::default();
        assert!(!settings.quick_ask.enabled);
        assert_eq!(settings.quick_ask.provider, PostProvider::Gemini);

        let settings: Settings = serde_json::from_str(
            r#"{"quickAsk":{"enabled":true,"chat":true,"provider":"openrouter","openrouter":{"model":"m"}}}"#,
        )
        .unwrap();
        assert!(settings.quick_ask.enabled);
        assert!(settings.quick_ask.chat);
        assert_eq!(settings.quick_ask.provider, PostProvider::OpenRouter);
        assert_eq!(settings.quick_ask.openrouter.model, "m");
        assert!(serde_json::to_string(&settings).unwrap().contains(r#""quickAsk":{"enabled":true"#));
    }

    #[test]
    fn test_quick_ask_changes_are_logged_without_keys() {
        let old = Settings::default();
        let mut new = Settings::default();
        new.quick_ask.enabled = true;
        new.quick_ask.provider = PostProvider::Polza;
        new.quick_ask.chat = true;
        new.chat_hotkey = "Ctrl+Shift".to_string();
        new.quick_ask.polza.model = "openai/gpt-5-mini".to_string();
        new.api_keys.polza = "pk_secret".to_string();
        new.quick_ask.web_search = true;
        new.api_keys.exa = "exa_secret".to_string();

        let changes = new.describe_changes(&old).join("; ");
        assert!(changes.contains("quickAsk.enabled: 'false' → 'true'"));
        assert!(changes.contains("quickAsk.provider: 'gemini' → 'polza'"));
        assert!(changes.contains("quickAsk.polza.model: '' → 'openai/gpt-5-mini'"));
        assert!(changes.contains("quickAsk.chat: 'false' → 'true'"));
        assert!(changes.contains("chatHotkey: 'Ctrl+Alt' → 'Ctrl+Shift'"));
        assert!(changes.contains("apiKeys.polza updated (set)"));
        assert!(changes.contains("quickAsk.webSearch: 'false' → 'true'"));
        assert!(changes.contains("apiKeys.exa updated (set)"));
        let summary = new.summary();
        assert!(summary.contains(
            "quickAsk=on (chat on, provider polza, model openai/gpt-5-mini, key set, web search on, Exa key set)"
        ));
        for secret in ["pk_secret", "exa_secret"] {
            assert!(!changes.contains(secret));
            assert!(!summary.contains(secret));
        }
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
        settings.api_keys.openai = "test-key-123".to_string();
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
