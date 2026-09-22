//! Внешние данные для виджетов — порт провайдеров данных оригинального приложения.
//!
//! Важное отличие (осознанное): в `.vmc` провайдер хранится как **.NET-сборка в base64**
//! (`DataProviderContent`) плюс путь (`DataProviderPath`) и список свойств. Rust не может
//! исполнять .NET-сборки, поэтому порт игнорирует встроенную DLL (поле сохраняется в файле
//! как есть) и выбирает встроенный провайдер по `DataProviderPath`. Свойства при этом
//! читаются ровно те, что положил оригинал, — файлы совместимы в обе стороны.

use anyhow::{anyhow, Context, Result};
use calamine::{open_workbook_auto, Reader};
use serde_json::Value;
use vmix_functions::xpath_select;
use vmix_xml::Node;

/// Сколько строк максимум отдаёт провайдер (в оригинале `100 * groupBy` элементов).
const MAX_ROWS: usize = 100;

// --------------------------------------------------------------- вид провайдера

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderKind {
    Json,
    Xml,
    FileSystem,
    Excel,
    GoogleSheets,
    NdiMonitor,
    Unknown(String),
}

impl ProviderKind {
    /// Определить провайдер по `DataProviderPath` из `.vmc`.
    pub fn from_path(path: &str) -> Self {
        let name = path
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(path)
            .to_ascii_lowercase();
        match name.as_str() {
            "xmldataprovider.dll" => Self::Xml,
            "jsondataprovider.dll" => Self::Json,
            "filesystemdataprovider.dll" => Self::FileSystem,
            "exceldataprovider.dll" => Self::Excel,
            "googlesheetsprovider.dll" => Self::GoogleSheets,
            "ndimonitordataprovider.dll" => Self::NdiMonitor,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Json => "JSON",
            Self::Xml => "XML",
            Self::FileSystem => "Файлы",
            Self::Excel => "Excel",
            Self::GoogleSheets => "Google Sheets",
            Self::NdiMonitor => "NDI-монитор",
            Self::Unknown(name) => name,
        }
    }

    /// Поддержан ли провайдер в порте (остальные пока только распознаются).
    pub fn is_supported(&self) -> bool {
        matches!(
            self,
            Self::Json | Self::Xml | Self::FileSystem | Self::Excel | Self::GoogleSheets | Self::NdiMonitor
        )
    }
}

// --------------------------------------------------------------- загрузка источника

/// Прочитать данные источника: `http(s)://`, `file://` или обычный путь.
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    fetch_bytes_with_headers(url, &[])
}

/// То же, но с заголовками HTTP (свойство `Headers` у JSON-провайдера: строки «Имя: значение»).
pub fn fetch_bytes_with_headers(url: &str, headers: &[String]) -> Result<Vec<u8>> {
    let url = url.trim();
    if url.is_empty() {
        return Err(anyhow!("не задан источник данных"));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(5))
            .build();
        let mut request = agent.get(url);
        for header in headers {
            let header = header.trim();
            if header.is_empty() {
                continue;
            }
            if let Some((name, value)) = header.split_once(':') {
                request = request.set(name.trim(), value.trim());
            }
        }
        let response = request.call().with_context(|| format!("запрос {url}"))?;
        let mut buffer = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut buffer)
            .context("чтение ответа")?;
        return Ok(buffer);
    }
    let path = url.strip_prefix("file://").unwrap_or(url);
    std::fs::read(path).with_context(|| format!("чтение {path}"))
}

// --------------------------------------------------------------- XML-провайдер

/// Порт `vMixGenericXmlDataProvider/XmlDataProvider.cs`.
/// Свойства (порядок как в `.vmc`): `[url, xpath, namespaces, groupBy]`.
#[derive(Debug, Clone)]
pub struct XmlDataProvider {
    pub url: String,
    pub xpath: String,
    pub namespaces: String,
    pub group_by: usize,
    pub period_ms: u64,
    values: Vec<String>,
    error: Option<String>,
}

impl XmlDataProvider {
    pub fn from_properties(properties: &[String]) -> Self {
        Self {
            url: properties.first().cloned().unwrap_or_default(),
            xpath: properties.get(1).cloned().unwrap_or_default(),
            namespaces: properties.get(2).cloned().unwrap_or_default(),
            group_by: properties
                .get(3)
                .and_then(|value| value.trim().parse::<usize>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(1),
            period_ms: 1000,
            values: Vec::new(),
            error: None,
        }
    }

    pub fn set_period_ms(&mut self, period_ms: u64) {
        self.period_ms = period_ms;
    }

    pub fn values(&self) -> &[String] {
        &self.values
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Перечитать источник и пересобрать строки.
    pub fn refresh(&mut self) -> Result<()> {
        if self.url.trim().is_empty() || self.xpath.trim().is_empty() {
            self.values.clear();
            self.error = Some("не задан источник или XPath".into());
            return Ok(());
        }
        match fetch_bytes(&self.url).and_then(|bytes| vmix_xml::parse(&bytes)) {
            Ok(document) => {
                self.values = rows_from_xml(&document, &self.xpath, self.group_by);
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.values.clear();
                self.error = Some(format!("{error:#}"));
                Ok(())
            }
        }
    }
}

/// Собрать строки из XML: XPath-выражения разделяются `|`, результаты склеиваются
/// в строки по `group_by` колонок через `|` — как в оригинале
/// (`./vmix/inputs/input/@number|./vmix/inputs/input/@title` + `GroupBy=2` → `1|Colour Bars`).
pub fn rows_from_xml(document: &Node, xpath: &str, group_by: usize) -> Vec<String> {
    let group_by = group_by.max(1);
    let expressions: Vec<&str> = xpath
        .split('|')
        .map(str::trim)
        .filter(|expression| !expression.is_empty())
        .collect();
    if expressions.is_empty() {
        return Vec::new();
    }

    let columns: Vec<Vec<String>> = expressions
        .iter()
        .map(|expression| xpath_select(document, expression))
        .collect();

    // XPathEvaluate с объединением `|` возвращает узлы в порядке документа, поэтому
    // для выборок одинаковой длины (классика: `@number|@title`) значения чередуются
    // по строкам: 1, «Text…», 2, «Colour Bars», …
    let interleave = columns.len() > 1
        && columns
            .iter()
            .all(|column| column.len() == columns[0].len());
    let limit = MAX_ROWS * group_by;
    let mut cells: Vec<String> = Vec::new();
    if interleave {
        'rows: for row in 0..columns[0].len() {
            for column in &columns {
                cells.push(column[row].clone());
                if cells.len() >= limit {
                    break 'rows;
                }
            }
        }
    } else {
        for column in columns {
            for value in column {
                if cells.len() >= limit {
                    break;
                }
                cells.push(value);
            }
        }
    }

    cells
        .chunks(group_by)
        .filter(|chunk| chunk.iter().any(|cell| !cell.is_empty()))
        .map(|chunk| chunk.join("|"))
        .take(MAX_ROWS)
        .collect()
}

// --------------------------------------------------------------- JSON-провайдер

/// Порт `JsonDataProvider`: свойства `[url, jsonPath, groupBy, headers]`.
#[derive(Debug, Clone)]
pub struct JsonDataProvider {
    pub url: String,
    pub json_path: String,
    pub group_by: usize,
    pub headers: Vec<String>,
    pub period_ms: u64,
    values: Vec<String>,
    error: Option<String>,
}

impl JsonDataProvider {
    pub fn from_properties(properties: &[String]) -> Self {
        let headers = properties
            .get(3)
            .map(|value| {
                value
                    .split(['\r', '\n'])
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            url: properties.first().cloned().unwrap_or_default(),
            json_path: properties
                .get(1)
                .cloned()
                .unwrap_or_default()
                .replace(['\r', '\n'], ""),
            group_by: properties
                .get(2)
                .and_then(|value| value.trim().parse::<usize>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(1),
            headers,
            period_ms: 1000,
            values: Vec::new(),
            error: None,
        }
    }

    pub fn set_period_ms(&mut self, period_ms: u64) {
        self.period_ms = period_ms;
    }

    pub fn values(&self) -> &[String] {
        &self.values
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn refresh(&mut self) -> Result<()> {
        if self.url.trim().is_empty() || self.json_path.trim().is_empty() {
            self.values.clear();
            self.error = Some("не задан источник или JSONPath".into());
            return Ok(());
        }
        match fetch_bytes_with_headers(&self.url, &self.headers)
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).context("разбор JSON"))
        {
            Ok(document) => {
                let matches = jsonpath_select(&document, &self.json_path);
                self.values = group_rows(&matches, self.group_by);
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.values.clear();
                self.error = Some(format!("{error:#}"));
                Ok(())
            }
        }
    }
}

/// Склейка значений в строки по `groupBy` — как в оригинале (через `|`).
fn group_rows(values: &[String], group_by: usize) -> Vec<String> {
    let group_by = group_by.max(1);
    let limit = MAX_ROWS * group_by;
    let values: Vec<&String> = values.iter().take(limit).collect();
    if group_by == 1 {
        return values.into_iter().cloned().collect();
    }
    values
        .chunks(group_by)
        .map(|chunk| chunk.iter().map(|value| value.as_str()).collect::<Vec<_>>().join("|"))
        .collect()
}

/// Подмножество JSONPath: `$`, `.имя`, `['имя']`, `[номер]`, `[*]`, `..имя` (потомки).
/// Полный RFC 9535 порту не нужен — реальные источники описываются этими формами.
pub fn jsonpath_select(document: &Value, path: &str) -> Vec<String> {
    let path = path.trim();
    let mut current: Vec<&Value> = if path.starts_with('$') {
        vec![document]
    } else {
        vec![document]
    };
    let mut rest = path.trim_start_matches('$');
    let mut descendants = false;

    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("..") {
            descendants = true;
            rest = tail;
        }
        if let Some(tail) = rest.strip_prefix('.') {
            rest = tail;
        }

        // имя или ключ в скобках
        let (name, tail) = if let Some(after) = rest.strip_prefix('[') {
            match after.split_once(']') {
                Some((inner, tail)) => (inner.trim().to_string(), tail),
                None => break,
            }
        } else {
            let end = rest
                .find(|symbol: char| symbol == '.' || symbol == '[')
                .unwrap_or(rest.len());
            (rest[..end].to_string(), &rest[end..])
        };
        rest = tail;

        let unquoted = name.trim_matches(|symbol| symbol == '\'' || symbol == '"');
        let mut next: Vec<&Value> = Vec::new();
        for node in current {
            if unquoted == "*" {
                match node {
                    Value::Array(items) => next.extend(items.iter()),
                    Value::Object(map) => next.extend(map.values()),
                    _ => {}
                }
                continue;
            }
            if let Ok(index) = unquoted.parse::<usize>() {
                if let Value::Array(items) = node {
                    if let Some(item) = items.get(index) {
                        next.push(item);
                    }
                }
                continue;
            }
            if descendants {
                collect_descendants(node, unquoted, &mut next);
            } else if let Value::Object(map) = node {
                if let Some(value) = map.get(unquoted) {
                    next.push(value);
                }
            }
        }
        current = next;
        descendants = false;
        if current.is_empty() {
            break;
        }
    }

    current
        .into_iter()
        .map(|value| match value {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            Value::Bool(flag) => flag.to_string(),
            Value::Number(number) => number.to_string(),
            other => other.to_string(),
        })
        .collect()
}

fn collect_descendants<'a>(node: &'a Value, name: &str, out: &mut Vec<&'a Value>) {
    match node {
        Value::Object(map) => {
            for (key, value) in map {
                if key == name {
                    out.push(value);
                }
                collect_descendants(value, name, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_descendants(item, name, out);
            }
        }
        _ => {}
    }
}

// --------------------------------------------------------------- Excel-провайдер

/// Порт `ExcelDataProvider`: свойства `[filePath, startRow, endRow, startCol, endCol, sheetIndex, isTable]`.
#[derive(Debug, Clone)]
pub struct ExcelDataProvider {
    pub file_path: String,
    pub start_row: usize,
    pub end_row: i64,
    pub start_col: usize,
    pub end_col: i64,
    pub sheet: String,
    pub is_table: bool,
    pub period_ms: u64,
    values: Vec<String>,
    error: Option<String>,
}

impl ExcelDataProvider {
    pub fn from_properties(properties: &[String]) -> Self {
        let number = |index: usize, fallback: i64| {
            properties
                .get(index)
                .and_then(|value| value.trim().parse::<i64>().ok())
                .unwrap_or(fallback)
        };
        Self {
            file_path: properties.first().cloned().unwrap_or_default(),
            start_row: number(1, 0).max(0) as usize,
            end_row: number(2, -1),
            start_col: column_index(properties.get(3).map(String::as_str).unwrap_or("0")).unwrap_or(0),
            end_col: properties
                .get(4)
                .and_then(|value| column_index(value))
                .map(|value| value as i64)
                .unwrap_or(-1),
            sheet: properties.get(5).cloned().unwrap_or_default(),
            is_table: matches!(
                properties.get(6).map(String::as_str),
                Some("true") | Some("True") | Some("1")
            ),
            period_ms: 1000,
            values: Vec::new(),
            error: None,
        }
    }

    pub fn set_period_ms(&mut self, period_ms: u64) {
        self.period_ms = period_ms;
    }

    pub fn values(&self) -> &[String] {
        &self.values
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn refresh(&mut self) -> Result<()> {
        if self.file_path.trim().is_empty() {
            self.values.clear();
            self.error = Some("не задан файл".into());
            return Ok(());
        }
        match self.read_rows() {
            Ok(values) => {
                self.values = values;
                self.error = None;
            }
            Err(error) => {
                self.values.clear();
                self.error = Some(format!("{error:#}"));
            }
        }
        Ok(())
    }

    fn read_rows(&self) -> Result<Vec<String>> {
        let mut workbook = open_workbook_auto(&self.file_path)
            .with_context(|| format!("открытие {}", self.file_path))?;
        let names = workbook.sheet_names().to_vec();
        let range = if self.sheet.trim().is_empty() {
            workbook
                .worksheet_range_at(0)
                .ok_or_else(|| anyhow!("в книге нет листов"))?
        } else if let Ok(index) = self.sheet.trim().parse::<usize>() {
            workbook
                .worksheet_range_at(index)
                .ok_or_else(|| anyhow!("нет листа с индексом {index}"))?
        } else {
            workbook.worksheet_range(self.sheet.trim())
        }
        .map_err(|error| anyhow!("чтение листа: {error}"))?;

        let mut values = Vec::new();
        for (row_index, row) in range.rows().enumerate() {
            if row_index < self.start_row {
                continue;
            }
            if self.end_row >= 0 && row_index as i64 > self.end_row {
                break;
            }
            if values.len() >= MAX_ROWS {
                break;
            }

            let last = if self.end_col >= 0 {
                (self.end_col as usize).min(row.len().saturating_sub(1))
            } else {
                row.len().saturating_sub(1)
            };
            let mut line = String::new();
            let mut cells: Vec<String> = Vec::new();
            for column in self.start_col..=last {
                let Some(cell) = row.get(column) else {
                    continue;
                };
                // как `GetValue(i)?.ToString() ?? ""` в оригинале
                let text = cell.to_string();
                if self.is_table {
                    if !line.is_empty() {
                        line.push('|');
                    }
                    line.push_str(&text);
                } else {
                    cells.push(text);
                }
            }
            if self.is_table {
                if !line.is_empty() {
                    values.push(line);
                }
            } else if cells.iter().any(|cell| !cell.is_empty()) {
                values.extend(cells);
            }
        }
        let _ = names;
        Ok(values)
    }
}

// --------------------------------------------------------------- Google Sheets

/// Порт `GoogleSheetsDataProvider`: свойства
/// `[apiKey, startRow, endRow, startCol, endCol, sheetIndex, isTable, sheetKey, period]`.
/// Данные берутся из Google Sheets API v4 (`includeGridData=true`), как в `Popcron.Sheets`.
#[derive(Debug, Clone)]
pub struct GoogleSheetsDataProvider {
    pub api_key: String,
    pub start_row: usize,
    pub end_row: i64,
    pub start_col: usize,
    pub end_col: i64,
    pub sheet_index: usize,
    pub is_table: bool,
    pub sheet_key: String,
    pub period_ms: u64,
    /// База API — вынесена, чтобы тесты могли подставить локальный сервер.
    pub base_url: String,
    values: Vec<String>,
    error: Option<String>,
}

impl GoogleSheetsDataProvider {
    pub fn from_properties(properties: &[String]) -> Self {
        let number = |index: usize, fallback: i64| {
            properties
                .get(index)
                .and_then(|value| value.trim().parse::<i64>().ok())
                .unwrap_or(fallback)
        };
        Self {
            api_key: properties.first().cloned().unwrap_or_default(),
            start_row: number(1, 0).max(0) as usize,
            end_row: number(2, -1),
            start_col: number(3, 0).max(0) as usize,
            end_col: number(4, -1),
            sheet_index: number(5, 0).max(0) as usize,
            is_table: !matches!(
                properties.get(6).map(String::as_str),
                Some("false") | Some("False") | Some("0")
            ),
            sheet_key: properties.get(7).cloned().unwrap_or_default(),
            period_ms: number(8, 5000).max(0) as u64,
            base_url: "https://sheets.googleapis.com/v4".to_string(),
            values: Vec::new(),
            error: None,
        }
    }

    pub fn set_base_url(&mut self, base_url: impl Into<String>) {
        self.base_url = base_url.into();
    }

    pub fn values(&self) -> &[String] {
        &self.values
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Ссылка вида `https://docs.google.com/spreadsheets/d/<ключ>/edit` → ключ.
    pub fn spreadsheet_key(&self) -> String {
        sheet_key_from(&self.sheet_key)
    }

    pub fn refresh(&mut self) -> Result<()> {
        let key = self.spreadsheet_key();
        if key.is_empty() {
            self.values.clear();
            self.error = Some("не задан ключ таблицы".into());
            return Ok(());
        }
        let mut url = format!(
            "{}/spreadsheets/{}?includeGridData=true",
            self.base_url.trim_end_matches('/'),
            key
        );
        if !self.api_key.trim().is_empty() {
            url.push_str(&format!("&key={}", self.api_key.trim()));
        }

        match fetch_bytes(&url)
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).context("разбор ответа Sheets"))
        {
            Ok(document) => {
                self.values = sheet_rows(&document, self.sheet_index, self);
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.values.clear();
                self.error = Some(format!("{error:#}"));
                Ok(())
            }
        }
    }
}

/// Ключ таблицы из ссылки или «как есть».
pub fn sheet_key_from(text: &str) -> String {
    let text = text.trim();
    if let Some(rest) = text.split("/d/").nth(1) {
        return rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .to_string();
    }
    text.to_string()
}

/// Разобрать `sheets[].data[].rowData[].values[]` в строки/ячейки.
pub fn sheet_rows(
    document: &Value,
    sheet_index: usize,
    provider: &GoogleSheetsDataProvider,
) -> Vec<String> {
    let Some(sheet) = document
        .get("sheets")
        .and_then(|sheets| sheets.get(sheet_index))
    else {
        return Vec::new();
    };
    let Some(data) = sheet.get("data").and_then(|data| data.get(0)) else {
        return Vec::new();
    };
    let Some(rows) = data.get("rowData").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut values = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        if row_index < provider.start_row {
            continue;
        }
        if provider.end_row >= 0 && row_index as i64 > provider.end_row {
            break;
        }
        if values.len() >= MAX_ROWS {
            break;
        }
        let cells = row
            .get("values")
            .and_then(Value::as_array)
            .map(|cells| cells.as_slice())
            .unwrap_or(&[]);
        let last = if provider.end_col >= 0 {
            (provider.end_col as usize).min(cells.len().saturating_sub(1))
        } else {
            cells.len().saturating_sub(1)
        };

        let mut line = String::new();
        let mut collected: Vec<String> = Vec::new();
        for cell in cells.iter().take(last + 1).skip(provider.start_col) {
            let text = cell
                .get("formattedValue")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    cell.get("effectiveValue")
                        .map(|value| match value {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default()
                });
            if provider.is_table {
                if !line.is_empty() {
                    line.push('|');
                }
                line.push_str(&text);
            } else {
                collected.push(text);
            }
        }
        if provider.is_table {
            if !line.is_empty() {
                values.push(line);
            }
        } else if collected.iter().any(|cell| !cell.is_empty()) {
            values.extend(collected);
        }
    }
    values
}

// --------------------------------------------------------------- NDI-монитор

/// Порт `vMixUTCNDIMonitorDataProvider`: свойства
/// `[sourceName, null, multiViewLayout, aspectRatio, isAudioEnabled, isLowBandwidth]`.
///
/// Провайдер отдаёт список NDI-источников. Оригинал берёт их из NDI SDK (finder) и умеет
/// принимать видео; порт читает NDI-входы из состояния vMix (`type="NDI"`), поэтому работает
/// без установленного NDI runtime. Приём видео и mDNS-обнаружение — следующий шаг.
#[derive(Debug, Clone)]
pub struct NdiDataProvider {
    pub source_name: String,
    pub multi_view_layout: i64,
    pub aspect_ratio: String,
    pub audio_enabled: bool,
    pub low_bandwidth: bool,
    pub period_ms: u64,
    values: Vec<String>,
    error: Option<String>,
}

impl NdiDataProvider {
    pub fn from_properties(properties: &[String]) -> Self {
        let number = |index: usize, fallback: i64| {
            properties
                .get(index)
                .and_then(|value| value.trim().parse::<i64>().ok())
                .unwrap_or(fallback)
        };
        let flag = |index: usize, fallback: bool| match properties.get(index).map(String::as_str) {
            Some("true") | Some("True") | Some("1") => true,
            Some("false") | Some("False") | Some("0") => false,
            _ => fallback,
        };
        Self {
            source_name: properties.first().cloned().unwrap_or_default(),
            multi_view_layout: number(2, 0),
            aspect_ratio: properties.get(3).cloned().unwrap_or_default(),
            audio_enabled: flag(4, true),
            low_bandwidth: flag(5, false),
            period_ms: 1000,
            values: Vec::new(),
            error: None,
        }
    }

    pub fn set_period_ms(&mut self, period_ms: u64) {
        self.period_ms = period_ms;
    }

    pub fn values(&self) -> &[String] {
        &self.values
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Список NDI-источников из состояния vMix; при заданном имени — только совпавшие.
    pub fn refresh_with_state(&mut self, state: Option<&Node>) -> Result<()> {
        let Some(state) = state else {
            self.values.clear();
            self.error = Some("нет состояния vMix".into());
            return Ok(());
        };
        let mut sources: Vec<String> = state
            .path(&["inputs"])
            .map(|inputs| {
                inputs
                    .children_named("input")
                    .filter(|input| {
                        input
                            .attr("type")
                            .map(|kind| kind.eq_ignore_ascii_case("NDI"))
                            .unwrap_or(false)
                    })
                    .map(|input| {
                        input
                            .attr("title")
                            .map(str::to_string)
                            .unwrap_or_else(|| input.text.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !self.source_name.trim().is_empty() {
            sources.retain(|source| source == self.source_name.trim());
        }
        self.values = sources;
        self.error = None;
        Ok(())
    }
}

/// «A» → 0, «AA» → 26, «3» → 3 (как `ParseExcelColumn` в оригинале).
pub fn column_index(text: &str) -> Option<usize> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().all(|symbol| symbol.is_ascii_digit()) {
        return text.parse().ok();
    }
    let mut value = 0usize;
    for symbol in text.chars() {
        if !symbol.is_ascii_alphabetic() {
            return None;
        }
        value = value * 26 + (symbol.to_ascii_uppercase() as usize - 'A' as usize + 1);
    }
    Some(value.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Свойства провайдера ровно такие, как в `Examples/InputSelector.vmc`.
    fn real_properties() -> Vec<String> {
        vec![
            "http://127.0.0.1:8088/api".into(),
            "./vmix/inputs/input/@number|./vmix/inputs/input/@title".into(),
            String::new(),
            "2".into(),
        ]
    }

    const STATE: &str = r#"<vmix>
<inputs>
<input key="k1" number="1" type="GT" title="Text Middle Centre.gtzip" state="Running"/>
<input key="k2" number="2" type="Video" title="Colour Bars" state="Running"/>
<input key="k3" number="3" type="Colour" title="Red" state="Running"/>
<input key="k4" number="4" type="Colour" title="Green" state="Running"/>
<input key="k5" number="5" type="Colour" title="Blue" state="Running"/>
</inputs>
</vmix>"#;

    #[test]
    fn recognises_provider_by_path() {
        assert_eq!(
            ProviderKind::from_path(r"D:\Coding\vMixUTC\bin\DataProviders\XmlDataProvider.dll"),
            ProviderKind::Xml
        );
        assert_eq!(
            ProviderKind::from_path("/opt/vmix/DataProviders/JsonDataProvider.dll"),
            ProviderKind::Json
        );
        assert_eq!(
            ProviderKind::from_path("ExcelDataProvider.dll"),
            ProviderKind::Excel
        );
        assert!(ProviderKind::Xml.is_supported());
        assert!(ProviderKind::Json.is_supported());
        assert!(ProviderKind::Excel.is_supported());
        assert!(ProviderKind::GoogleSheets.is_supported());
        assert!(ProviderKind::NdiMonitor.is_supported());
        assert!(ProviderKind::FileSystem.is_supported());
    }

    #[test]
    fn reads_properties_like_the_original() {
        let provider = XmlDataProvider::from_properties(&real_properties());
        assert_eq!(provider.url, "http://127.0.0.1:8088/api");
        assert_eq!(provider.group_by, 2);
        assert!(provider.xpath.contains("@number"));
    }

    #[test]
    fn builds_rows_from_union_xpath() {
        let document = vmix_xml::parse(STATE.as_bytes()).unwrap();
        let rows = rows_from_xml(
            &document,
            "./vmix/inputs/input/@number|./vmix/inputs/input/@title",
            2,
        );
        assert_eq!(
            rows,
            vec![
                "1|Text Middle Centre.gtzip".to_string(),
                "2|Colour Bars".to_string(),
                "3|Red".to_string(),
                "4|Green".to_string(),
                "5|Blue".to_string(),
            ]
        );
        // как в оригинале: выбранная строка «5|Blue» — это последняя строка списка
        assert_eq!(rows.last().map(String::as_str), Some("5|Blue"));
    }

    #[test]
    fn single_column_xpath_keeps_one_value_per_row() {
        let document = vmix_xml::parse(STATE.as_bytes()).unwrap();
        let rows = rows_from_xml(&document, "//inputs/input/@title", 1);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0], "Text Middle Centre.gtzip");
    }

    #[test]
    fn refresh_reads_local_file() {
        let path = std::env::temp_dir().join(format!("vmix-provider-{}.xml", std::process::id()));
        std::fs::write(&path, STATE).unwrap();
        let mut provider = XmlDataProvider::from_properties(&[
            path.display().to_string(),
            "//inputs/input/@number".into(),
            String::new(),
            "1".into(),
        ]);
        provider.refresh().unwrap();
        assert_eq!(provider.values(), ["1", "2", "3", "4", "5"]);
        assert!(provider.error().is_none());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn json_provider_reads_values_and_groups_rows() {
        let path = std::env::temp_dir().join(format!("vmix-json-{}.json", std::process::id()));
        std::fs::write(
            &path,
            r#"{"items":[{"name":"Red","score":3},{"name":"Blue","score":1},{"name":"Green","score":2}]}"#,
        )
        .unwrap();

        let mut single = JsonDataProvider::from_properties(&[
            path.display().to_string(),
            "$.items[*].name".into(),
            "1".into(),
            String::new(),
        ]);
        single.refresh().unwrap();
        assert_eq!(single.values(), ["Red", "Blue", "Green"]);
        assert!(single.error().is_none());

        // groupBy=2 → по два значения в строке, как в оригинале
        let mut grouped = JsonDataProvider::from_properties(&[
            path.display().to_string(),
            "$.items[*].name".into(),
            "2".into(),
            String::new(),
        ]);
        grouped.refresh().unwrap();
        assert_eq!(grouped.values(), ["Red|Blue", "Green"]);

        // числа тоже читаются
        let mut scores = JsonDataProvider::from_properties(&[
            path.display().to_string(),
            "$.items[*].score".into(),
            "1".into(),
            String::new(),
        ]);
        scores.refresh().unwrap();
        assert_eq!(scores.values(), ["3", "1", "2"]);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn jsonpath_subset_handles_common_forms() {
        let document: serde_json::Value = serde_json::from_str(
            r#"{"data":{"rows":[{"title":"A"},{"title":"B"}]},"name":"top"}"#,
        )
        .unwrap();
        assert_eq!(jsonpath_select(&document, "$.name"), ["top"]);
        assert_eq!(jsonpath_select(&document, "$.data.rows[*].title"), ["A", "B"]);
        assert_eq!(jsonpath_select(&document, "$['data']['rows'][0]['title']"), ["A"]);
        assert_eq!(jsonpath_select(&document, "$..title"), ["A", "B"]);
        assert!(jsonpath_select(&document, "$.нет.такого").is_empty());
    }

    #[test]
    fn json_provider_reads_http_api_with_headers() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let body = r#"{"scores":[{"team":"Red","value":3},{"team":"Blue","value":1}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            request
        });

        let mut provider = JsonDataProvider::from_properties(&[
            format!("http://127.0.0.1:{port}/api/scores"),
            "$.scores[*].team".into(),
            "1".into(),
            "X-Api-Key: secret\nAccept: application/json".into(),
        ]);
        provider.refresh().unwrap();
        assert_eq!(provider.values(), ["Red", "Blue"]);

        let request = handle.join().unwrap();
        assert!(
            request.to_lowercase().contains("x-api-key: secret"),
            "заголовок не ушёл: {request}"
        );
    }

    #[test]
    fn excel_provider_reads_sheets_ranges_and_tables() {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/table.xlsx");
        let path = fixture.display().to_string();

        // строки-таблицы: срез A..C, со второй строки (первая — заголовок)
        let mut table = ExcelDataProvider::from_properties(&[
            path.clone(),
            "1".into(),
            "3".into(),
            "A".into(),
            "C".into(),
            "Scores".into(),
            "true".into(),
        ]);
        table.refresh().unwrap();
        assert_eq!(
            table.values(),
            ["Red|3|Москва", "Blue|1|Казань", "Green|2|Сочи"],
            "{:?}",
            table.error()
        );

        // без IsTable — отдельные ячейки
        let mut cells = ExcelDataProvider::from_properties(&[
            path.clone(),
            "1".into(),
            "1".into(),
            "A".into(),
            "B".into(),
            "Scores".into(),
            "false".into(),
        ]);
        cells.refresh().unwrap();
        assert_eq!(cells.values(), ["Red", "3"]);

        // лист по индексу: второй лист пустой
        let mut empty = ExcelDataProvider::from_properties(&[
            path.clone(),
            "0".into(),
            "-1".into(),
            "A".into(),
            "C".into(),
            "1".into(),
            "true".into(),
        ]);
        empty.refresh().unwrap();
        assert!(empty.values().is_empty());

        // битый путь не роняет провайдер
        let mut broken = ExcelDataProvider::from_properties(&[
            "/нет/такого.xlsx".into(),
            "0".into(),
            "-1".into(),
            "A".into(),
            "C".into(),
            String::new(),
            "true".into(),
        ]);
        broken.refresh().unwrap();
        assert!(broken.error().is_some());
    }

    #[test]
    fn google_sheets_reads_rows_from_api() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        // ответ как у Google Sheets API v4 с includeGridData
        let body = r#"{"sheets":[{"data":[{"rowData":[
            {"values":[{"formattedValue":"Team"},{"formattedValue":"Score"}]},
            {"values":[{"formattedValue":"Спартак"},{"formattedValue":"3"}]},
            {"values":[{"formattedValue":"Динамо"},{"formattedValue":"1"}]}
        ]}]}]}"#;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..2 {
                let Ok((mut stream, _)) = listener.accept() else { break };
                let mut buffer = [0u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                seen.push(String::from_utf8_lossy(&buffer[..read]).to_string());
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
            seen
        });

        let mut provider = GoogleSheetsDataProvider::from_properties(&[
            "demo-key".into(),
            "1".into(),
            "-1".into(),
            "0".into(),
            "-1".into(),
            "0".into(),
            "true".into(),
            "https://docs.google.com/spreadsheets/d/SHEET123/edit#gid=0".into(),
            "5000".into(),
        ]);
        assert_eq!(provider.spreadsheet_key(), "SHEET123");
        provider.set_base_url(format!("http://127.0.0.1:{port}"));
        provider.refresh().unwrap();
        assert_eq!(
            provider.values(),
            ["Спартак|3", "Динамо|1"],
            "{:?}",
            provider.error()
        );

        // без IsTable — отдельные ячейки
        let mut cells = GoogleSheetsDataProvider::from_properties(&[
            String::new(),
            "1".into(),
            "1".into(),
            "0".into(),
            "1".into(),
            "0".into(),
            "false".into(),
            "SHEET123".into(),
            "5000".into(),
        ]);
        cells.set_base_url(format!("http://127.0.0.1:{port}"));
        cells.refresh().unwrap();
        assert_eq!(cells.values(), ["Спартак", "3"]);

        let requests = handle.join().unwrap();
        assert!(
            requests[0].contains("/spreadsheets/SHEET123?includeGridData=true&key=demo-key"),
            "неверный запрос: {}",
            requests[0]
        );
    }

    #[test]
    fn sheet_key_parses_links() {
        assert_eq!(
            sheet_key_from("https://docs.google.com/spreadsheets/d/abc123/edit#gid=0"),
            "abc123"
        );
        assert_eq!(sheet_key_from("abc123"), "abc123");
        assert_eq!(sheet_key_from(""), "");
    }

    #[test]
    fn ndi_provider_lists_sources_from_vmix_state() {
        let state = vmix_xml::parse(
            br#"<vmix><inputs>
<input key="n1" number="7" type="NDI" title="STUDIO-PC (Camera 1)" state="Running"/>
<input key="n2" number="8" type="NDI" title="LAPTOP (Screen)" state="Running"/>
<input key="v1" number="9" type="Video" title="Clip.mp4" state="Running"/>
</inputs></vmix>"#,
        )
        .unwrap();

        let mut provider = NdiDataProvider::from_properties(&[String::new()]);
        provider.refresh_with_state(Some(&state)).unwrap();
        assert_eq!(
            provider.values(),
            ["STUDIO-PC (Camera 1)", "LAPTOP (Screen)"]
        );

        // фильтр по имени источника, как в свойствах оригинала
        let mut filtered = NdiDataProvider::from_properties(&["LAPTOP (Screen)".into()]);
        filtered.refresh_with_state(Some(&state)).unwrap();
        assert_eq!(filtered.values(), ["LAPTOP (Screen)"]);

        // без состояния — понятная ошибка, без паники
        let mut broken = NdiDataProvider::from_properties(&[String::new()]);
        broken.refresh_with_state(None).unwrap();
        assert!(broken.values().is_empty());
        assert!(broken.error().is_some());
    }

    #[test]
    fn column_index_parses_letters_and_numbers() {
        assert_eq!(column_index("A"), Some(0));
        assert_eq!(column_index("C"), Some(2));
        assert_eq!(column_index("Z"), Some(25));
        assert_eq!(column_index("AA"), Some(26));
        assert_eq!(column_index("AB"), Some(27));
        assert_eq!(column_index("3"), Some(3));
        assert_eq!(column_index(""), None);
    }

    #[test]
    fn refresh_reports_broken_source_without_panicking() {
        let mut provider = XmlDataProvider::from_properties(&[
            "/нет/такого/файла.xml".into(),
            "//inputs/input/@number".into(),
            String::new(),
            "1".into(),
        ]);
        provider.refresh().unwrap();
        assert!(provider.values().is_empty());
        assert!(provider.error().is_some());
    }
}
