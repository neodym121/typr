//! History: the latest dictations with what speech recognition heard and
//! what post-processing made of it, kept in history.json in the config
//! folder. The oldest entries go once the limit (Settings → historyLimit)
//! is reached. Entries made from an uploaded file keep its name.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

use crate::settings::{Engine, Settings};

/// The choices History offers for how many entries it keeps
pub const LIMITS: &[usize] = &[5, 10, 20];
pub const DEFAULT_LIMIT: usize = 20;
/// The folder of the recordings, next to history.json
pub const AUDIO_DIR: &str = "history-audio";
pub const CHANGED_EVENT: &str = "history-changed";

/// The nearest choice at or above `limit`.
pub fn normalize_limit(limit: usize) -> usize {
    LIMITS
        .iter()
        .copied()
        .find(|&allowed| limit <= allowed)
        .unwrap_or(DEFAULT_LIMIT)
}

/// A recording to keep beside its entry.
pub struct Audio {
    pub bytes: Vec<u8>,
    /// "wav", "mp3", … — no dot
    pub extension: String,
}

/// One dictation, or one uploaded file, as History shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: u64,
    /// When the recording was made, milliseconds since the epoch
    pub created_at: u64,
    /// Length of the recording in seconds; 0 when it isn't known
    pub duration: f32,
    /// What speech recognition heard
    pub text: String,
    /// What post-processing made of it, when it ran
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed: Option<String>,
    /// The model that transcribed it, e.g. "Groq · whisper-large-v3-turbo"
    pub model: String,
    /// The uploaded file, when the entry came from one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The recording in the audio folder, e.g. "12.wav"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
}

impl Entry {
    /// A dictation; `id` and `audio` are given by `add`.
    pub fn dictation(duration: f32, text: String, processed: Option<String>, model: String) -> Self {
        Self { id: 0, created_at: now(), duration, text, processed, model, file: None, audio: None }
    }

    /// A transcription of the uploaded file `file`.
    pub fn uploaded(file: String, duration: f32, text: String, model: String) -> Self {
        Self { id: 0, created_at: now(), duration, text, processed: None, model, file: Some(file), audio: None }
    }
}

/// Milliseconds since the epoch
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// The speech recognition model of the settings, as History names it.
pub fn model_label(settings: &Settings) -> String {
    let named = |model: &str, default: &str| {
        let model = model.trim();
        if model.is_empty() { default.to_string() } else { model.to_string() }
    };
    match settings.engine {
        Engine::Local => crate::local::catalog::model(&settings.local_model)
            .map(|spec| spec.name.to_string())
            .unwrap_or_else(|| named(&settings.local_model, crate::local::catalog::DEFAULT_MODEL)),
        Engine::Groq => format!("Groq · {}", named(&settings.groq_model, "whisper-large-v3-turbo")),
        Engine::AssemblyAi => format!("AssemblyAI · {}", named(&settings.assemblyai_model, "universal-3-5-pro")),
        Engine::Polza => format!("Polza · {}", named(&settings.polza_model, "openai/whisper-large-v3")),
    }
}

static DIR: OnceLock<PathBuf> = OnceLock::new();
static ENTRIES: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

fn lock() -> MutexGuard<'static, Vec<Entry>> {
    ENTRIES.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn file_in(dir: &Path) -> PathBuf {
    dir.join("history.json")
}

/// Reads history.json from `dir`, oldest first. A missing or broken file is
/// an empty history.
pub fn load(dir: &Path) -> Vec<Entry> {
    let Ok(json) = fs::read_to_string(file_in(dir)) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = serde_json::from_str(&json).unwrap_or_else(|e| {
        log::warn!("history.json can't be read, the history starts empty: {}", e);
        Vec::new()
    });
    entries.sort_by_key(|entry| entry.created_at);
    entries
}

fn save(dir: &Path, entries: &[Entry]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(entries)
        .map_err(|e| format!("History can't be written as JSON: {}", e))?;
    let file = file_in(dir);
    fs::create_dir_all(dir)
        .and_then(|()| fs::write(&file, json))
        .map_err(|e| format!("Failed to write {}: {}", file.display(), e))
}

/// Keeps the newest `limit` entries and removes the recordings of the rest.
fn keep_newest(entries: &mut Vec<Entry>, limit: usize, dir: Option<&Path>) {
    let limit = normalize_limit(limit);
    if entries.len() <= limit {
        return;
    }
    let dropped: Vec<Entry> = entries.drain(..entries.len() - limit).collect();
    if let Some(dir) = dir {
        for entry in &dropped {
            remove_audio(dir, entry);
        }
    }
}

/// Keeps the newest `limit` entries.
pub fn trim(entries: &mut Vec<Entry>, limit: usize) {
    keep_newest(entries, limit, None);
}

fn audio_folder(dir: &Path) -> PathBuf {
    dir.join(AUDIO_DIR)
}

fn remove_audio(dir: &Path, entry: &Entry) {
    let Some(name) = &entry.audio else {
        return;
    };
    let path = audio_folder(dir).join(name);
    if let Err(e) = fs::remove_file(&path) {
        log::debug!("Failed to remove {}: {}", path.display(), e);
    }
}

/// Writes the recording of `id` and answers its file name.
fn write_audio(dir: &Path, id: u64, audio: &Audio) -> Option<String> {
    let folder = audio_folder(dir);
    let name = format!("{}.{}", id, audio.extension);
    if let Err(e) = fs::create_dir_all(&folder).and_then(|()| fs::write(folder.join(&name), &audio.bytes)) {
        log::error!("Failed to save the recording of history entry {}: {}", id, e);
        return None;
    }
    Some(name)
}

/// Reads the history of `dir` and drops what is over `limit`. Called once at
/// start, before anything can be added.
pub fn init(dir: PathBuf, limit: usize) {
    let mut entries = load(&dir);
    let before = entries.len();
    keep_newest(&mut entries, limit, Some(&dir));
    log::info!(
        "History: {} entries in {}{}",
        entries.len(),
        file_in(&dir).display(),
        if entries.len() == before { "" } else { ", the oldest ones were over the limit" }
    );
    if before != entries.len() {
        if let Err(e) = save(&dir, &entries) {
            log::warn!("{}", e);
        }
    }
    let _ = DIR.set(dir);
    *lock() = entries;
}

/// The history as it is now, oldest first.
pub fn entries() -> Vec<Entry> {
    lock().clone()
}

/// The recording of the entry `id`, if it has one.
pub fn audio(id: u64) -> Result<Option<Vec<u8>>, String> {
    let Some(dir) = DIR.get() else {
        return Ok(None);
    };
    let Some(name) = lock()
        .iter()
        .find(|entry| entry.id == id)
        .and_then(|entry| entry.audio.clone())
    else {
        return Ok(None);
    };
    let path = audio_folder(dir).join(&name);
    match fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) => Err(format!("Failed to read {}: {}", path.display(), e)),
    }
}

/// Saves the entries to history.json, if the folder is known.
fn persist(entries: &[Entry]) {
    let Some(dir) = DIR.get() else {
        return;
    };
    if let Err(e) = save(dir, entries) {
        log::error!("{}", e);
    }
}

fn emit(app: &AppHandle, entries: &[Entry]) {
    if let Err(e) = app.emit(CHANGED_EVENT, entries) {
        log::debug!("Failed to emit {}: {}", CHANGED_EVENT, e);
    }
}

/// Adds an entry with its recording, drops the oldest ones over `limit` and
/// tells the window.
pub fn add(app: &AppHandle, mut entry: Entry, audio: Option<Audio>, limit: usize) -> Vec<Entry> {
    let entries = {
        let mut entries = lock();
        entry.id = entries.iter().map(|entry| entry.id).max().unwrap_or(0) + 1;
        if let (Some(dir), Some(audio)) = (DIR.get(), audio) {
            entry.audio = write_audio(dir, entry.id, &audio);
        }
        entries.push(entry);
        keep_newest(&mut entries, limit, DIR.get().map(PathBuf::as_path));
        persist(&entries);
        entries.clone()
    };
    emit(app, &entries);
    entries
}

/// Forgets the entry `id` and its recording, and tells the window.
pub fn delete(app: &AppHandle, id: u64) -> Vec<Entry> {
    let entries = {
        let mut entries = lock();
        if let Some(dir) = DIR.get() {
            if let Some(entry) = entries.iter().find(|entry| entry.id == id) {
                remove_audio(dir, entry);
            }
        }
        entries.retain(|entry| entry.id != id);
        persist(&entries);
        entries.clone()
    };
    emit(app, &entries);
    entries
}

/// Puts the post-processed text of the entry `id` in place; `None` when the
/// entry is gone.
pub fn set_processed(app: &AppHandle, id: u64, processed: String) -> Option<Vec<Entry>> {
    let entries = {
        let mut entries = lock();
        let entry = entries.iter_mut().find(|entry| entry.id == id)?;
        entry.processed = Some(processed);
        persist(&entries);
        entries.clone()
    };
    emit(app, &entries);
    Some(entries)
}

/// The limit changed: drops what is over it, with their recordings, and
/// tells the window.
pub fn set_limit(app: &AppHandle, limit: usize) -> Vec<Entry> {
    let entries = {
        let mut entries = lock();
        keep_newest(&mut entries, limit, DIR.get().map(PathBuf::as_path));
        persist(&entries);
        entries.clone()
    };
    emit(app, &entries);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("typr-history-{}-{}", test, std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Scratch(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn entry(at: u64, text: &str) -> Entry {
        Entry {
            id: at,
            created_at: at,
            duration: 1.5,
            text: text.to_string(),
            processed: None,
            model: "Groq · whisper-large-v3-turbo".to_string(),
            file: None,
            audio: None,
        }
    }

    #[test]
    fn test_trim_keeps_the_newest() {
        let mut entries: Vec<Entry> = (1..=10).map(|n| entry(n, "text")).collect();
        trim(&mut entries, 5);
        assert_eq!(entries.first().unwrap().created_at, 6);
        assert_eq!(entries.last().unwrap().created_at, 10);
        // Out of the choices: the nearest one
        trim(&mut entries, 1);
        assert_eq!(entries.len(), 5);
        let mut many: Vec<Entry> = (1..=40).map(|n| entry(n, "text")).collect();
        trim(&mut many, 100);
        assert_eq!(many.len(), 20);
    }

    #[test]
    fn test_the_limit_choices() {
        assert_eq!(normalize_limit(1), 5);
        assert_eq!(normalize_limit(5), 5);
        assert_eq!(normalize_limit(6), 10);
        assert_eq!(normalize_limit(10), 10);
        assert_eq!(normalize_limit(11), 20);
        assert_eq!(normalize_limit(30), 20);
    }

    #[test]
    fn test_recordings_are_written_and_removed_with_their_entry() {
        let scratch = Scratch::new("audio");
        fs::create_dir_all(&scratch.0).unwrap();
        let mut entry = entry(3, "text");
        let name = write_audio(&scratch.0, entry.id, &Audio { bytes: b"RIFF".to_vec(), extension: "wav".to_string() })
            .expect("the recording is written");
        entry.audio = Some(name.clone());
        assert_eq!(name, "3.wav");
        assert_eq!(fs::read(audio_folder(&scratch.0).join(&name)).unwrap(), b"RIFF");
        remove_audio(&scratch.0, &entry);
        assert!(!audio_folder(&scratch.0).join(&name).exists());
    }

    #[test]
    fn test_saved_history_reads_back() {
        let scratch = Scratch::new("round-trip");
        let mut first = entry(1000, "привет");
        first.processed = Some("Привет.".to_string());
        let mut second = entry(2000, "hello");
        second.file = Some("note.wav".to_string());
        second.duration = 0.0;
        let entries = vec![first, second];
        save(&scratch.0, &entries).unwrap();
        assert_eq!(load(&scratch.0), entries);
    }

    #[test]
    fn test_a_missing_or_broken_file_is_empty() {
        assert!(load(&Scratch::new("missing").0).is_empty());
        let scratch = Scratch::new("broken");
        fs::create_dir_all(&scratch.0).unwrap();
        fs::write(file_in(&scratch.0), "{ not json").unwrap();
        assert!(load(&scratch.0).is_empty());
    }

    #[test]
    fn test_the_file_is_plain_camel_case_json() {
        let scratch = Scratch::new("json");
        save(&scratch.0, &[entry(1, "text")]).unwrap();
        let json = fs::read_to_string(file_in(&scratch.0)).unwrap();
        assert!(json.contains(r#""createdAt": 1"#), "{}", json);
        assert!(json.contains(r#""model": "Groq · whisper-large-v3-turbo""#), "{}", json);
    }
}
