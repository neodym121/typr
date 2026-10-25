//! Interface language. English by default; Russian when the system language
//! is Russian or one whose speakers usually read Russian more easily than
//! English. The choice in General overrides the system.

use crate::settings::Engine;

/// ISO 639-1 codes that get the Russian interface by default: Russian,
/// Ukrainian, Belarusian and languages of other CIS countries where Russian
/// is widely spoken.
const RUSSIAN_DEFAULT: &[&str] = &["ru", "uk", "be", "kk", "ky", "tg", "uz", "tk", "hy", "az"];

/// "ru" or "en" for a locale such as `ru-RU`, `uk_UA` or `en-US`.
pub fn language_for_locale(locale: &str) -> &'static str {
    let primary = locale
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if RUSSIAN_DEFAULT.contains(&primary.as_str()) {
        "ru"
    } else {
        "en"
    }
}

/// Interface language that suits the system's display language.
pub fn system_language() -> &'static str {
    sys_locale::get_locale()
        .map(|locale| language_for_locale(&locale))
        .unwrap_or("en")
}

/// `setting` is the saved choice: "en", "ru", or empty to follow the system.
pub fn resolve(setting: &str) -> &'static str {
    match setting {
        "en" => "en",
        "ru" => "ru",
        _ => system_language(),
    }
}

pub fn tray_hotkey(language: &str, enabled: bool) -> &'static str {
    match (language, enabled) {
        ("ru", true) => "Хоткей: вкл",
        ("ru", false) => "Хоткей: выкл",
        (_, true) => "Hotkey: On",
        (_, false) => "Hotkey: Off",
    }
}

pub fn tray_exit(language: &str) -> &'static str {
    if language == "ru" {
        "Выход"
    } else {
        "Exit"
    }
}

pub fn tray_open(language: &str) -> &'static str {
    if language == "ru" {
        "Открыть настройки"
    } else {
        "Open settings"
    }
}

pub fn tray_paste_last(language: &str) -> &'static str {
    if language == "ru" {
        "Вставить последнее"
    } else {
        "Paste last dictation"
    }
}

pub fn tray_engine(language: &str) -> &'static str {
    if language == "ru" {
        "Распознавание"
    } else {
        "Engine"
    }
}

/// Name of an engine in the tray menu
pub fn engine_label(language: &str, engine: Engine) -> &'static str {
    match (language, engine) {
        ("ru", Engine::Local) => "Локально",
        (_, engine) => engine.label(),
    }
}

pub fn tray_update(language: &str, version: &str) -> String {
    if language == "ru" {
        format!("Доступна версия {}", version)
    } else {
        format!("Version {} is available", version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_for_locale() {
        assert_eq!(language_for_locale("ru-RU"), "ru");
        assert_eq!(language_for_locale("uk_UA"), "ru");
        assert_eq!(language_for_locale("be"), "ru");
        assert_eq!(language_for_locale("kk-KZ"), "ru");
        assert_eq!(language_for_locale("en-US"), "en");
        assert_eq!(language_for_locale("de-DE"), "en");
        assert_eq!(language_for_locale(""), "en");
    }

    #[test]
    fn test_saved_choice_wins() {
        assert_eq!(resolve("ru"), "ru");
        assert_eq!(resolve("en"), "en");
    }
}
