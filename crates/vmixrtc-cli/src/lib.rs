//! Проверка переноса: сравнение `.vmc` «прочитали → записали → прочитали» и сверка
//! содержимого с каталогом функций.
//!
//! Это инструмент честной проверки «1 в 1»: он не верит на слово, а берёт реальные файлы
//! оригинала, прогоняет их через порт и сообщает, что потерялось или не распозналось.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use vmix_config::{Vmc, WidgetData, WidgetKind, WidgetPatch};
use vmix_functions::{Catalogue, NATIVE_FUNCTIONS};

/// Одна найденная проблема.
#[derive(Debug, Clone)]
pub struct Issue {
    pub file: String,
    pub message: String,
}

/// Итог проверки одного файла (или каталога).
#[derive(Debug, Default)]
pub struct Report {
    pub files: usize,
    pub widgets: usize,
    pub orders: usize,
    pub issues: Vec<Issue>,
    /// Сколько виджетов каждого типа встретилось.
    pub kinds: BTreeMap<String, usize>,
    /// Нативные функции, которые реально используются в файлах.
    pub native_used: BTreeMap<String, usize>,
    /// Нативные функции, которые порт ещё не исполняет.
    pub native_missing: BTreeMap<String, usize>,
    /// Виды провайдеров и сколько раз встретились.
    pub providers: BTreeMap<String, usize>,
}

impl Report {
    pub fn merge(&mut self, other: Report) {
        self.files += other.files;
        self.widgets += other.widgets;
        self.orders += other.orders;
        self.issues.extend(other.issues);
        for (key, value) in other.kinds {
            *self.kinds.entry(key).or_default() += value;
        }
        for (key, value) in other.native_used {
            *self.native_used.entry(key).or_default() += value;
        }
        for (key, value) in other.native_missing {
            *self.native_missing.entry(key).or_default() += value;
        }
        for (key, value) in other.providers {
            *self.providers.entry(key).or_default() += value;
        }
    }

    /// Короткая сводка для вывода.
    pub fn summary(&self) -> String {
        let mut text = format!(
            "файлов: {}, виджетов: {}, команд: {}, проблем: {}\n",
            self.files,
            self.widgets,
            self.orders,
            self.issues.len()
        );
        text.push_str("  типы виджетов: ");
        text.push_str(
            &self
                .kinds
                .iter()
                .map(|(kind, count)| format!("{kind}×{count}"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        text.push('\n');
        if !self.providers.is_empty() {
            text.push_str("  провайдеры: ");
            text.push_str(
                &self
                    .providers
                    .iter()
                    .map(|(kind, count)| format!("{kind}×{count}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            text.push('\n');
        }
        if !self.native_used.is_empty() {
            text.push_str("  нативные функции в файлах: ");
            text.push_str(
                &self
                    .native_used
                    .iter()
                    .map(|(name, count)| format!("{name}×{count}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            text.push('\n');
        }
        if !self.native_missing.is_empty() {
            text.push_str("  ⚠ ещё не исполняются: ");
            text.push_str(
                &self
                    .native_missing
                    .iter()
                    .map(|(name, count)| format!("{name}×{count}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            text.push('\n');
        }
        text
    }
}

/// Отпечаток виджета: всё, что порт обязан сохранить при чтении и записи.
fn fingerprint(widget: &WidgetData) -> String {
    let commands = widget
        .commands
        .iter()
        .map(|command| {
            format!(
                "{}|{:?}|{:?}|{:?}|{:?}|{}",
                command.function,
                command.input,
                command.input_key,
                command.parameter,
                command.string_parameter,
                command.executable
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    let extras = widget
        .extras
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",");
    let geometry = format!(
        "{:.1},{:.1},{:.1},{:.1}",
        widget.left, widget.top, widget.width, widget.height
    );
    [
        widget.kind.type_name(),
        widget.name.clone(),
        geometry,
        format!("z{}", widget.z_index),
        format!("p{}", widget.page),
        widget.text.clone(),
        widget.color.to_argb(),
        widget.border_color.to_argb(),
        commands,
        extras,
    ]
    .join("|")
}

/// Проверить один файл.
pub fn verify_file(path: &Path, catalogue: &Catalogue) -> Result<Report> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut report = Report {
        files: 1,
        ..Default::default()
    };

    let bytes = std::fs::read(path).with_context(|| format!("чтение {}", path.display()))?;
    let original = Vmc::parse(&bytes).with_context(|| format!("разбор {name}"))?;

    // 1. «прочитали → записали → прочитали»
    let written = original.to_bytes();
    let reloaded = Vmc::parse(&written).with_context(|| format!("повторный разбор {name}"))?;

    let before: Vec<String> = original.widgets_data().iter().map(fingerprint).collect();
    let after: Vec<String> = reloaded.widgets_data().iter().map(fingerprint).collect();
    if before != after {
        for (position, (left, right)) in before.iter().zip(after.iter()).enumerate() {
            if left != right {
                report.issues.push(Issue {
                    file: name.clone(),
                    message: format!("виджет #{position} изменился при записи:\n      было: {left}\n      стало: {right}"),
                });
            }
        }
        if before.len() != after.len() {
            report.issues.push(Issue {
                file: name.clone(),
                message: format!(
                    "число виджетов изменилось: {} → {}",
                    before.len(),
                    after.len()
                ),
            });
        }
    }

    // 2. настройки окна и глобальные переменные
    let describe = |vmc: &Vmc| {
        vmc.window_settings()
            .map(|settings| {
                format!(
                    "{:?}|{:?}|{}",
                    settings.ip, settings.port, settings.locked
                )
            })
            .unwrap_or_else(|| "нет настроек".to_string())
    };
    let settings_before = describe(&original);
    let settings_after = describe(&reloaded);
    if settings_before != settings_after {
        report.issues.push(Issue {
            file: name.clone(),
            message: format!("настройки окна изменились: {settings_before} → {settings_after}"),
        });
    }
    if original.global_variables() != reloaded.global_variables() {
        report.issues.push(Issue {
            file: name.clone(),
            message: "глобальные переменные изменились при записи".to_string(),
        });
    }

    // 3. типы виджетов, функции и провайдеры
    for widget in reloaded.widgets_data() {
        report.widgets += 1;
        *report
            .kinds
            .entry(widget.kind.label().to_string())
            .or_default() += 1;
        if let WidgetKind::Other(type_name) = &widget.kind {
            report.issues.push(Issue {
                file: name.clone(),
                message: format!("неизвестный тип виджета: {type_name}"),
            });
        }

        // команды должны находиться в каталоге
        let modern = matches!(widget.kind, WidgetKind::NewButton);
        for command in &widget.commands {
            if command.function.trim().is_empty() {
                continue;
            }
            report.orders += 1;
            let known = catalogue
                .find(&command.function)
                .map(|function| function.native || !function.format_string.is_empty())
                .unwrap_or(false);
            let native = NATIVE_FUNCTIONS.contains(&command.function.as_str());
            if !known && !native {
                report.issues.push(Issue {
                    file: name.clone(),
                    message: format!(
                        "функция «{}» не найдена в каталоге (виджет «{}»)",
                        command.function, widget.name
                    ),
                });
            }
            if native {
                *report.native_used.entry(command.function.clone()).or_default() += 1;
                if !IMPLEMENTED_NATIVE.contains(&command.function.as_str()) {
                    *report
                        .native_missing
                        .entry(command.function.clone())
                        .or_default() += 1;
                }
            }
            let _ = modern;
        }

        // провайдеры внешних данных
        if let Some(external) = &widget.external {
            if !external.provider_path.trim().is_empty() {
                let kind = vmix_providers::ProviderKind::from_path(&external.provider_path);
                *report
                    .providers
                    .entry(kind.label().to_string())
                    .or_default() += 1;
                if !kind.is_supported() {
                    report.issues.push(Issue {
                        file: name.clone(),
                        message: format!(
                            "провайдер «{}» пока не поддержан (виджет «{}»)",
                            kind.label(),
                            widget.name
                        ),
                    });
                }
            }
        }
    }

    // 4. простая правка не должна ломать файл
    let mut mutable = original.clone();
    if let Some(first) = mutable.widgets_data().first() {
        let index = first.index;
        let left = first.left;
        mutable
            .update_widget(
                index,
                &WidgetPatch {
                    left: Some(left + 1.0),
                    ..Default::default()
                },
            )
            .with_context(|| format!("правка виджета в {name}"))?;
        let patched = Vmc::parse(&mutable.to_bytes())
            .with_context(|| format!("чтение после правки {name}"))?;
        let moved = patched
            .widgets_data()
            .into_iter()
            .find(|widget| widget.index == index)
            .map(|widget| widget.left);
        if moved != Some(left + 1.0) {
            report.issues.push(Issue {
                file: name.clone(),
                message: format!("правка координат не сохранилась: {moved:?}"),
            });
        }
    }

    Ok(report)
}

/// Нативные функции, которые порт уже исполняет (см. `vmix-script`).
pub const IMPLEMENTED_NATIVE: &[&str] = &[
    "API",
    "APIPOST",
    "Condition",
    "ConditionEnd",
    "Delay",
    "Else",
    "ExecLink",
    "GoTo",
    "HasVariable",
    "IsPressed",
    "NextPage",
    "PrevPage",
    "SetPage",
    "SetGlobalVariable",
    "SetVariable",
    "Timer",
    "ValueChanged",
];

/// Проверить все `.vmc` в каталоге.
pub fn verify_dir(dir: &Path, catalogue: &Catalogue) -> Result<Report> {
    let mut report = Report::default();
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("чтение каталога {}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .map(|extension| extension.eq_ignore_ascii_case("vmc"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    for file in files {
        report.merge(verify_file(&file, catalogue)?);
    }
    Ok(report)
}
