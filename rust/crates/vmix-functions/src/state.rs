//! Состояние виджета по данным vMix — порт `vMixControlButtonHelper.CalculateStateDependency`.
//!
//! Оригинал: подставляет аргументы команды в `ActiveStateXPath`, вытаскивает значение из
//! XML состояния vMix и сравнивает с `ActiveStateValue` (с префиксами-операторами).
//! Здесь тот же порядок действий плюс минимальный XPath, покрывающий все пути из
//! `Functions.xml` / `NewFunctions.xml` (29 уникальных форм).

use crate::{Command, FunctionRef};
use std::collections::BTreeMap;
use vmix_xml::Node;

// --------------------------------------------------------------- карты входов

#[derive(Debug, Clone, Default)]
pub struct InputMaps {
    pub number_by_key: BTreeMap<String, i64>,
    pub key_by_number: BTreeMap<i64, String>,
}

impl InputMaps {
    /// Собрать соответствия ключ ↔ номер из состояния vMix (`//inputs/input`).
    pub fn from_state(state: &Node) -> Self {
        let mut maps = Self::default();
        let Some(inputs) = state.child("inputs") else {
            return maps;
        };
        for input in inputs.children_named("input") {
            let (Some(key), Some(number)) = (
                input.attr("key"),
                input.attr("number").and_then(|value| value.parse::<i64>().ok()),
            ) else {
                continue;
            };
            maps.number_by_key.insert(key.to_string(), number);
            maps.key_by_number.insert(number, key.to_string());
        }
        maps
    }
}

// --------------------------------------------------------------- аргументы

/// Аргументы форматирования путей и значений — как `XPathFormattingArgs` в оригинале.
#[derive(Debug, Clone, Default)]
pub struct StateArgs {
    pub input_key: String,
    pub input_number: i64,
    pub int_parameter: i64,
    pub string_parameter: String,
    pub key_by_int: String,
    pub key_by_string: String,
    pub float_parameter: String,
}

impl StateArgs {
    pub fn from_command(command: &Command, maps: &InputMaps) -> Self {
        let input_key = command.input_key.clone().unwrap_or_default();
        let input_number = if input_key.is_empty() {
            command.input.unwrap_or(-1)
        } else {
            *maps.number_by_key.get(&input_key).unwrap_or(&-1)
        };
        let int_parameter = command
            .parameter
            .as_deref()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or(0);
        let string_parameter = command.string_parameter.clone().unwrap_or_default();
        let string_as_int = string_parameter.trim().parse::<i64>().ok();
        Self {
            key_by_int: maps
                .key_by_number
                .get(&int_parameter)
                .cloned()
                .unwrap_or_default(),
            key_by_string: string_as_int
                .and_then(|number| maps.key_by_number.get(&number).cloned())
                .unwrap_or_default(),
            input_key,
            input_number,
            int_parameter,
            string_parameter,
            float_parameter: command.float_parameter.clone().unwrap_or_default(),
        }
    }

    /// Подстановка в шаблон: те же девять позиций, что в оригинале.
    pub fn format(&self, template: &str) -> String {
        let values = [
            self.input_key.clone(),
            self.int_parameter.to_string(),
            self.string_parameter.clone(),
            (self.int_parameter - 1).to_string(),
            self.input_number.to_string(),
            String::new(),
            self.key_by_int.clone(),
            self.key_by_string.clone(),
            self.float_parameter.clone(),
        ];
        let mut result = template.to_string();
        for (index, value) in values.iter().enumerate() {
            result = result.replace(&format!("{{{index}}}"), value);
        }
        result
    }
}

// --------------------------------------------------------------- минимальный XPath

/// Значение по XPath из наблюдаемых форм:
/// `//inputs/input[@key='x']/@state`, `//audio/bus1/@volume`, `//active`, `//overlays/overlay[@number='1']`.
pub fn xpath_value(state: &Node, xpath: &str) -> Option<String> {
    let descendant = xpath.starts_with("//");
    let steps: Vec<&str> = xpath
        .trim_start_matches('/')
        .split('/')
        .filter(|step| !step.is_empty())
        .collect();
    let (first, rest) = steps.split_first()?;

    let mut current = if descendant {
        state
            .descendants(element_name(first))
            .into_iter()
            .find(|node| step_matches(node, first))?
    } else {
        let node = state.child(element_name(first))?;
        if !step_matches(node, first) {
            return None;
        }
        node
    };

    for step in rest {
        if let Some(attribute) = step.strip_prefix('@') {
            return current.attr(attribute).map(str::to_string);
        }
        current = current
            .children
            .iter()
            .find(|node| step_matches(node, step))?;
    }
    Some(current.text.clone())
}

/// Выбрать **все** значения по XPath (нужно провайдерам данных, в отличие от
/// `xpath_value`, который берёт первое совпадение).
pub fn xpath_select(state: &Node, xpath: &str) -> Vec<String> {
    let descendant = xpath.starts_with("//");
    let relative = xpath.starts_with("./");
    let steps: Vec<&str> = xpath
        .trim_start_matches("./")
        .trim_start_matches('/')
        .split('/')
        .filter(|step| !step.is_empty())
        .collect();
    let Some((first, rest)) = steps.split_first() else {
        return Vec::new();
    };

    // первый шаг: // — по всем потомкам; `./vmix/...` начинается с самого корня
    let mut current: Vec<&Node> = if element_name(first) == state.name {
        vec![state]
    } else if descendant {
        state
            .descendants(element_name(first))
            .into_iter()
            .filter(|node| step_matches(node, first))
            .collect()
    } else {
        state
            .child(element_name(first))
            .filter(|node| step_matches(node, first))
            .into_iter()
            .collect()
    };
    let _ = relative;

    for step in rest {
        if let Some(attribute) = step.strip_prefix('@') {
            return current
                .into_iter()
                .filter_map(|node| node.attr(attribute))
                .map(str::to_string)
                .collect();
        }
        current = current
            .into_iter()
            .flat_map(|node| node.children.iter())
            .filter(|node| step_matches(node, step))
            .collect();
    }

    current
        .into_iter()
        .map(|node| node.text.clone())
        .collect()
}

fn element_name(step: &str) -> &str {
    step.split('[').next().unwrap_or(step)
}

fn step_matches(node: &Node, step: &str) -> bool {
    let Some((name, predicate)) = step.split_once('[') else {
        return node.name == step;
    };
    if node.name != name {
        return false;
    }
    let predicate = predicate.trim_end_matches(']').trim();
    let Some(attribute) = predicate.strip_prefix('@') else {
        return false;
    };
    let Some((key, value)) = attribute.split_once('=') else {
        return false;
    };
    let value = value.trim().trim_matches(|symbol| symbol == '\'' || symbol == '"');
    node.attr(key.trim()) == Some(value)
}

// --------------------------------------------------------------- сравнение

/// Сравнение фактического значения с шаблоном — порт `ValuesMatch`:
/// `!` — отрицание, `~` — содержит, `` ` `` — не содержит, `*` — любое, `-` — пусто.
pub fn values_match(actual: &str, expected_pattern: &str) -> bool {
    let mut pattern = expected_pattern;
    let mut negated = false;
    if let Some(rest) = pattern.strip_prefix('!') {
        negated = true;
        pattern = rest;
    }

    let (operator, value) = match pattern.chars().next() {
        Some(symbol @ ('~' | '`')) => (Some(symbol), &pattern[symbol.len_utf8()..]),
        other => (other, pattern),
    };

    let result = match operator {
        Some('~') => actual.contains(value),
        Some('`') => !actual.contains(value),
        _ if pattern == "*" => true,
        _ if pattern == "-" => actual.trim().is_empty(),
        _ => actual == pattern,
    };

    if negated {
        !result
    } else {
        result
    }
}

// --------------------------------------------------------------- вычисление

/// Активно ли состояние для команды: `None`, если у функции нет пути состояния.
pub fn evaluate(
    command: &Command,
    function: &FunctionRef,
    state: &Node,
    maps: &InputMaps,
) -> Option<bool> {
    if !command.use_in_active_state {
        return None;
    }
    let args = StateArgs::from_command(command, maps);
    let template = effective_xpath(function, &args)?;
    let xpath = args.format(&template);
    let actual = xpath_value(state, &xpath)?;
    let expected = args.format(&function.active_state_value);
    Some(values_match(&actual, &expected))
}

fn effective_xpath(function: &FunctionRef, args: &StateArgs) -> Option<String> {
    if let Some(templates) = function.active_state_xpath_int_dependence.as_ref() {
        if args.int_parameter >= 0 {
            if let Some(template) = templates.get(args.int_parameter as usize) {
                if !template.trim().is_empty() {
                    return Some(template.clone());
                }
            }
        }
    }
    let xpath = function.active_state_xpath.trim();
    if !xpath.is_empty() {
        return Some(xpath.to_string());
    }
    // Устаревшая форма. В текущем оригинале состояние считается только по XPath,
    // поэтому в старых `.vmc` (например, в Examples/*.vmc) подсветка не работает;
    // порт умеет и её — переводим путь в XPath.
    legacy_xpath(&function.active_state_path)
}

/// Перевод устаревшего `ActiveStatePath` в XPath. Покрывает все формы из
/// `Functions.xml` / `NewFunctions.xml` (23 уникальных пути).
pub fn legacy_xpath(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    match path {
        "Active" | "Preview" | "Recording" | "Streaming" | "External" | "MultiCorder"
        | "FadeToBlack" | "Fullscreen" => {
            // camelCase: Active → active, MultiCorder → multiCorder
            let mut name = path[..1].to_lowercase();
            name.push_str(&path[1..]);
            return Some(format!("//{name}"));
        }
        _ => {}
    }

    // Overlays[{3}].ActiveInput — {3} = параметр-1, номер оверлея 1-based = {1}
    if path.starts_with("Overlays[") && path.ends_with("].ActiveInput") {
        return Some("//overlays/overlay[@number='{1}']".to_string());
    }

    // Audio[Master].Muted / Audio[Bus{2}].Muted
    if let Some(rest) = path.strip_prefix("Audio[") {
        let (target, tail) = rest.split_once(']')?;
        let attribute = match tail.trim_start_matches('.') {
            "Muted" => "@muted",
            "Solo" => "@solo",
            "SendToMaster" => "@sendToMaster",
            "Volume" => "@volume",
            _ => return None,
        };
        let element = if target.eq_ignore_ascii_case("master") {
            "master".to_string()
        } else {
            format!("bus{}", target.trim_start_matches("Bus"))
        };
        return Some(format!("//audio/{element}/{attribute}"));
    }

    // Inputs[{0}].<свойство> и Inputs[{0}].Elements[<тип>/Index/{n}].<поле>
    if let Some(rest) = path.strip_prefix("Inputs[") {
        let (input, tail) = rest.split_once(']')?;
        let prefix = format!("//inputs/input[@key='{input}']");
        let tail = tail.trim_start_matches('.');

        if let Some(elements) = tail.strip_prefix("Elements[") {
            let (element, field) = elements.split_once(']')?;
            let field = field.trim_start_matches('.');
            let index = element.split('/').last().unwrap_or_default();
            return match field {
                "Text" => Some(format!("{prefix}/text[@index='{index}']")),
                "Image" => Some(format!("{prefix}/image[@index='{index}']")),
                "Key" => Some(format!("{prefix}/overlay[@index='{index}']/@key")),
                _ => None,
            };
        }

        let attribute = match tail {
            "State" => "@state",
            "Muted" => "@muted",
            "Solo" => "@solo",
            "Volume" => "@volume",
            "Audiobusses" => "@audiobusses",
            "Balance" => "@balance",
            "GainDb" => "@gainDb",
            _ => return None,
        };
        return Some(format!("{prefix}/{attribute}"));
    }

    None
}

/// Вспомогательное: карты входов и вычисление для набора команд виджета.
pub fn widget_active(
    commands: &[Command],
    functions: &[Option<&FunctionRef>],
    state: &Node,
) -> bool {
    let maps = InputMaps::from_state(state);
    commands
        .iter()
        .zip(functions.iter())
        .any(|(command, function)| match function {
            Some(function) => evaluate(command, function, state, &maps) == Some(true),
            None => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = r#"<vmix>
<inputs>
<input key="keyA" number="1" type="Colour" title="Студия" state="Running" muted="False" volume="100" audiobusses="M"/>
<input key="keyB" number="2" type="Video" title="Камера" state="Paused" muted="True" volume="50" audiobusses="M,A"/>
</inputs>
<overlays><overlay number="1">keyA</overlay><overlay number="2"/></overlays>
<audio><master volume="95" muted="False"/><bus1 volume="80" muted="True"/></audio>
<transition number="1" effect="Cut" duration="0"/>
<recording>True</recording><streaming>False</streaming><external>False</external>
<multiCorder>False</multiCorder><fadeToBlack>False</fadeToBlack><fullscreen>False</fullscreen>
<active>2</active>
</vmix>"#;

    fn state() -> Node {
        vmix_xml::parse(STATE.as_bytes()).unwrap()
    }

    #[test]
    fn minimal_xpath_reads_observed_paths() {
        let state = state();
        assert_eq!(
            xpath_value(&state, "//inputs/input[@key='keyB']/@state").as_deref(),
            Some("Paused")
        );
        assert_eq!(xpath_value(&state, "//active").as_deref(), Some("2"));
        assert_eq!(xpath_value(&state, "//audio/bus1/@volume").as_deref(), Some("80"));
        assert_eq!(
            xpath_value(&state, "//overlays/overlay[@number='1']").as_deref(),
            Some("keyA")
        );
        assert_eq!(
            xpath_value(&state, "//inputs/input[@key='keyA']/@audiobusses").as_deref(),
            Some("M")
        );
        assert_eq!(xpath_value(&state, "//inputs/input[@key='нет']/@state"), None);
        assert_eq!(xpath_value(&state, "//recording").as_deref(), Some("True"));
    }

    #[test]
    fn xpath_select_returns_all_matches() {
        let state = state();
        assert_eq!(
            xpath_select(&state, "./vmix/inputs/input/@number"),
            vec!["1".to_string(), "2".to_string()]
        );
        assert_eq!(
            xpath_select(&state, "//inputs/input[@key='keyB']/@state"),
            vec!["Paused".to_string()]
        );
        assert_eq!(
            xpath_select(&state, "//inputs/input/@title"),
            vec!["Студия".to_string(), "Камера".to_string()]
        );
        assert!(xpath_select(&state, "//нет/такого").is_empty());
    }

    #[test]
    fn values_match_handles_operators() {
        assert!(values_match("Running", "Running"));
        assert!(!values_match("Paused", "Running"));
        assert!(values_match("Paused", "!Running"));
        assert!(values_match("M,A", "~A"));
        assert!(!values_match("M,A", "`A"));
        assert!(values_match("что угодно", "*"));
        assert!(values_match("   ", "-"));
        assert!(!values_match("x", "-"));
    }

    #[test]
    fn args_use_input_maps_like_the_original() {
        let maps = InputMaps::from_state(&state());
        let command = Command {
            function: "SetText".into(),
            input_key: Some("keyB".into()),
            parameter: Some("1".into()),
            ..Default::default()
        };
        let args = StateArgs::from_command(&command, &maps);
        assert_eq!(args.input_number, 2);
        assert_eq!(args.key_by_int, "keyA"); // вход с номером 1
        assert_eq!(
            args.format("//inputs/input[@key='{0}']/text[@index='{1}']"),
            "//inputs/input[@key='keyB']/text[@index='1']"
        );
    }

    #[test]
    fn evaluates_real_function_paths() {
        let maps = InputMaps::from_state(&state());
        let state_node = state();

        // Recording → True
        let recording = FunctionRef {
            function: "StartRecording".into(),
            active_state_xpath: "//recording".into(),
            active_state_value: "True".into(),
            ..Default::default()
        };
        let command = Command::new("StartRecording");
        assert_eq!(evaluate(&command, &recording, &state_node, &maps), Some(true));

        // //inputs/input[@key='{0}']/@state == Running
        let running = FunctionRef {
            function: "InputRunning".into(),
            active_state_xpath: "//inputs/input[@key='{0}']/@state".into(),
            active_state_value: "Running".into(),
            ..Default::default()
        };
        let stydio = Command {
            function: "InputRunning".into(),
            input_key: Some("keyA".into()),
            use_in_active_state: true,
            ..Default::default()
        };
        let camera = Command {
            function: "InputRunning".into(),
            input_key: Some("keyB".into()),
            use_in_active_state: true,
            ..Default::default()
        };
        assert_eq!(evaluate(&stydio, &running, &state_node, &maps), Some(true));
        assert_eq!(evaluate(&camera, &running, &state_node, &maps), Some(false));
    }

    #[test]
    fn overlay_command_from_scoreboard_is_recognised() {
        // команда из Scoreboard.vmc: Parameter=1 → OverlayInput1, значение — ключ входа
        let maps = InputMaps::from_state(&state());
        let function = FunctionRef {
            function: "OverlayInputX".into(),
            active_state_xpath: "//overlays/overlay[@number='{1}']".into(),
            active_state_value: "{0}".into(),
            ..Default::default()
        };
        let active = Command {
            function: "OverlayInputX".into(),
            input_key: Some("keyA".into()),
            input: Some(-1),
            parameter: Some("1".into()),
            use_in_active_state: true,
            ..Default::default()
        };
        let inactive = Command {
            function: "OverlayInputX".into(),
            input_key: Some("keyB".into()),
            input: Some(-1),
            parameter: Some("1".into()),
            use_in_active_state: true,
            ..Default::default()
        };
        assert_eq!(evaluate(&active, &function, &state(), &maps), Some(true));
        assert_eq!(evaluate(&inactive, &function, &state(), &maps), Some(false));
    }

    #[test]
    fn legacy_paths_from_the_catalogues_are_translated() {
        // все 23 формы, встречающиеся в Functions.xml / NewFunctions.xml
        let paths = [
            "Overlays[{3}].ActiveInput",
            "Inputs[{0}].State",
            "Inputs[{0}].Muted",
            "Inputs[{0}].Solo",
            "Inputs[{0}].Volume",
            "Inputs[{0}].Audiobusses",
            "Inputs[{0}].Balance",
            "Inputs[{0}].GainDb",
            "Inputs[{0}].Elements[InputText/Index/{1}].Text",
            "Inputs[{0}].Elements[InputImage/Index/{1}].Image",
            "Inputs[{0}].Elements[InputOverlay/Index/{3}].Key",
            "Audio[Master].Muted",
            "Audio[Bus{2}].Muted",
            "Audio[Bus{2}].Solo",
            "Audio[Bus{2}].SendToMaster",
            "Active",
            "Preview",
            "Recording",
            "Streaming",
            "External",
            "MultiCorder",
            "FadeToBlack",
            "Fullscreen",
        ];
        for path in paths {
            let xpath = legacy_xpath(path).unwrap_or_else(|| panic!("{path} не переведён"));
            assert!(xpath.starts_with("//"), "{path} → {xpath}");
        }
        assert_eq!(
            legacy_xpath("Overlays[{3}].ActiveInput").unwrap(),
            "//overlays/overlay[@number='{1}']"
        );
        assert_eq!(
            legacy_xpath("Inputs[{0}].Elements[InputText/Index/{1}].Text").unwrap(),
            "//inputs/input[@key='{0}']/text[@index='{1}']"
        );
        assert_eq!(
            legacy_xpath("Inputs[{0}].Elements[InputOverlay/Index/{3}].Key").unwrap(),
            "//inputs/input[@key='{0}']/overlay[@index='{3}']/@key"
        );
        assert_eq!(
            legacy_xpath("Audio[Bus{2}].SendToMaster").unwrap(),
            "//audio/bus{2}/@sendToMaster"
        );
        assert_eq!(legacy_xpath("Recording").unwrap(), "//recording");
        assert_eq!(legacy_xpath("MultiCorder").unwrap(), "//multiCorder");
        assert_eq!(legacy_xpath("Неизвестный.Путь"), None);
    }

    #[test]
    fn legacy_overlay_path_lights_the_button() {
        // ровно то, что лежит в Scoreboard.vmc: только ActiveStatePath, без XPath
        let maps = InputMaps::from_state(&state());
        let function = FunctionRef {
            function: "OverlayInputX".into(),
            active_state_path: "Overlays[{3}].ActiveInput".into(),
            active_state_value: "{0}".into(),
            ..Default::default()
        };
        let on_air = Command {
            function: "OverlayInputX".into(),
            input_key: Some("keyA".into()),
            input: Some(-1),
            parameter: Some("1".into()),
            use_in_active_state: true,
            ..Default::default()
        };
        let another = Command {
            function: "OverlayInputX".into(),
            input_key: Some("keyB".into()),
            input: Some(-1),
            parameter: Some("1".into()),
            use_in_active_state: true,
            ..Default::default()
        };
        assert_eq!(evaluate(&on_air, &function, &state(), &maps), Some(true));
        assert_eq!(evaluate(&another, &function, &state(), &maps), Some(false));
    }

    #[test]
    fn commands_without_state_path_are_ignored() {
        let maps = InputMaps::from_state(&state());
        let function = FunctionRef {
            function: "Cut".into(),
            ..Default::default()
        };
        let command = Command::new("Cut");
        assert_eq!(evaluate(&command, &function, &state(), &maps), None);

        let mut disabled = Command::new("Cut");
        disabled.use_in_active_state = false;
        assert_eq!(evaluate(&disabled, &function, &state(), &maps), None);
    }
}
