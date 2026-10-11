//! Общие словари и текущий язык сообщений — для всех крейтов порта.
//!
//! Словари — те же файлы, что читает интерфейс (`crates/vmixrtc/ui/locales/{ru,en}.json`):
//! один источник строк и для Rust, и для JavaScript. Ключ → строка, подстановки `{0}`, `{1}`, …
//! как `string.Format` в оригинале.
//!
//! Язык один на процесс: приложение выставляет его при старте и при смене в настройках
//! (`set_language`), библиотечные крейты только читают — через макрос [`tr!`]:
//!
//! ```
//! use vmix_i18n::tr;
//! let message = tr!("msg.vmc.noWidget", 7);
//! assert!(message.contains('7'));
//! ```
//!
//! Пока язык не выбран, действует первый из [`LANGUAGES`] — русский, как и раньше.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Поддерживаемые языки. Первый — язык по умолчанию.
pub const LANGUAGES: [&str; 2] = ["ru", "en"];

const SOURCES: [&str; 2] = [
    include_str!("../../vmixrtc/ui/locales/ru.json"),
    include_str!("../../vmixrtc/ui/locales/en.json"),
];

/// Индекс текущего языка в [`LANGUAGES`].
static CURRENT: AtomicUsize = AtomicUsize::new(0);

fn dictionaries() -> &'static [BTreeMap<String, String>; 2] {
    static DICTIONARIES: OnceLock<[BTreeMap<String, String>; 2]> = OnceLock::new();
    DICTIONARIES.get_or_init(|| SOURCES.map(|json| serde_json::from_str(json).unwrap_or_default()))
}

/// Номер языка: `en`, `en-US`, `EN_us` → английский.
fn position(language: &str) -> Option<usize> {
    let language = language.trim().to_ascii_lowercase();
    let code = language.split(['-', '_', '.']).next().unwrap_or_default();
    LANGUAGES.iter().position(|known| *known == code)
}

/// Выбрать язык сообщений для всего процесса. Возвращает `false`, если язык не поддержан.
pub fn set_language(language: &str) -> bool {
    match position(language) {
        Some(index) => {
            CURRENT.store(index, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// Текущий язык сообщений.
pub fn language() -> &'static str {
    LANGUAGES[CURRENT.load(Ordering::Relaxed)]
}

/// Словарь языка целиком — для интерфейса и проверок паритета.
pub fn dictionary(language: &str) -> Option<&'static BTreeMap<String, String>> {
    position(language).map(|index| &dictionaries()[index])
}

/// Перевод на указанный язык. Нет ключа — берётся язык по умолчанию, нет и там —
/// возвращается сам ключ (его видно в интерфейсе и тестах).
pub fn text_in(language: &str, key: &str) -> String {
    let index = position(language).unwrap_or(0);
    dictionaries()[index]
        .get(key)
        .or_else(|| dictionaries()[0].get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// Перевод на указанный язык с подстановкой `{0}`, `{1}`, …
pub fn format_in(language: &str, key: &str, args: &[&dyn Display]) -> String {
    let mut text = text_in(language, key);
    for (position, value) in args.iter().enumerate() {
        text = text.replace(&format!("{{{position}}}"), &value.to_string());
    }
    text
}

/// Перевод на текущий язык.
pub fn text(key: &str) -> String {
    text_in(language(), key)
}

/// Перевод на текущий язык с подстановкой `{0}`, `{1}`, …
pub fn format(key: &str, args: &[&dyn Display]) -> String {
    format_in(language(), key, args)
}

/// Сообщение на текущем языке: `tr!("ключ")` или `tr!("ключ", значение, …)`.
#[macro_export]
macro_rules! tr {
    ($key:expr) => {
        $crate::text($key)
    };
    ($key:expr, $($arg:expr),+ $(,)?) => {
        $crate::format($key, &[$(&$arg as &dyn ::std::fmt::Display),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn dictionaries_are_loaded_for_both_languages() {
        for language in LANGUAGES {
            let dictionary = dictionary(language).expect("словарь");
            assert!(dictionary.len() > 100, "{language}: {}", dictionary.len());
        }
        assert!(dictionary("de").is_none());
    }

    #[test]
    fn placeholders_are_substituted_in_each_language() {
        assert_eq!(
            format_in("ru", "msg.vmc.noWidget", &[&7]),
            "виджета с индексом 7 нет"
        );
        assert_eq!(
            format_in("en", "msg.vmc.noWidget", &[&7]),
            "there is no widget with index 7"
        );
        assert_eq!(format_in("en-US", "msg.log.ok", &[&"Cut"]), "Cut → ok");
    }

    #[test]
    fn missing_key_returns_the_key() {
        assert_eq!(text_in("en", "no.such.key"), "no.such.key");
    }

    #[test]
    fn default_language_is_the_first_one() {
        // язык процесса тесты не меняют: от него зависят сообщения в соседних тестах
        assert_eq!(language(), LANGUAGES[0]);
    }

    fn rust_sources(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_sources(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }

    /// Каждый ключ, который код передаёт в `tr!`, есть в обоих словарях.
    #[test]
    fn every_key_used_in_code_is_translated() {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates");
        let mut sources = Vec::new();
        rust_sources(crates, &mut sources);
        assert!(sources.len() > 10, "исходники не найдены: {}", crates.display());

        let marker = "tr!(\"";
        let mut used = 0;
        for path in sources {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            for (start, _) in text.match_indices(marker) {
                // `include_str!("…")` тоже кончается на `tr!("` — это не наш макрос
                let before = text[..start].chars().next_back();
                if before.is_some_and(|symbol| symbol.is_alphanumeric() || symbol == '_') {
                    continue;
                }
                let rest = &text[start + marker.len()..];
                let Some(end) = rest.find('"') else { continue };
                let key = &rest[..end];
                // пример из документации к макросу, а не ключ
                if !key.is_ascii() {
                    continue;
                }
                used += 1;
                for language in LANGUAGES {
                    assert!(
                        dictionary(language).is_some_and(|dictionary| dictionary.contains_key(key)),
                        "{}: ключа «{key}» нет в словаре {language}",
                        path.display()
                    );
                }
            }
        }
        assert!(used > 50, "ключей в коде найдено подозрительно мало: {used}");
    }
}
