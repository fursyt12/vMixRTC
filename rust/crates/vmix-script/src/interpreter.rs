//! Интерпретатор команд кнопки — порт `vMixControlButton.ExecutionThread`.
//!
//! Управление: стек условий (`Condition`/`Else`/`ConditionEnd`), переходы (`GoTo`),
//! задержки (`Timer`/`Delay`). Нативные функции UTC исполняются здесь, остальные
//! превращаются в запрос к vMix.
//!
//! Реализовано: `Condition`, `Else`, `ConditionEnd`, `IsPressed`, `HasVariable`,
//! `SetVariable`, `SetGlobalVariable`, `ValueChanged`, `Timer`, `Delay`, `GoTo`,
//! `API`, `APIPOST`, `NextPage`, `PrevPage`, `SetPage`.
//! Прочие нативные (`Win`, `ExecLink`, `LIVE*`, `Sync*`, `SetButtonColor`) пока
//! отмечаются в журнале и не исполняются.

use crate::expression::{evaluate, format_number, ExpressionContext, Value};
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::time::Duration;
use vmix_config::WidgetData;
use vmix_functions::{xpath_value, Catalogue, Command};
use vmix_xml::Node;

/// Максимум прыжков — защита от зацикленного скрипта (в оригинале есть свой счётчик).
const MAX_JUMPS: usize = 1000;

use vmix_functions::NATIVE_FUNCTIONS;

/// Куда отправлять запросы.
pub trait FunctionSender {
    /// Запрос к vMix API (`Function=…&Input=…`).
    fn send_query(&self, query: &str) -> Result<String>;

    /// `API`/`APIPOST` — произвольный URL.
    fn fetch_url(&self, url: &str, _post: bool) -> Result<String> {
        Err(anyhow!("внешний запрос не поддержан: {url}"))
    }
}

#[derive(Debug, Default, Clone)]
pub struct ScriptOutcome {
    pub log: Vec<String>,
    pub page_delta: i32,
    pub page: Option<usize>,
    /// Изменённые глобальные переменные — их применяет вызывающая сторона.
    pub globals: Vec<(String, String)>,
    /// Ссылки, запрошенные `ExecLink`: вызывающая сторона запускает их после скрипта.
    pub links: Vec<String>,
}

pub struct ScriptRunner<'a> {
    pub commands: &'a [Command],
    pub catalogue: Option<&'a Catalogue>,
    pub state: Option<&'a Node>,
    /// Локальные переменные виджета (`_varN`).
    pub locals: BTreeMap<String, Value>,
    /// Глобальные переменные контроллера.
    pub globals: BTreeMap<String, Value>,
    /// Значения для `ValueChanged`.
    pub tracked: BTreeMap<String, Value>,
    pub is_pushed: bool,
    pub input_key: String,
    /// В тестах — false, чтобы `Timer` не спал по-настоящему.
    pub allow_sleep: bool,
}

impl<'a> ScriptRunner<'a> {
    pub fn new(commands: &'a [Command], state: Option<&'a Node>) -> Self {
        Self {
            commands,
            catalogue: None,
            state,
            locals: BTreeMap::new(),
            globals: BTreeMap::new(),
            tracked: BTreeMap::new(),
            is_pushed: false,
            input_key: String::new(),
            allow_sleep: true,
        }
    }

    pub fn with_catalogue(mut self, catalogue: &'a Catalogue) -> Self {
        self.catalogue = Some(catalogue);
        self
    }

    pub fn run(&mut self, sender: &dyn FunctionSender) -> Result<ScriptOutcome> {
        let mut outcome = ScriptOutcome::default();
        let mut conditions: Vec<Option<bool>> = Vec::new();
        let mut pointer: i64 = 0;
        let mut jumps = 0usize;

        while pointer >= 0 && (pointer as usize) < self.commands.len() {
            let index = pointer as usize;
            pointer += 1;
            let command = self.commands[index].clone();
            if !command.executable {
                continue;
            }

            let enclosing = conditions.last().copied().unwrap_or(Some(true));
            // функции управления условиями исполняются всегда (как в оригинале)
            let control = matches!(
                command.function.as_str(),
                "ConditionEnd" | "Condition" | "HasVariable" | "IsPressed" | "Else"
            );
            if !(enclosing.unwrap_or(false) || control) {
                continue;
            }

            match command.function.as_str() {
                "NextPage" => {
                    outcome.page_delta += 1;
                    outcome.log.push("NextPage → следующая страница".into());
                }
                "PrevPage" => {
                    outcome.page_delta -= 1;
                    outcome.log.push("PrevPage → предыдущая страница".into());
                }
                "SetPage" => {
                    let page = self.eval_number(command.parameter.as_deref().unwrap_or(""));
                    outcome.page = Some(page.max(0.0) as usize);
                    outcome.log.push(format!("SetPage → страница {}", page as i64));
                }
                "Condition" => {
                    let result = if enclosing.unwrap_or(false) {
                        Some(self.test_condition(&command))
                    } else {
                        None
                    };
                    outcome
                        .log
                        .push(format!("Condition → {}", describe_condition(result)));
                    conditions.push(result);
                }
                "Else" => {
                    let flipped = conditions.pop().flatten().map(|value| !value);
                    outcome
                        .log
                        .push(format!("Else → {}", describe_condition(flipped)));
                    conditions.push(flipped);
                }
                "ConditionEnd" => {
                    conditions.pop();
                    outcome.log.push("ConditionEnd".into());
                }
                "IsPressed" => {
                    conditions.push(Some(self.is_pushed));
                    outcome
                        .log
                        .push(format!("IsPressed → {}", self.is_pushed));
                }
                "HasVariable" => {
                    let result = if enclosing.unwrap_or(false) {
                        let key = format_number(
                            self.eval_number(command.parameter.as_deref().unwrap_or("")),
                        );
                        Some(self.locals.contains_key(&key) || self.globals.contains_key(&key))
                    } else {
                        None
                    };
                    outcome
                        .log
                        .push(format!("HasVariable → {}", describe_condition(result)));
                    conditions.push(result);
                }
                "SetVariable" => {
                    let name = format_number(
                        self.eval_number(command.parameter.as_deref().unwrap_or("")),
                    );
                    let value = self.object_parameter(&command);
                    outcome
                        .log
                        .push(format!("SetVariable {name} = {}", value.as_text()));
                    self.locals.insert(name, value);
                }
                "SetGlobalVariable" => {
                    let raw = command.parameter.clone().unwrap_or_default();
                    let value = self.object_parameter(&command);
                    let name = match self.evaluate_text(&raw) {
                        Value::Number(number) => format_number(number),
                        Value::Text(text) if !text.is_empty() => text,
                        other => other.as_text(),
                    };
                    let name = if name.is_empty() {
                        command
                            .string_parameter
                            .clone()
                            .unwrap_or_else(|| "global".to_string())
                    } else {
                        name
                    };
                    outcome
                        .log
                        .push(format!("SetGlobalVariable {name} = {}", value.as_text()));
                    self.globals.insert(name.clone(), value.clone());
                    outcome.globals.push((name, value.as_text()));
                }
                "ValueChanged" => {
                    let template = command.string_parameter.clone().unwrap_or_default();
                    let key = template.replace("{0}", &self.command_key(&command));
                    let value = self.object_parameter(&command);
                    let changed = self
                        .tracked
                        .get(&key)
                        .map(|previous| previous != &value)
                        .unwrap_or(false);
                    outcome
                        .log
                        .push(format!("ValueChanged {key} → {changed}"));
                    self.tracked.insert(key, value);
                    conditions.push(Some(changed));
                }
                "Timer" | "Delay" => {
                    let source = if command.function == "Delay" {
                        command.string_parameter.as_deref().unwrap_or("")
                    } else {
                        command.parameter.as_deref().unwrap_or("")
                    };
                    let milliseconds = self.eval_number(source).max(0.0) as u64;
                    outcome
                        .log
                        .push(format!("{} {milliseconds} мс", command.function));
                    if self.allow_sleep && milliseconds > 0 {
                        std::thread::sleep(Duration::from_millis(milliseconds));
                    }
                }
                "GoTo" => {
                    let target = self.eval_number(command.parameter.as_deref().unwrap_or(""));
                    jumps += 1;
                    if jumps > MAX_JUMPS {
                        outcome
                            .log
                            .push("GoTo → слишком много переходов, остановлено".into());
                        break;
                    }
                    outcome.log.push(format!("GoTo → {}", target as i64));
                    pointer = target as i64 - 1;
                }
                "ExecLink" => {
                    // как в оригинале: параметр — имя ссылки, её запускает приложение
                    let link = self.object_parameter(&command).as_text();
                    if link.trim().is_empty() {
                        outcome.log.push("ExecLink → пустая ссылка".into());
                    } else {
                        outcome.log.push(format!("ExecLink → «{link}»"));
                        outcome.links.push(link);
                    }
                }
                "API" | "APIPOST" => {
                    let path = self.object_parameter(&command).as_text();
                    let url = format!("http://{path}");
                    let post = command.function == "APIPOST";
                    match sender.fetch_url(&url, post) {
                        Ok(_) => outcome.log.push(format!("{} {url} → ок", command.function)),
                        Err(error) => outcome
                            .log
                            .push(format!("{} {url} → {error}", command.function)),
                    }
                }
                _ => {
                    if self.is_native(&command.function) {
                        outcome.log.push(format!(
                            "{} → нативная функция UTC, пока не поддержана",
                            command.function
                        ));
                    } else {
                        let resolved = self.resolve(&command);
                        match resolved
                            .render(self.catalogue)
                            .and_then(|query| sender.send_query(&query).map(|answer| (query, answer)))
                        {
                            Ok((query, answer)) => {
                                let answer = answer.trim();
                                let answer = if answer.is_empty() {
                                    "ок".to_string()
                                } else {
                                    answer.lines().next().unwrap_or("ок").to_string()
                                };
                                outcome.log.push(format!("{query} → {answer}"));
                            }
                            // как в оригинале: сбой одной команды не рвёт весь скрипт
                            Err(error) => outcome
                                .log
                                .push(format!("{} → ошибка: {error:#}", command.function)),
                        }
                    }
                }
            }
        }

        Ok(outcome)
    }

    /// Ключ входа команды (в оригинале везде `cmd.InputKey`), с запасным значением раннера.
    fn command_key(&self, command: &Command) -> String {
        command
            .input_key
            .clone()
            .filter(|key| !key.is_empty())
            .unwrap_or_else(|| self.input_key.clone())
    }

    fn is_native(&self, function: &str) -> bool {
        NATIVE_FUNCTIONS.contains(&function)
            || self
                .catalogue
                .and_then(|catalogue| catalogue.find(function))
                .map(|function| function.native)
                .unwrap_or(false)
    }

    /// `TestCondition` из оригинала: сравнивает два выражения оператором
    /// из `AdditionalParameters[2]`.
    fn test_condition(&self, command: &Command) -> bool {
        let parameters = &command.additional_parameters;
        if parameters.len() < 4 {
            return false;
        }
        let key = self.command_key(command);
        let substitute = |template: &str| {
            template.replace("{0}", &key).replace("{1}", &parameters[0])
        };
        let left = evaluate(&substitute(&parameters[1]), self).unwrap_or(Value::Null);
        let right = evaluate(&substitute(&parameters[3]), self).unwrap_or(Value::Null);
        let operator = parameters[2].trim();

        let context = ConditionContext {
            base: self,
            left,
            right,
        };
        let expression = format!("_var65534 {operator} _var65535");
        evaluate(&expression, &context)
            .map(|value| value.as_bool())
            .unwrap_or(false)
    }

    /// Значение-«объект» команды: float, string или число — как `CalculateObjectParameter`.
    /// Значение-«объект» команды — как `CalculateObjectParameter` в оригинале:
    /// вычисленный `StringParameter` (с подстановкой ключа входа), иначе — он же «как есть».
    pub fn object_parameter(&self, command: &Command) -> Value {
        let template = command.string_parameter.clone().unwrap_or_default();
        if template.trim().is_empty() {
            return Value::Null;
        }
        let expression = template.replace("{0}", &self.command_key(command));
        evaluate(&expression, self).unwrap_or_else(|_| Value::Text(template))
    }

    /// Подставить в команду вычисленные параметры перед отправкой в vMix.
    pub fn resolve(&self, command: &Command) -> Command {
        let mut resolved = command.clone();
        if let Some(parameter) = &command.parameter {
            resolved.parameter = Some(self.evaluate_text(parameter).as_text());
        }
        if let Some(value) = &command.string_parameter {
            resolved.string_parameter = Some(self.evaluate_text(value).as_text());
        }
        if let Some(value) = &command.float_parameter {
            resolved.float_parameter = Some(self.evaluate_text(value).as_text());
        }
        resolved
    }

    fn evaluate_text(&self, expression: &str) -> Value {
        if expression.trim().is_empty() {
            return Value::Null;
        }
        evaluate(expression, self).unwrap_or_else(|_| Value::Text(expression.to_string()))
    }

    fn eval_number(&self, expression: &str) -> f64 {
        self.evaluate_text(expression).as_number()
    }
}

fn describe_condition(result: Option<bool>) -> &'static str {
    match result {
        Some(true) => "истина",
        Some(false) => "ложь",
        None => "пропущено (внешний блок неактивен)",
    }
}

impl ExpressionContext for ScriptRunner<'_> {
    fn state_value(&self, path: &str) -> Option<String> {
        let state = self.state?;
        resolve_state_path(state, path, &self.input_key)
    }

    fn variable(&self, name: &str) -> Option<Value> {
        if let Some(index) = name.strip_prefix("_var") {
            if let Some(value) = self.locals.get(index) {
                return Some(value.clone());
            }
            if let Some(value) = self.globals.get(index) {
                return Some(value.clone());
            }
        }
        self.globals.get(name).cloned()
    }
}

/// Контекст для сравнения в `Condition`: добавляет псевдопеременные 65534/65535,
/// как это делает оригинал.
struct ConditionContext<'a> {
    base: &'a ScriptRunner<'a>,
    left: Value,
    right: Value,
}

impl ExpressionContext for ConditionContext<'_> {
    fn state_value(&self, path: &str) -> Option<String> {
        self.base.state_value(path)
    }

    fn variable(&self, name: &str) -> Option<Value> {
        match name {
            "_var65534" => Some(self.left.clone()),
            "_var65535" => Some(self.right.clone()),
            other => self.base.variable(other),
        }
    }
}

/// Прочитать значение состояния по «пути» из выражений (`_('Overlays[0].ActiveInput')`).
pub fn resolve_state_path(state: &Node, raw_path: &str, input_key: &str) -> Option<String> {
    let path = raw_path.replace("{0}", input_key);

    // Overlays[N].ActiveInput — индекс 0-based; в оригинале это свойство-объект,
    // поэтому сравнивать его можно с номером входа: отдаём номер, а не ключ.
    if path.ends_with("].ActiveInput") {
        if let Some(index) = path
            .strip_prefix("Overlays[")
            .and_then(|rest| rest.split(']').next())
            .and_then(|value| value.trim().parse::<usize>().ok())
        {
            let key = state
                .path(&["overlays"])
                .and_then(|overlays| overlays.children_named("overlay").nth(index))
                .map(|overlay| overlay.text.clone())
                .unwrap_or_default();
            if key.is_empty() {
                return Some(String::new());
            }
            return input_by_key(state, &key)
                .and_then(|input| input.attr("number"))
                .map(str::to_string);
        }
    }

    if let Some(rest) = path.strip_prefix("Inputs[") {
        let (key, tail) = rest.split_once(']')?;
        let input = input_by_key(state, key)?;
        let tail = tail.trim_start_matches('.');

        let attribute = match tail {
            "Number" => "@number",
            "State" => "@state",
            "Muted" => "@muted",
            "Solo" => "@solo",
            "Volume" => "@volume",
            "Audiobusses" => "@audiobusses",
            "Title" => "@title",
            "Type" => "@type",
            _ => "",
        };
        if !attribute.is_empty() {
            return input.attr(&attribute[1..]).map(str::to_string);
        }

        if let Some(elements) = tail.strip_prefix("Elements[") {
            let (element, field) = elements.split_once(']')?;
            let index: i64 = element.split('/').last()?.trim().parse().ok()?;
            let field = field.trim_start_matches('.');
            let collection = match field {
                "Text" => "text",
                "Image" => "image",
                "Key" => "overlay",
                _ => return None,
            };
            let by_index = input
                .children_named(collection)
                .find(|node| node.attr("index") == Some(index.to_string().as_str()))
                .map(|node| node.text.clone());
            return by_index.or_else(|| {
                input
                    .children_named(collection)
                    .nth(index.saturating_sub(1) as usize)
                    .map(|node| node.text.clone())
            });
        }
    }

    // корневые скаляры: Recording, Streaming, Active, Preview, …
    let mut name = path.chars().next()?.to_lowercase().to_string();
    name.push_str(&path[1..]);
    state.child_text(&name).map(str::to_string)
}

fn input_by_key<'a>(state: &'a Node, key: &str) -> Option<&'a Node> {
    state
        .path(&["inputs"])?
        .children_named("input")
        .find(|input| input.attr("key") == Some(key))
}

/// Значение состояния по XPath (для путей из `ActiveStateXPath`).
pub fn state_by_xpath(state: &Node, xpath: &str) -> Option<String> {
    xpath_value(state, xpath)
}

/// Команды виджета → модель скриптового слоя.
pub fn commands_of_widget(widget: &WidgetData) -> Vec<Command> {
    widget
        .commands
        .iter()
        .map(|command| Command {
            function: command.function.clone(),
            input: command.input,
            input_key: command.input_key.clone(),
            parameter: command.parameter.clone(),
            string_parameter: command.string_parameter.clone(),
            float_parameter: command.float_parameter.clone(),
            mix: command.mix.clone(),
            channel: command.channel.clone(),
            additional_parameters: command.additional_parameters.clone(),
            collapsed: command.collapsed,
            executable: command.executable,
            use_in_active_state: command.use_in_active_state,
            format_string: Some(command.format_string.clone())
                .filter(|format| !format.is_empty()),
            description: command.description.clone(),
            native: command.native,
            active_state_path: command.active_state_path.clone(),
            active_state_xpath: command.active_state_xpath.clone(),
            active_state_value: command.active_state_value.clone(),
            active_state_xpath_int_dependence: command
                .active_state_xpath_int_dependence
                .clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use vmix_config::Vmc;

    #[derive(Default)]
    struct Recorder {
        queries: RefCell<Vec<String>>,
        urls: RefCell<Vec<(String, bool)>>,
        answer: String,
    }

    impl FunctionSender for Recorder {
        fn send_query(&self, query: &str) -> Result<String> {
            self.queries.borrow_mut().push(query.to_string());
            Ok(self.answer.clone())
        }

        fn fetch_url(&self, url: &str, post: bool) -> Result<String> {
            self.urls.borrow_mut().push((url.to_string(), post));
            Ok(String::new())
        }
    }

    fn command(function: &str) -> Command {
        Command {
            function: function.to_string(),
            executable: true,
            use_in_active_state: true,
            // в .vmc сигнатура функции лежит внутри команды
            format_string: Some(format!("Function={function}")),
            ..Default::default()
        }
    }

    #[test]
    fn conditions_pick_the_right_branch() {
        let commands = vec![
            Command {
                additional_parameters: vec![
                    String::new(),
                    "_('Recording')".into(),
                    "=".into(),
                    "'True'".into(),
                ],
                ..command("Condition")
            },
            command("StartRecording"),
            command("Else"),
            command("StopRecording"),
            command("ConditionEnd"),
        ];

        let state = vmix_xml::parse(b"<vmix><recording>True</recording></vmix>").unwrap();
        let sender = Recorder::default();
        let mut runner = ScriptRunner::new(&commands, Some(&state));
        runner.allow_sleep = false;
        let outcome = runner.run(&sender).unwrap();

        assert_eq!(
            sender.queries.borrow().as_slice(),
            ["Function=StartRecording".to_string()],
            "должна была выполниться ветка «тогда»: {:?}",
            outcome.log
        );
        assert!(outcome.log.iter().any(|line| line.contains("Condition → истина")));
    }

    #[test]
    fn else_branch_runs_when_condition_is_false() {
        let commands = vec![
            Command {
                additional_parameters: vec![
                    String::new(),
                    "_('Recording')".into(),
                    "=".into(),
                    "'True'".into(),
                ],
                ..command("Condition")
            },
            command("StartRecording"),
            command("Else"),
            command("StopRecording"),
            command("ConditionEnd"),
        ];

        let state = vmix_xml::parse(b"<vmix><recording>False</recording></vmix>").unwrap();
        let sender = Recorder::default();
        let mut runner = ScriptRunner::new(&commands, Some(&state));
        runner.allow_sleep = false;
        runner.run(&sender).unwrap();

        assert_eq!(
            sender.queries.borrow().as_slice(),
            ["Function=StopRecording".to_string()]
        );
    }

    #[test]
    fn commands_after_condition_end_run_again() {
        let commands = vec![
            Command {
                additional_parameters: vec![
                    String::new(),
                    "'1'".into(),
                    "=".into(),
                    "'2'".into(),
                ],
                ..command("Condition")
            },
            command("StartRecording"),
            command("ConditionEnd"),
            command("StopRecording"),
        ];
        let sender = Recorder::default();
        let mut runner = ScriptRunner::new(&commands, None);
        runner.allow_sleep = false;
        runner.run(&sender).unwrap();
        assert_eq!(
            sender.queries.borrow().as_slice(),
            ["Function=StopRecording".to_string()]
        );
    }

    #[test]
    fn variables_are_set_and_visible_to_conditions() {
        let commands = vec![
            Command {
                parameter: Some("5".into()),
                string_parameter: Some("'привет'".into()),
                ..command("SetVariable")
            },
            Command {
                parameter: Some("5".into()),
                ..command("HasVariable")
            },
            command("StartRecording"),
            command("ConditionEnd"),
        ];
        let sender = Recorder::default();
        let mut runner = ScriptRunner::new(&commands, None);
        runner.allow_sleep = false;
        let outcome = runner.run(&sender).unwrap();
        assert_eq!(runner.locals.get("5"), Some(&Value::text("привет")));
        assert_eq!(sender.queries.borrow().len(), 1);
        assert!(outcome.log.iter().any(|line| line.contains("SetVariable 5")));
    }

    #[test]
    fn global_variables_are_reported_to_the_caller() {
        let commands = vec![Command {
            parameter: Some("'Score' ".trim().into()),
            string_parameter: Some("'2'".into()),
            ..command("SetGlobalVariable")
        }];
        let sender = Recorder::default();
        let mut runner = ScriptRunner::new(&commands, None);
        let outcome = runner.run(&sender).unwrap();
        assert_eq!(outcome.globals, vec![("Score".to_string(), "2".to_string())]);
    }

    #[test]
    fn goto_jumps_and_loop_guard_stops() {
        let commands = vec![
            command("StartRecording"),
            Command {
                parameter: Some("1".into()),
                ..command("GoTo")
            },
        ];
        let sender = Recorder::default();
        let mut runner = ScriptRunner::new(&commands, None);
        let outcome = runner.run(&sender).unwrap();
        assert!(outcome.log.iter().any(|line| line.contains("слишком много переходов")));
    }

    #[test]
    fn timer_sleeps_only_when_allowed() {
        let commands = vec![Command {
            parameter: Some("50".into()),
            ..command("Timer")
        }];
        let sender = Recorder::default();
        let mut runner = ScriptRunner::new(&commands, None);
        runner.allow_sleep = false;
        let started = std::time::Instant::now();
        runner.run(&sender).unwrap();
        assert!(started.elapsed().as_millis() < 40, "спать не должны были");
    }

    #[test]
    fn value_changed_fires_only_when_the_value_differs() {
        // Как в оригинале: ключ — это шаблон StringParameter, значение — он же, вычисленный.
        let commands = vec![
            Command {
                string_parameter: Some("_('Recording')".into()),
                ..command("ValueChanged")
            },
            command("StartRecording"),
            command("ConditionEnd"),
        ];
        let sender = Recorder::default();
        let state = vmix_xml::parse(b"<vmix><recording>True</recording></vmix>").unwrap();

        let mut runner = ScriptRunner::new(&commands, Some(&state));
        runner.run(&sender).unwrap();
        assert_eq!(
            sender.queries.borrow().len(),
            0,
            "первый раз значение только запоминается"
        );

        // то же значение — по-прежнему ничего
        sender.queries.borrow_mut().clear();
        runner.run(&sender).unwrap();
        assert_eq!(sender.queries.borrow().len(), 0);

        // состояние изменилось — ветка исполняется
        let changed_state = vmix_xml::parse(b"<vmix><recording>False</recording></vmix>").unwrap();
        runner.state = Some(&changed_state);
        runner.run(&sender).unwrap();
        assert_eq!(sender.queries.borrow().len(), 1);
    }

    #[test]
    fn exec_link_returns_links_to_the_caller() {
        let commands = vec![
            Command {
                string_parameter: Some("'Play.Execute'".into()),
                ..command("ExecLink")
            },
            command("Cut"),
        ];
        let sender = Recorder::default();
        let mut runner = ScriptRunner::new(&commands, None);
        runner.allow_sleep = false;
        let outcome = runner.run(&sender).unwrap();
        assert_eq!(outcome.links, vec!["Play.Execute".to_string()]);
        // сама ссылка в vMix не уходит, а команда после неё — уходит
        assert_eq!(
            sender.queries.borrow().as_slice(),
            ["Function=Cut".to_string()]
        );
    }

    #[test]
    fn api_command_uses_url_not_vmix() {
        let commands = vec![Command {
            string_parameter: Some("'example.com/hook'".into()),
            ..command("API")
        }];
        let sender = Recorder::default();
        let mut runner = ScriptRunner::new(&commands, None);
        runner.run(&sender).unwrap();
        assert_eq!(
            sender.urls.borrow().as_slice(),
            [("http://example.com/hook".to_string(), false)]
        );
        assert!(sender.queries.borrow().is_empty());
    }

    #[test]
    fn overlay_active_input_resolves_to_input_number() {
        let state = vmix_xml::parse(
            br#"<vmix><inputs><input key="keyA" number="5" type="GT" title="Top"/></inputs>
<overlays><overlay number="1">keyA</overlay><overlay number="2"/></overlays></vmix>"#,
        )
        .unwrap();
        assert_eq!(
            resolve_state_path(&state, "Overlays[0].ActiveInput", "").as_deref(),
            Some("5")
        );
        assert_eq!(
            resolve_state_path(&state, "Overlays[1].ActiveInput", "").as_deref(),
            Some("")
        );
        assert_eq!(
            resolve_state_path(&state, "Inputs[keyA].Number", "").as_deref(),
            Some("5")
        );
        assert_eq!(
            resolve_state_path(&state, "Inputs[keyA].Elements[1].Text", ""),
            None
        );
    }

    #[test]
    fn unsupported_native_functions_are_logged_not_sent() {
        // `ExecLink` уже реализован, поэтому берём те, что пока не исполняются
        let commands = vec![command("Win"), command("LIVEOn")];
        let sender = Recorder::default();
        let mut runner = ScriptRunner::new(&commands, None);
        let outcome = runner.run(&sender).unwrap();
        assert!(sender.queries.borrow().is_empty());
        assert_eq!(
            outcome
                .log
                .iter()
                .filter(|line| line.contains("нативная функция"))
                .count(),
            2,
            "{:?}",
            outcome.log
        );
    }

    /// Реальный скрипт из Scoreboard.vmc: условие по оверлею, Timer и ветка Else.
    #[test]
    fn real_switch_top_scoreboard_script() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/Scoreboard.vmc");
        let vmc = Vmc::parse(&std::fs::read(&path).unwrap()).unwrap();
        let widget = vmc
            .widgets_data()
            .into_iter()
            .find(|widget| {
                widget
                    .commands
                    .iter()
                    .any(|command| command.function == "Condition")
            })
            .expect("в Scoreboard.vmc есть кнопка с условием");
        let commands = commands_of_widget(&widget);
        assert!(commands.iter().any(|command| command.function == "Timer"));

        // состояние: оверлей 1 занят другим входом → ветка «тогда»
        let state_xml = r#"<vmix><inputs>
<input key="e22bcc58-40e1-4770-9559-5ab1614e6574" number="3" type="GT" title="Top"/>
</inputs><overlays><overlay number="1">другой</overlay></overlays></vmix>"#;
        let state = vmix_xml::parse(state_xml.as_bytes()).unwrap();

        let sender = Recorder::default();
        let mut runner = ScriptRunner::new(&commands, Some(&state));
        runner.allow_sleep = false;
        let outcome = runner.run(&sender).unwrap();

        let queries = sender.queries.borrow().clone();
        assert!(
            queries.iter().any(|query| query.contains("OverlayInput1Out")),
            "ветка «тогда» должна выключить оверлей: {queries:?} / {:?}",
            outcome.log
        );
        assert!(
            outcome.log.iter().any(|line| line.contains("Timer")),
            "Timer должен отработать: {:?}",
            outcome.log
        );
    }
}
