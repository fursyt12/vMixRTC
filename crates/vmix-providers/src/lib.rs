//! Провайдеры данных.
//!
//! В оригинале интерфейс `IvMixDataProvider` смешивает данные и WPF-UI:
//! `ShowProperties(Window owner)`, `UIElement CustomUI`, `GetProperties()/SetProperties()`.
//! В порте UI-часть вынесена в слой интерфейса, а здесь остаётся только то, что
//! относится к данным: период обновления, значения и сохранение свойств.
//!
//! Соответствие (`vMixControllerDataProvider/IvMixDataProvider.cs`):
//! * `Period`          → [`DataProvider::period_ms`]
//! * `Values`          → [`DataProvider::keys`] + [`DataProvider::value`]
//! * `GetProperties()` → [`DataProvider::save_properties`]
//! * `SetProperties()` → [`DataProvider::load_properties`]
//! * `ShowProperties`/`CustomUI` — уходят в UI-слой (порт окна свойств).

mod external;

pub use external::{
    column_index, fetch_bytes, fetch_bytes_with_headers, jsonpath_select, rows_from_xml,
    sheet_key_from, sheet_rows, ExcelDataProvider, GoogleSheetsDataProvider, JsonDataProvider,
    NdiDataProvider, ProviderKind, XmlDataProvider,
};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub trait DataProvider: Send {
    /// Имя для UI и для сохранения в `.vmc`.
    fn name(&self) -> &str;

    /// Период обновления в миллисекундах (`Period` в оригинале).
    fn period_ms(&self) -> u64;

    /// Ключи текущего набора данных — то, из чего UI строит «выбор дата-сета».
    fn keys(&self) -> Vec<String>;

    fn value(&self, key: &str) -> Option<String>;

    /// Перечитать источник. Вызывается планировщиком раз в `period_ms`.
    fn refresh(&mut self) -> Result<()>;

    /// Сериализация свойств для `.vmc`.
    fn save_properties(&self) -> Vec<String>;

    fn load_properties(&mut self, properties: &[String]) -> Result<()>;
}

// ------------------------------------------------------------- JSON

/// Чтение JSON-файла: скалярные листья раскрываются в ключи с путём через точку
/// (`scores.home`). Массивы дают ключи с индексом (`scores.0`).
#[derive(Debug)]
pub struct JsonFileProvider {
    path: PathBuf,
    period_ms: u64,
    data: BTreeMap<String, String>,
}

impl JsonFileProvider {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut provider = Self {
            path: path.as_ref().to_path_buf(),
            period_ms: 1000,
            data: BTreeMap::new(),
        };
        provider.refresh()?;
        Ok(provider)
    }

    pub fn set_period_ms(&mut self, period_ms: u64) {
        self.period_ms = period_ms;
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn flatten(prefix: &str, value: &Value, out: &mut BTreeMap<String, String>) {
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    Self::flatten(&path, value, out);
                }
            }
            Value::Array(items) => {
                for (index, value) in items.iter().enumerate() {
                    let path = if prefix.is_empty() {
                        index.to_string()
                    } else {
                        format!("{prefix}.{index}")
                    };
                    Self::flatten(&path, value, out);
                }
            }
            Value::Null => {
                out.insert(prefix.to_string(), String::new());
            }
            Value::Bool(flag) => {
                out.insert(prefix.to_string(), flag.to_string());
            }
            Value::Number(number) => {
                out.insert(prefix.to_string(), number.to_string());
            }
            Value::String(text) => {
                out.insert(prefix.to_string(), text.clone());
            }
        }
    }
}

impl DataProvider for JsonFileProvider {
    fn name(&self) -> &str {
        "JsonDataProvider"
    }

    fn period_ms(&self) -> u64 {
        self.period_ms
    }

    fn keys(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    fn value(&self, key: &str) -> Option<String> {
        self.data.get(key).cloned()
    }

    fn refresh(&mut self) -> Result<()> {
        let text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("чтение JSON {}", self.path.display()))?;
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("разбор JSON {}", self.path.display()))?;
        let mut data = BTreeMap::new();
        Self::flatten("", &value, &mut data);
        self.data = data;
        Ok(())
    }

    fn save_properties(&self) -> Vec<String> {
        vec![self.path.display().to_string(), self.period_ms.to_string()]
    }

    fn load_properties(&mut self, properties: &[String]) -> Result<()> {
        if let Some(path) = properties.first() {
            self.path = PathBuf::from(path);
        }
        if let Some(period) = properties.get(1) {
            self.period_ms = period
                .parse()
                .map_err(|_| anyhow!("некорректный период: {period}"))?;
        }
        self.refresh()
    }
}

// ------------------------------------------------------------- файловая система

/// Список файлов каталога (порт `FileSystemDataProvider`).
#[derive(Debug)]
pub struct FileSystemProvider {
    dir: PathBuf,
    period_ms: u64,
    data: BTreeMap<String, String>,
}

impl FileSystemProvider {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let mut provider = Self {
            dir: dir.as_ref().to_path_buf(),
            period_ms: 1000,
            data: BTreeMap::new(),
        };
        provider.refresh()?;
        Ok(provider)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl DataProvider for FileSystemProvider {
    fn name(&self) -> &str {
        "FileSystemDataProvider"
    }

    fn period_ms(&self) -> u64 {
        self.period_ms
    }

    fn keys(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    fn value(&self, key: &str) -> Option<String> {
        self.data.get(key).cloned()
    }

    fn refresh(&mut self) -> Result<()> {
        let mut data = BTreeMap::new();
        let entries = std::fs::read_dir(&self.dir)
            .with_context(|| format!("чтение каталога {}", self.dir.display()))?;
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let name = entry.file_name().to_string_lossy().into_owned();
                data.insert(name, entry.path().display().to_string());
            }
        }
        self.data = data;
        Ok(())
    }

    fn save_properties(&self) -> Vec<String> {
        vec![self.dir.display().to_string(), self.period_ms.to_string()]
    }

    fn load_properties(&mut self, properties: &[String]) -> Result<()> {
        if let Some(dir) = properties.first() {
            self.dir = PathBuf::from(dir);
        }
        if let Some(period) = properties.get(1) {
            self.period_ms = period
                .parse()
                .map_err(|_| anyhow!("некорректный период: {period}"))?;
        }
        self.refresh()
    }
}

// ------------------------------------------------------------- XML

/// Чтение XML-файла (порт `XmlDataProvider`, который отдаёт значения по путям).
/// Ключи — пути элементов от корня (`/root/scores/home`) и атрибуты (`/root/input/@key`),
/// что позволяет позже натянуть выборку по XPath без смены модели данных.
#[derive(Debug)]
pub struct XmlFileProvider {
    path: PathBuf,
    period_ms: u64,
    data: BTreeMap<String, String>,
}

impl XmlFileProvider {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut provider = Self {
            path: path.as_ref().to_path_buf(),
            period_ms: 1000,
            data: BTreeMap::new(),
        };
        provider.refresh()?;
        Ok(provider)
    }

    pub fn set_period_ms(&mut self, period_ms: u64) {
        self.period_ms = period_ms;
    }

    fn flatten(prefix: &str, node: &vmix_xml::Node, out: &mut BTreeMap<String, String>) {
        for (key, value) in &node.attrs {
            if key.starts_with("xmlns") {
                continue;
            }
            out.insert(format!("{prefix}/@{key}"), value.clone());
        }
        if !node.text.trim().is_empty() {
            out.insert(prefix.to_string(), node.text.clone());
        }
        for child in &node.children {
            Self::flatten(&format!("{prefix}/{}", child.name), child, out);
        }
    }
}

impl DataProvider for XmlFileProvider {
    fn name(&self) -> &str {
        "XmlDataProvider"
    }

    fn period_ms(&self) -> u64 {
        self.period_ms
    }

    fn keys(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    fn value(&self, key: &str) -> Option<String> {
        self.data.get(key).cloned()
    }

    fn refresh(&mut self) -> Result<()> {
        let bytes = std::fs::read(&self.path)
            .with_context(|| format!("чтение XML {}", self.path.display()))?;
        let root = vmix_xml::parse(&bytes)?;
        let mut data = BTreeMap::new();
        Self::flatten(&format!("/{}", root.name), &root, &mut data);
        self.data = data;
        Ok(())
    }

    fn save_properties(&self) -> Vec<String> {
        vec![self.path.display().to_string(), self.period_ms.to_string()]
    }

    fn load_properties(&mut self, properties: &[String]) -> Result<()> {
        if let Some(path) = properties.first() {
            self.path = PathBuf::from(path);
        }
        if let Some(period) = properties.get(1) {
            self.period_ms = period
                .parse()
                .map_err(|_| anyhow!("некорректный период: {period}"))?;
        }
        self.refresh()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vmix-providers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn json_provider_flattens_nested_values() {
        let dir = temp_dir("json");
        let file = dir.join("data.json");
        std::fs::write(
            &file,
            r#"{"scores":{"home":2,"away":[1,3]},"title":"Final","live":true}"#,
        )
        .unwrap();

        let provider = JsonFileProvider::open(&file).unwrap();
        assert_eq!(provider.value("scores.home").as_deref(), Some("2"));
        assert_eq!(provider.value("scores.away.1").as_deref(), Some("3"));
        assert_eq!(provider.value("title").as_deref(), Some("Final"));
        assert_eq!(provider.value("live").as_deref(), Some("true"));
        assert_eq!(provider.keys().len(), 5);

        // файл изменился — refresh подхватывает
        let mut provider = provider;
        std::fs::write(&file, r#"{"scores":{"home":5}}"#).unwrap();
        provider.refresh().unwrap();
        assert_eq!(provider.value("scores.home").as_deref(), Some("5"));
        assert!(provider.value("title").is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn json_provider_survives_round_trip_of_properties() {
        let dir = temp_dir("json-props");
        let file = dir.join("data.json");
        std::fs::write(&file, r#"{"a":1}"#).unwrap();

        let mut provider = JsonFileProvider::open(&file).unwrap();
        provider.set_period_ms(250);
        let saved = provider.save_properties();

        // свойства из .vmc применяются к уже открытому провайдеру
        let mut provider = JsonFileProvider::open(&file).unwrap();
        provider.load_properties(&saved).unwrap();
        assert_eq!(provider.period_ms(), 250);
        assert_eq!(provider.value("a").as_deref(), Some("1"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn xml_provider_exposes_element_paths_and_attributes() {
        let dir = temp_dir("xml");
        let file = dir.join("board.xml");
        std::fs::write(
            &file,
            r#"<board><scores home="2" away="3"/><title>Final</title></board>"#,
        )
        .unwrap();

        let provider = XmlFileProvider::open(&file).unwrap();
        assert_eq!(provider.value("/board/scores/@home").as_deref(), Some("2"));
        assert_eq!(provider.value("/board/scores/@away").as_deref(), Some("3"));
        assert_eq!(provider.value("/board/title").as_deref(), Some("Final"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn filesystem_provider_lists_files() {        let dir = temp_dir("fs");
        std::fs::write(dir.join("intro.mp4"), b"x").unwrap();
        std::fs::write(dir.join("outro.mp4"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();

        let provider = FileSystemProvider::open(&dir).unwrap();
        assert_eq!(provider.keys(), vec!["intro.mp4".to_string(), "outro.mp4".to_string()]);
        assert!(provider.value("intro.mp4").unwrap().ends_with("intro.mp4"));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
