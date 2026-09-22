//! Каталог функций vMix и команды виджетов.
//!
//! Перенос `vMixController/Classes/Scripting`:
//! * [`FunctionRef`] — запись каталога (`Functions.xml` / `NewFunctions.xml`) с
//!   `FormatString`, по которому строится запрос к vMix;
//! * [`Command`] — команда кнопки, текстовый вид `Функция(параметры)` с
//!   атрибутами `[C]`, `[!E]`, `[!S]` (`vMixControlButtonCommand.FromString/ToString`);
//! * [`Catalogue`] — поиск функции по имени.
//!
//! Плейсхолдеры `FormatString` (по комментарию в самом `Functions.xml`):
//! `{0}` — Input, `{1}` — Parameter, `{2}` — StringParameter, `{3}` — Parameter-1,
//! `{4}` — номер входа, `{5}` — `Input=` если вход задан, `{6}` — InputKey по номеру,
//! `{7}` — InputKey по строке, `{8}` — Float-параметр.

/// Нативные функции UTC (`Native=true` в `Functions.xml` / `NewFunctions.xml`).
/// Исполняет их само приложение — в vMix такие функции не уходят.
pub const NATIVE_FUNCTIONS: &[&str] = &[
    "API",
    "APIPOST",
    "Condition",
    "ConditionEnd",
    "Delay",
    "Else",
    "EndIf",
    "ExecLink",
    "GoTo",
    "HasVariable",
    "If",
    "IsPressed",
    "LIVEOff",
    "LIVEOn",
    "LIVEToggle",
    "NextPage",
    "None",
    "PrevPage",
    "SetButtonColor",
    "SetGlobalVariable",
    "SetPage",
    "SetVariable",
    "SyncInternalButtonState",
    "SyncState",
    "Timer",
    "ValueChanged",
    "Win",
];

use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::path::Path;
use vmix_xml::Node;

mod state;

pub use state::{
    evaluate, legacy_xpath, values_match, widget_active, xpath_select, xpath_value, InputMaps,
    StateArgs,
};

// --------------------------------------------------------------- каталог

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FunctionRef {
    pub function: String,
    pub description: String,
    pub format_string: String,
    /// `Functions.xml` использует `{0}…{8}`, `NewFunctions.xml` — `$Value`, `$InputKey`, …
    pub new_style: bool,
    pub value_name: String,
    pub category: String,
    pub native: bool,
    pub is_group: bool,
    pub timeout: i64,
    pub has_input: bool,
    pub has_int: bool,
    pub has_string: bool,
    pub has_float: bool,
    pub input_description: String,
    pub int_description: String,
    pub string_description: String,
    pub float_description: String,
    pub active_state_path: String,
    pub active_state_value: String,
    pub active_state_xpath: String,
    /// У части функций путь состояния зависит от параметра (`ActiveStateXPathIntDependence`).
    pub active_state_xpath_int_dependence: Option<Vec<String>>,
    /// Подсказки значений для редактора скрипта (`IntValues` / `StringValues`).
    pub int_values: Vec<String>,
    pub string_values: Vec<String>,
    /// Сколько дополнительных параметров показывает редактор (`AdditionalCount`).
    pub additional_count: usize,
}

impl FunctionRef {
    pub fn from_node(node: &Node, new_style: bool) -> Self {
        let text = |name: &str| node.child_text(name).unwrap_or_default().to_string();
        let flag = |name: &str| matches!(node.child_text(name), Some("true") | Some("True"));
        let format_string = text("FormatString");
        let has = |placeholder: &str| format_string.contains(placeholder);
        let timeout = text("Timeout").parse().unwrap_or(5000);
        Self {
            function: text("Function"),
            description: text("Description"),
            has_input: if new_style {
                has("$InputNumber") || has("$InputKey")
            } else {
                has("{0}")
            },
            has_int: if new_style {
                has("$SelectedIndex") || has("$IntMinus")
            } else {
                has("{1}")
            },
            has_string: if new_style {
                has("$Value")
            } else {
                has("{2}")
            },
            has_float: if new_style { false } else { has("{6}") || has("{8}") },
            input_description: text("InputDescription"),
            int_description: text("IntDescription"),
            string_description: text("StringDescription"),
            float_description: text("FloatDescription"),
            native: flag("Native"),
            is_group: flag("IsGroup"),
            timeout,
            active_state_path: text("ActiveStatePath"),
            active_state_value: text("ActiveStateValue"),
            active_state_xpath: text("ActiveStateXPath"),
            int_values: node
                .child("IntValues")
                .map(|values| {
                    values
                        .children_named("int")
                        .map(|item| item.text.clone())
                        .collect()
                })
                .unwrap_or_default(),
            string_values: node
                .child("StringValues")
                .map(|values| {
                    values
                        .children_named("string")
                        .map(|item| item.text.clone())
                        .collect()
                })
                .unwrap_or_default(),
            additional_count: text("AdditionalCount").parse().unwrap_or(0),
            active_state_xpath_int_dependence: node
                .child("ActiveStateXPathIntDependence")
                .map(|dependence| {
                    dependence
                        .children_named("string")
                        .map(|item| item.text.clone())
                        .collect::<Vec<_>>()
                })
                .filter(|items: &Vec<String>| !items.is_empty()),
            new_style,
            value_name: text("ValueName"),
            category: text("Category"),
            format_string,
        }
    }

    /// Собрать запрос к vMix (`Function=…&Input=…`) для команды.
    pub fn render(&self, command: &Command) -> String {
        if self.new_style {
            return self.render_new_style(command);
        }

        // Как в оригинале (`vMixControlButton.cs`): {0} — ключ входа, если он задан,
        // иначе номер; {4} — номер входа; {5} — префикс «Input=».
        let input_number = command.input.map(|value| value.to_string()).unwrap_or_default();
        let input_key = command.input_key.clone().unwrap_or_default();
        let input = if input_key.is_empty() {
            input_number.clone()
        } else {
            input_key.clone()
        };
        let parameter = command.parameter.clone().unwrap_or_default();
        let string_parameter = url_encode(command.string_parameter.as_deref().unwrap_or_default());
        let parameter_minus_one = command
            .parameter
            .as_deref()
            .and_then(|value| value.trim().parse::<f64>().ok())
            .map(|value| trim_number(value - 1.0))
            .unwrap_or_default();
        let input_prefix = if command.input.is_some() || !input_key.is_empty() {
            "Input="
        } else {
            ""
        };
        let float_parameter = command.float_parameter.clone().unwrap_or_default();

        let values = [
            input,
            parameter,
            string_parameter,
            parameter_minus_one,
            input_number,
            input_prefix.to_string(),
            input_key.clone(),
            input_key,
            float_parameter,
        ];

        let mut result = self.format_string.clone();
        for (index, value) in values.iter().enumerate() {
            result = result.replace(&format!("{{{index}}}"), value);
        }
        result
    }

    /// Новые функции: `Function=Delay&Value=$Value`.
    fn render_new_style(&self, command: &Command) -> String {
        let input = command.input.map(|value| value.to_string()).unwrap_or_default();
        let parameter = command.parameter.clone().unwrap_or_default();
        let minus_one = command
            .parameter
            .as_deref()
            .and_then(|value| value.trim().parse::<f64>().ok())
            .map(|value| trim_number(value - 1.0))
            .unwrap_or_default();
        let replacements = [
            ("$Value", url_encode(command.string_parameter.as_deref().unwrap_or_default())),
            ("$InputKey", command.input_key.clone().unwrap_or_default()),
            ("$InputNumber", input.clone()),
            ("$GetInputKey", command.input_key.clone().unwrap_or_default()),
            ("$SelectedIndex", parameter),
            ("$IntMinus", minus_one),
            ("$Mix", command.mix.clone().unwrap_or_default()),
            ("$Channel", command.channel.clone().unwrap_or_default()),
        ];
        let mut result = self.format_string.clone();
        for (token, value) in replacements {
            result = result.replace(token, &value);
        }
        result
    }
}

/// Два каталога, как в оригинале: `Functions.xml` — для старых кнопок
/// (`Input`/`Parameter`), `NewFunctions.xml` — для новых (`$InputKey`/`$Value`/`$Mix`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalogue {
    classic: BTreeMap<String, FunctionRef>,
    modern: BTreeMap<String, FunctionRef>,
}

impl Catalogue {
    pub fn parse(xml: &[u8]) -> Result<Self> {
        let root = vmix_xml::parse(xml)?;
        let mut classic = BTreeMap::new();
        let mut modern = BTreeMap::new();
        for node in &root.children {
            let new_style = match node.name.as_str() {
                "vMixFunctionReference" => false,
                "vMixNewFunctionReference" => true,
                _ => continue,
            };
            let function = FunctionRef::from_node(node, new_style);
            if function.function.is_empty() {
                continue;
            }
            let target = if new_style { &mut modern } else { &mut classic };
            target.insert(function.function.clone(), function);
        }
        if classic.is_empty() && modern.is_empty() {
            return Err(anyhow!("в каталоге нет функций"));
        }
        Ok(Self { classic, modern })
    }

    /// Загрузить каталог из списка файлов (в оригинале — `Functions.xml` и `NewFunctions.xml`).
    pub fn load(files: &[impl AsRef<Path>]) -> Result<Self> {
        let mut classic = BTreeMap::new();
        let mut modern = BTreeMap::new();
        let mut errors = Vec::new();
        for file in files {
            let path = file.as_ref();
            if !path.exists() {
                continue;
            }
            let bytes = match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    errors.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            match Self::parse(&bytes) {
                Ok(catalogue) => {
                    classic.extend(catalogue.classic);
                    modern.extend(catalogue.modern);
                }
                Err(error) => errors.push(format!("{}: {error:#}", path.display())),
            }
        }
        if classic.is_empty() && modern.is_empty() {
            return Err(anyhow!(
                "каталог функций не загружен: {}",
                if errors.is_empty() {
                    "файлы не найдены".to_string()
                } else {
                    errors.join("; ")
                }
            ));
        }
        Ok(Self { classic, modern })
    }

    /// Каталог рядом с исполняемым файлом или в каталоге проекта (для разработки).
    /// Куда порт смотрит в поисках `Functions.xml` / `NewFunctions.xml`.
    ///
    /// Порядок: переменная окружения → каталог рядом с исполняемым файлом (в том числе
    /// ресурсы пакета: `data/` у exe, `../data` и `../Resources/data` в macOS-бандле) →
    /// системные каталоги (`/usr/share/vmixrtc` из пакетов Arch/deb) → текущий каталог.
    pub fn discover_candidates() -> Vec<std::path::PathBuf> {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(dir) = std::env::var("VMIX_FUNCTIONS_DIR") {
            candidates.push(std::path::PathBuf::from(dir));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                // рядом с бинарём и в подкаталоге data/
                candidates.push(dir.to_path_buf());
                candidates.push(dir.join("data"));
                candidates.push(dir.join("../data"));
                // macOS: vMixRTC.app/Contents/MacOS/vmixrtc → Contents/Resources/data
                candidates.push(dir.join("../Resources/data"));
                candidates.push(dir.join("../Resources"));
                // Linux-пакеты Tauri: бинарь в /usr/bin, ресурсы в /usr/lib/<Product>/data
                // (так же внутри AppImage и в распакованном дереве)
                candidates.push(dir.join("../lib/vMixRTC/data"));
                candidates.push(dir.join("../lib/vmixrtc/data"));
                candidates.push(dir.join("../lib/vMixRTC"));
                candidates.push(dir.join("../lib/vmixrtc"));
            }
        }
        // системные каталоги пакетов (Arch: /usr/share/vmixrtc, deb: /usr/share/vmixrtc)
        candidates.push(std::path::PathBuf::from("/usr/lib/vMixRTC/data"));
        candidates.push(std::path::PathBuf::from("/usr/lib/vmixrtc/data"));
        candidates.push(std::path::PathBuf::from("/usr/lib/vMixRTC"));
        candidates.push(std::path::PathBuf::from("/usr/lib/vmixrtc"));
        candidates.push(std::path::PathBuf::from("/usr/share/vmixrtc"));
        candidates.push(std::path::PathBuf::from("/usr/local/share/vmixrtc"));
        // запуск из дерева исходников
        candidates.push(std::path::PathBuf::from("data"));
        candidates.push(std::path::PathBuf::from("../data"));
        candidates.push(std::path::PathBuf::from("../../data"));
        candidates
    }

    pub fn discover() -> Result<Self> {
        let candidates = Self::discover_candidates();

        let mut tried = Vec::new();
        for dir in candidates {
            let files = [dir.join("Functions.xml"), dir.join("NewFunctions.xml")];
            let existing: Vec<_> = files.iter().filter(|path| path.exists()).cloned().collect();
            if existing.is_empty() {
                tried.push(dir.display().to_string());
                continue;
            }
            if let Ok(catalogue) = Self::load(&existing) {
                return Ok(catalogue);
            }
            tried.push(dir.display().to_string());
        }
        Err(anyhow!(
            "Functions.xml не найден (искали в: {})",
            tried.join(", ")
        ))
    }

    /// Поиск: сначала старый каталог (классические кнопки), затем новый.
    pub fn find(&self, function: &str) -> Option<&FunctionRef> {
        self.find_classic(function).or_else(|| self.find_modern(function))
    }

    pub fn find_classic(&self, function: &str) -> Option<&FunctionRef> {
        self.classic.get(function)
    }

    pub fn find_modern(&self, function: &str) -> Option<&FunctionRef> {
        self.modern.get(function)
    }

    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .classic
            .keys()
            .chain(self.modern.keys())
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Функции, которые можно вызывать (без групп и заглушки `None`).
    pub fn callable_names(&self, modern: bool) -> Vec<&str> {
        let source = if modern { &self.modern } else { &self.classic };
        let mut names: Vec<&str> = source
            .values()
            .filter(|function| !function.is_group && function.function != "None")
            .map(|function| function.function.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    pub fn len(&self) -> usize {
        self.names().len()
    }

    pub fn is_empty(&self) -> bool {
        self.classic.is_empty() && self.modern.is_empty()
    }

    /// Запрос к vMix по команде: приоритет — сохранённый в команде `FormatString`,
    /// затем старый каталог, затем новый (как два разных типа кнопок в оригинале).
    pub fn render(&self, command: &Command) -> Result<String> {
        if let Some(format) = command.format_string.as_deref().filter(|f| !f.is_empty()) {
            let function = FunctionRef {
                function: command.function.clone(),
                format_string: format.to_string(),
                // в .vmc у новых кнопок формат со `$`, у старых — с `{n}`
                new_style: format.contains('$'),
                ..Default::default()
            };
            return Ok(function.render(command));
        }
        self.find_classic(&command.function)
            .or_else(|| self.find_modern(&command.function))
            .map(|function| function.render(command))
            .ok_or_else(|| anyhow!("неизвестная функция: {}", command.function))
    }
}

// --------------------------------------------------------------- команда

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Command {
    pub function: String,
    pub input: Option<i64>,
    pub input_key: Option<String>,
    pub parameter: Option<String>,
    pub string_parameter: Option<String>,
    pub float_parameter: Option<String>,
    /// Только у новых кнопок: `$Mix` и `$Channel`.
    pub mix: Option<String>,
    pub channel: Option<String>,
    /// `Condition` сравнивает два выражения из этих параметров
    /// (`AdditionalParameters[1] <оператор [2]> AdditionalParameters[3]`).
    pub additional_parameters: Vec<String>,
    pub collapsed: bool,
    pub executable: bool,
    pub use_in_active_state: bool,
    /// `FormatString`, если команда пришла из `.vmc` (там он хранится внутри команды).
    pub format_string: Option<String>,
    /// Сигнатура функции внутри команды — аналог `Action` в оригинале.
    pub description: String,
    pub native: bool,
    pub active_state_path: String,
    pub active_state_xpath: String,
    pub active_state_value: String,
    pub active_state_xpath_int_dependence: Vec<String>,
}

impl Command {
    pub fn new(function: impl Into<String>) -> Self {
        Self {
            function: function.into(),
            executable: true,
            use_in_active_state: true,
            ..Default::default()
        }
    }

    pub fn with_input(mut self, input: i64) -> Self {
        self.input = Some(input);
        self
    }

    pub fn with_parameter(mut self, value: impl Into<String>) -> Self {
        self.parameter = Some(value.into());
        self
    }

    pub fn with_string(mut self, value: impl Into<String>) -> Self {
        self.string_parameter = Some(value.into());
        self
    }

    /// Запрос к vMix: при наличии каталога — через него (как в оригинале), иначе
    /// по сохранённому в команде `FormatString`.
    pub fn render(&self, catalogue: Option<&Catalogue>) -> Result<String> {
        if let Some(catalogue) = catalogue {
            return catalogue.render(self);
        }
        let format = self
            .format_string
            .as_deref()
            .filter(|format| !format.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "для команды «{}» нет FormatString и недоступен каталог функций",
                    self.function
                )
            })?;
        let function = FunctionRef {
            function: self.function.clone(),
            format_string: format.to_string(),
            new_style: format.contains('$'),
            ..Default::default()
        };
        Ok(function.render(self))
    }

    /// Текстовый вид `Функция(параметры)` — как `ToString()` в оригинале.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        if self.collapsed {
            text.push_str("[C] ");
        }
        if !self.executable {
            text.push_str("[!E] ");
        }
        if !self.use_in_active_state {
            text.push_str("[!S] ");
        }
        text.push_str(&self.function);
        text.push('(');
        let mut parameters: Vec<String> = Vec::new();
        if let Some(key) = &self.input_key {
            if !key.is_empty() {
                parameters.push(key.clone());
            }
        } else if let Some(input) = self.input {
            parameters.push(input.to_string());
        }
        if let Some(parameter) = &self.parameter {
            parameters.push(parameter.clone());
        }
        if let Some(value) = &self.float_parameter {
            parameters.push(value.clone());
        }
        if let Some(value) = &self.string_parameter {
            parameters.push(value.clone());
        }
        for value in &self.additional_parameters {
            parameters.push(value.clone());
        }
        text.push_str(&parameters.join(","));
        text.push(')');
        text
    }

    /// Разбор строки с учётом сигнатуры функции: какие параметры относятся к входу,
    /// числу, тексту, float и дополнительным (как `vMixControlButtonCommand.FromString`).
    pub fn parse_with_signature(text: &str, catalogue: &Catalogue) -> Result<Self> {
        let mut command = Self::parse(text)?;
        let Some(function) = catalogue.find(&command.function) else {
            return Ok(command);
        };

        // заново разберём позиционные параметры — на этот раз по сигнатуре
        let parameters = positional_parameters(text);
        let mut queue = parameters.into_iter().filter(|value| !value.is_empty());

        command.input = None;
        command.input_key = None;
        command.parameter = None;
        command.string_parameter = None;
        command.float_parameter = None;
        command.additional_parameters.clear();

        if function.has_input {
            if let Some(first) = queue.next() {
                match first.trim_matches(|symbol| symbol == '\'' || symbol == '"') {
                    value if value.trim().parse::<i64>().is_ok() => {
                        command.input = value.trim().parse().ok()
                    }
                    value => command.input_key = Some(value.to_string()),
                }
            }
        }
        if function.has_int {
            command.parameter = queue.next();
        }
        if function.has_string {
            command.string_parameter = queue.next();
        }
        if function.has_float {
            command.float_parameter = queue.next();
        }
        for _ in 0..function.additional_count.max(if command.function == "Condition" { 4 } else { 0 }) {
            match queue.next() {
                Some(value) => command.additional_parameters.push(value),
                None => command.additional_parameters.push(String::new()),
            }
        }

        command.format_string = Some(function.format_string.clone());
        command.description = function.description.clone();
        command.native = function.native;
        command.active_state_path = function.active_state_path.clone();
        command.active_state_xpath = function.active_state_xpath.clone();
        command.active_state_value = function.active_state_value.clone();
        command.active_state_xpath_int_dependence =
            function.active_state_xpath_int_dependence.clone().unwrap_or_default();
        Ok(command)
    }

    /// Разбор текстового вида (атрибуты + имя функции + параметры по порядку).
    pub fn parse(text: &str) -> Result<Self> {
        let mut command = Self {
            executable: true,
            use_in_active_state: true,
            ..Default::default()
        };
        let mut rest = text.trim();
        loop {
            if let Some(tail) = rest.strip_prefix("[C] ") {
                command.collapsed = true;
                rest = tail;
            } else if let Some(tail) = rest.strip_prefix("[!E] ") {
                command.executable = false;
                rest = tail;
            } else if let Some(tail) = rest.strip_prefix("[!S] ") {
                command.use_in_active_state = false;
                rest = tail;
            } else {
                break;
            }
        }

        let open = match rest.find('(') {
            Some(open) => open,
            // как в оригинале: имя функции может быть без скобок
            None => {
                command.function = rest.trim().to_string();
                if command.function.is_empty() {
                    return Err(anyhow!("пустая команда"));
                }
                return Ok(command);
            }
        };
        let close = rest
            .rfind(')')
            .ok_or_else(|| anyhow!("нет «)» в команде «{text}»"))?;
        if close < open {
            return Err(anyhow!("неверные скобки в команде «{text}»"));
        }
        command.function = rest[..open].trim().to_string();
        if command.function.is_empty() {
            return Err(anyhow!("пустое имя функции в «{text}»"));
        }

        let parameters: Vec<String> = split_parameters(&rest[open + 1..close])
            .into_iter()
            .map(|value| value.trim().to_string())
            .collect();
        let mut queue = parameters.into_iter().filter(|value| !value.is_empty());

        if let Some(first) = queue.next() {
            match first.parse::<i64>() {
                Ok(number) => command.input = Some(number),
                Err(_) => command.input_key = Some(first),
            }
        }
        command.parameter = queue.next();
        // третий и четвёртый параметры — по смыслу функции: у части функций это Float
        let third = queue.next();
        let fourth = queue.next();
        if fourth.is_some() {
            command.float_parameter = third;
            command.string_parameter = fourth;
        } else {
            command.string_parameter = third;
        }
        Ok(command)
    }
}

/// Позиционные параметры из текстового вида команды.
fn positional_parameters(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    let Some(open) = trimmed.find('(') else {
        return Vec::new();
    };
    let Some(close) = trimmed.rfind(')') else {
        return Vec::new();
    };
    split_parameters(&trimmed[open + 1..close])
        .into_iter()
        .map(|value| value.trim().to_string())
        .collect()
}

/// Разделить параметры по запятым, не трогая запятые внутри кавычек.
fn split_parameters(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for symbol in text.chars() {
        match quote {
            Some(open) if symbol == open => {
                quote = None;
                current.push(symbol);
            }
            Some(_) => current.push(symbol),
            None if symbol == '"' || symbol == '\'' => {
                quote = Some(symbol);
                current.push(symbol);
            }
            None if symbol == ',' => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            None => current.push(symbol),
        }
    }
    parts.push(current.trim().to_string());
    parts
}

fn trim_number(value: f64) -> String {
    if (value.fract()).abs() < f64::EPSILON {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn discover_candidates_cover_package_paths() {
        let candidates = super::Catalogue::discover_candidates();
        let as_text: Vec<String> = candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        assert!(
            as_text.iter().any(|path| path == "/usr/share/vmixrtc"),
            "нет системного каталога пакета: {as_text:?}"
        );
        assert!(
            as_text.iter().any(|path| path.ends_with("Resources/data")),
            "нет ресурсов macOS-бандла: {as_text:?}"
        );
        assert!(
            as_text.iter().any(|path| path.ends_with("lib/vMixRTC/data")),
            "нет каталога ресурсов Linux-пакета: {as_text:?}"
        );
        assert!(
            as_text.iter().any(|path| path == "/usr/lib/vMixRTC/data"),
            "нет системного каталога ресурсов: {as_text:?}"
        );
        assert!(
            as_text.iter().any(|path| path.ends_with("data")),
            "нет запуска из дерева исходников: {as_text:?}"
        );
    }

    use super::*;

    fn real_catalogue() -> Catalogue {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        Catalogue::load(&[dir.join("Functions.xml"), dir.join("NewFunctions.xml")]).unwrap()
    }

    #[test]
    fn loads_the_real_catalogue() {
        let catalogue = real_catalogue();
        assert!(catalogue.len() > 800, "функций: {}", catalogue.len());

        // старый каталог: Input/Parameter через {n}
        let volume = catalogue.find_classic("SetVolume").expect("classic SetVolume");
        assert!(!volume.new_style);
        assert!(volume.has_input);
        assert!(volume.format_string.contains("{0}"));

        // новый каталог: $InputKey/$Value, есть Cut и Delay
        let cut = catalogue.find_modern("Cut").expect("modern Cut");
        assert!(cut.new_style);
        assert!(cut.format_string.contains("$Mix"));

        let delay = catalogue.find_modern("Delay").expect("Delay");
        assert!(delay.new_style);
        assert_eq!(delay.value_name, "Duration");
    }

    #[test]
    fn renders_new_style_queries() {
        let catalogue = real_catalogue();
        let delay = Command {
            function: "Delay".into(),
            string_parameter: Some("500".into()),
            ..Default::default()
        };
        assert_eq!(catalogue.render(&delay).unwrap(), "Function=Delay&Value=500");

        let cut = Command {
            function: "Cut".into(),
            mix: Some("2".into()),
            ..Default::default()
        };
        assert_eq!(catalogue.render(&cut).unwrap(), "Function=Cut&Mix=2");
    }

    #[test]
    fn renders_queries_like_the_original() {
        let catalogue = real_catalogue();

        let volume = Command::new("SetVolume").with_input(1).with_parameter("50");
        assert_eq!(
            catalogue.render(&volume).unwrap(),
            "Function=SetVolume&Input=1&Value=50"
        );

        let no_parameters = Command::new("StartRecording");
        assert_eq!(
            catalogue.render(&no_parameters).unwrap(),
            "Function=StartRecording"
        );

        let text = Command::new("SetText")
            .with_input(2)
            .with_parameter("0")
            .with_string("Привет мир");
        let rendered = catalogue.render(&text).unwrap();
        assert!(rendered.starts_with("Function=SetText&"));
        assert!(rendered.contains("Input=2"));
        assert!(rendered.contains("%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82%20%D0%BC%D0%B8%D1%80"));
    }

    #[test]
    fn command_text_round_trip() {
        for text in [
            "Cut",
            "Cut(1)",
            "SetVolume(2,75)",
            "SetText(1,0,\"Гол 1:0\")",
            "[C] Fade(3,500)",
            "[!E] StartRecording()",
        ] {
            let command = Command::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            let again = Command::parse(&command.to_text()).unwrap();
            assert_eq!(command, again, "не сошлось на «{text}»");
        }
    }

    #[test]
    fn renders_real_command_from_the_example_controller() {
        // команда из Scoreboard.vmc: {0} — ключ входа, {1} — параметр
        let command = Command {
            function: "OverlayInputX".into(),
            input: Some(-1),
            input_key: Some("2d74ef94-182f-4688-9f98-1e2138435092".into()),
            parameter: Some("1".into()),
            format_string: Some("Function=OverlayInput{1}&Input={0}".into()),
            ..Default::default()
        };
        assert_eq!(
            command.render(None).unwrap(),
            "Function=OverlayInput1&Input=2d74ef94-182f-4688-9f98-1e2138435092"
        );
    }

    #[test]
    fn command_from_vmc_uses_embedded_format_string() {
        // так команда выглядит в .vmc: Action/FormatString лежит внутри команды
        let command = Command {
            function: "SetVolume".into(),
            input: Some(1),
            parameter: Some("33".into()),
            format_string: Some("Function=SetVolume&Input={0}&Value={1}".into()),
            ..Default::default()
        };
        assert_eq!(
            command.render(None).unwrap(),
            "Function=SetVolume&Input=1&Value=33"
        );
    }

    #[test]
    fn unknown_function_is_reported() {
        let catalogue = real_catalogue();
        let command = Command::new("НетТакойФункции");
        assert!(catalogue.render(&command).is_err());
    }
}
