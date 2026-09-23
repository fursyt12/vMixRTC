//! Локализация: словари лежат рядом с интерфейсом и служат **одним источником** и для Rust,
//! и для JavaScript (UI читает те же файлы).
//!
//! Соответствие оригиналу: в C# строки берутся из `LocalizationManager` по ключам `loc:Loc`,
//! язык хранится в настройках приложения. Здесь так же: ключ → строка, язык — в файле настроек
//! платформы (Linux — `XDG_CONFIG_HOME`, macOS — `Application Support`, Windows — `APPDATA`).

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Поддерживаемые языки. Первый — язык по умолчанию.
pub const LANGUAGES: [&str; 2] = ["ru", "en"];

const RU: &str = include_str!("../ui/locales/ru.json");
const EN: &str = include_str!("../ui/locales/en.json");

/// Активный язык и словари.
pub struct I18n {
    language: String,
    dictionaries: BTreeMap<String, BTreeMap<String, String>>,
}

impl I18n {
    /// Загрузить словари и определить язык: настройки → переменные окружения → `ru`.
    pub fn load() -> Self {
        let language = read_settings()
            .or_else(|| std::env::var("VMIXRTC_LANG").ok())
            .filter(|language| LANGUAGES.contains(&language.as_str()))
            .unwrap_or_else(detect_from_env);
        Self::for_language(language)
    }

    /// Словари без чтения настроек — используется как `Default` (например, в тестах).
    pub fn builtin() -> Self {
        Self::for_language(detect_from_env())
    }

    /// Словари с явно заданным языком: тесты и интерфейс не зависят от локали машины.
    pub fn for_language(language: impl Into<String>) -> Self {
        let language = language.into();
        let language = if LANGUAGES.contains(&language.as_str()) {
            language
        } else {
            "ru".to_string()
        };
        Self::with_language(language)
    }

    fn with_language(language: String) -> Self {
        let mut dictionaries = BTreeMap::new();
        dictionaries.insert("ru".to_string(), parse(RU));
        dictionaries.insert("en".to_string(), parse(EN));
        Self {
            language,
            dictionaries,
        }
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// Сменить язык и сохранить выбор. Возвращает `false`, если язык не поддержан.
    pub fn set_language(&mut self, language: &str) -> bool {
        let language = language.trim().to_ascii_lowercase();
        let language = language.split(['-', '_']).next().unwrap_or("").to_string();
        if !LANGUAGES.contains(&language.as_str()) {
            return false;
        }
        self.language = language.clone();
        let _ = write_settings(&language);
        true
    }

    /// Перевод по ключу. Если ключа нет — возвращается сам ключ (видно в интерфейсе и тестах).
    pub fn t(&self, key: &str) -> String {
        self.dictionaries
            .get(&self.language)
            .and_then(|dictionary| dictionary.get(key))
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    /// Перевод с подстановкой `{0}`, `{1}`, … — как `string.Format` в оригинале.
    pub fn tf(&self, key: &str, args: &[&str]) -> String {
        let mut text = self.t(key);
        for (position, value) in args.iter().enumerate() {
            text = text.replace(&format!("{{{position}}}"), value);
        }
        text
    }

    /// Все ключи словаря — для проверок паритета и будущего редактора словарей.
    #[allow(dead_code)]
    pub fn keys(&self, language: &str) -> Vec<String> {
        self.dictionaries
            .get(language)
            .map(|dictionary| dictionary.keys().cloned().collect())
            .unwrap_or_default()
    }
}

impl Default for I18n {
    fn default() -> Self {
        Self::builtin()
    }
}

fn parse(json: &str) -> BTreeMap<String, String> {
    serde_json::from_str(json).unwrap_or_default()
}

/// Язык из переменных окружения (`LANG`, `LC_ALL`, `LANGUAGE`).
fn detect_from_env() -> String {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.to_ascii_lowercase();
            if value.starts_with("en") {
                return "en".to_string();
            }
            if value.starts_with("ru") {
                return "ru".to_string();
            }
        }
    }
    "ru".to_string()
}

/// Каталог настроек платформы.
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(app_data) = std::env::var("APPDATA") {
            return PathBuf::from(app_data).join("vmixrtc");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("vmixrtc");
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("vmixrtc");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("vmixrtc");
    }
    PathBuf::from(".vmixutc")
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

fn read_settings() -> Option<String> {
    let text = std::fs::read_to_string(settings_path()).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("language")
        .and_then(|language| language.as_str())
        .map(str::to_string)
}

fn write_settings(language: &str) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("каталог настроек {}", dir.display()))?;
    let payload = serde_json::json!({ "language": language });
    std::fs::write(settings_path(), format!("{payload:#}\n"))
        .with_context(|| format!("запись {}", settings_path().display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionaries_have_the_same_keys() {
        let i18n = I18n::load();
        let ru = i18n.keys("ru");
        let en = i18n.keys("en");
        assert!(ru.len() > 100, "словарь подозрительно маленький: {}", ru.len());
        assert_eq!(
            ru, en,
            "наборы ключей ru и en должны совпадать (нет перевода — нет ключа)"
        );
    }

    #[test]
    fn translations_have_no_empty_values() {
        let i18n = I18n::load();
        for language in LANGUAGES {
            for key in i18n.keys(language) {
                let value = i18n
                    .dictionaries
                    .get(language)
                    .and_then(|dictionary| dictionary.get(&key))
                    .cloned()
                    .unwrap_or_default();
                assert!(
                    !value.trim().is_empty(),
                    "пустой перевод {language}:{key}"
                );
            }
        }
    }

    #[test]
    fn placeholders_match_between_languages() {
        let i18n = I18n::load();
        for key in i18n.keys("ru") {
            let strip = |language: &str| {
                i18n.dictionaries
                    .get(language)
                    .and_then(|dictionary| dictionary.get(&key))
                    .map(|value| {
                        let mut text = value.clone();
                        for position in 0..4 {
                            text = text.replace(&format!("{{{position}}}"), "");
                        }
                        text
                    })
                    .unwrap_or_default()
            };
            let ru = i18n
                .dictionaries
                .get("ru")
                .and_then(|dictionary| dictionary.get(&key))
                .cloned()
                .unwrap_or_default();
            let en = i18n
                .dictionaries
                .get("en")
                .and_then(|dictionary| dictionary.get(&key))
                .cloned()
                .unwrap_or_default();
            let placeholders = |text: &str| {
                (0..4)
                    .filter(|position| text.contains(&format!("{{{position}}}")))
                    .count()
            };
            assert_eq!(
                placeholders(&ru),
                placeholders(&en),
                "разное число подстановок в {key}: «{ru}» / «{en}»"
            );
            let _ = strip;
        }
    }

    /// Собрать ключи из исходников интерфейса: `t("ключ")` и `data-i18n*="ключ"`.
    fn ui_keys() -> Vec<String> {
        let mut keys = Vec::new();
        let js = include_str!("../ui/app.js");
        for marker in ["t(\"", "t('"] {
            let mut rest = js;
            while let Some(position) = rest.find(marker) {
                // «t(» должен быть вызовом функции, а не хвостом слова:
                // closest(".picker") — это не ключ локализации
                let boundary = rest[..position]
                    .chars()
                    .next_back()
                    .map(|symbol| {
                        !symbol.is_alphanumeric() && symbol != '_' && symbol != '$' && symbol != '.'
                    })
                    .unwrap_or(true);
                rest = &rest[position + marker.len()..];
                if !boundary {
                    continue;
                }
                let quote = marker.chars().last().unwrap();
                if let Some(end) = rest.find(quote) {
                    let key = &rest[..end];
                    if !key.is_empty() && key.contains('.') && !key.contains(' ') {
                        keys.push(key.to_string());
                    }
                    rest = &rest[end..];
                } else {
                    break;
                }
            }
        }
        let html = include_str!("../ui/index.html");
        for marker in [
            "data-i18n=\"",
            "data-i18n-title=\"",
            "data-i18n-placeholder=\"",
        ] {
            let mut rest = html;
            while let Some(position) = rest.find(marker) {
                rest = &rest[position + marker.len()..];
                if let Some(end) = rest.find('"') {
                    keys.push(rest[..end].to_string());
                    rest = &rest[end..];
                } else {
                    break;
                }
            }
        }
        keys.sort();
        keys.dedup();
        keys
    }

    #[test]
    fn every_ui_key_is_translated() {
        let i18n = I18n::load();
        let ru = i18n.keys("ru");
        let missing: Vec<String> = ui_keys()
            .into_iter()
            .filter(|key| !ru.contains(key))
            .collect();
        assert!(
            missing.is_empty(),
            "в словарях нет ключей, которые использует интерфейс: {missing:?}"
        );
    }

    #[test]
    fn widget_kinds_and_providers_have_translations() {
        let i18n = I18n::load();
        for kind in vmix_config::WidgetKind::PALETTE {
            let key = kind.localization_key();
            assert_ne!(
                i18n.t(key),
                key,
                "нет перевода для вида виджета: {key}"
            );
        }
        for key in [
            "provider.xml",
            "provider.json",
            "provider.excel",
            "provider.sheets",
            "provider.ndi",
            "provider.files",
            "provider.unsupported",
        ] {
            assert_ne!(i18n.t(key), key, "нет перевода провайдера: {key}");
        }
    }

    #[test]
    fn missing_key_returns_the_key() {
        let mut i18n = I18n::load();
        assert_eq!(i18n.t("нет.такого.ключа"), "нет.такого.ключа");
        assert!(i18n.set_language("en"));
        assert_eq!(i18n.t("toolbar.save"), "Save");
        assert!(!i18n.set_language("de"), "неподдержанный язык отклоняется");
        assert_eq!(i18n.language(), "en");
        // подстановка как string.Format
        let i18n = I18n::load();
        assert!(i18n.tf("toolbar.connected", &["29.0"]).contains("29.0"));
    }
}
