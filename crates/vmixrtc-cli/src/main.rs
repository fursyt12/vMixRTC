//! `vmixrtc-cli` — CLI для проверки ядра порта без UI.
//!
//! Примеры:
//!   vmixrtc-cli status --host 127.0.0.1 --port 8088
//!   vmixrtc-cli call Cut --param Input=1
//!   vmixrtc-cli vmc examples/Scoreboard.vmc

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use vmix_api::{VmixClient, DEFAULT_PORT};
use vmix_config::Vmc;
use vmix_functions::Catalogue;
use vmix_script::{commands_of_widget, FunctionSender, ScriptRunner};

#[derive(Parser, Debug)]
#[command(
    name = "vmixrtc-cli",
    about = "Ядро Rust-порта vMixUTC: состояние vMix, функции, разбор .vmc",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Показать состояние vMix (входы, миксы, аудио, флаги).
    Status {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        login: Option<String>,
        #[arg(long)]
        password: Option<String>,
    },

    /// Вызвать функцию vMix: `vmixrtc-cli call Cut --param Input=1`.
    Call {
        function: String,
        #[arg(long = "param", value_parser = parse_kv)]
        params: Vec<(String, String)>,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        login: Option<String>,
        #[arg(long)]
        password: Option<String>,
        /// Только показать URL, не отправляя запрос.
        #[arg(long)]
        dry_run: bool,
    },

    /// Сводка по файлу контроллера: настройки окна, виджеты, глобальные переменные.
    Vmc { file: PathBuf },

    /// Создать демонстрационный контроллер: по одному виджету каждого типа палитры.
    Demo {
        file: PathBuf,
        #[arg(long, default_value = "vMixUTC")]
        title: String,
        /// Взять только один тип виджета (например `container`).
        #[arg(long)]
        kind: Option<String>,
    },

    /// Проверить перенос: прочитать, записать, прочитать и сверить с каталогом.
    Verify {
        /// Каталог с `.vmc` (по умолчанию — примеры оригинала).
        #[arg(default_value = "../examples")]
        dir: PathBuf,
        /// Показывать каждую проблему.
        #[arg(long)]
        verbose: bool,
    },

    /// Импортировать контроллер внутрь виджета-контейнера.
    Container {
        file: PathBuf,
        index: usize,
        source: PathBuf,
    },

    /// Прогнать скрипт виджета (как нажатие кнопки) и показать журнал.
    Press {
        file: PathBuf,
        index: usize,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Не отправлять запросы в vMix, только показать, что было бы отправлено.
        #[arg(long)]
        dry_run: bool,
    },
}

/// Отправка запросов из скрипта — та же логика, что в приложении.
struct VmixSender<'a> {
    client: &'a VmixClient,
}

impl FunctionSender for VmixSender<'_> {
    fn send_query(&self, query: &str) -> Result<String> {
        self.client.send_query(query)
    }

    fn fetch_url(&self, url: &str, post: bool) -> Result<String> {
        self.client.fetch_url(url, post)
    }
}

/// Сендер для `--dry-run`: печатает запросы вместо отправки.
struct DryRunSender;

impl FunctionSender for DryRunSender {
    fn send_query(&self, query: &str) -> Result<String> {
        println!("    → {query}");
        Ok(String::new())
    }

    fn fetch_url(&self, url: &str, post: bool) -> Result<String> {
        println!("    → {} {url}", if post { "POST" } else { "GET" });
        Ok(String::new())
    }
}

fn parse_kv(raw: &str) -> Result<(String, String), String> {
    let (key, value) = raw
        .split_once('=')
        .ok_or_else(|| format!("ожидался формат КЛЮЧ=ЗНАЧЕНИЕ, получено «{raw}»"))?;
    Ok((key.to_string(), value.to_string()))
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Status {
            host,
            port,
            login,
            password,
        } => {
            let client = client(host, port, login, password);
            let state = client.state()?;
            println!(
                "vMix {} ({}) — входов: {}, активный: {:?}, превью: {:?}",
                state.version.as_deref().unwrap_or("?"),
                state.edition.as_deref().unwrap_or("?"),
                state.inputs.len(),
                state.active,
                state.preview
            );
            println!("{:<4} {:<10} {:<24} {:<10} VOL", "#", "ТИП", "НАЗВАНИЕ", "СОСТОЯНИЕ");
            for input in &state.inputs {
                println!(
                    "{:<4} {:<10} {:<24} {:<10} {}",
                    input.number.map(|n| n.to_string()).unwrap_or_else(|| "-".into()),
                    input.kind.as_deref().unwrap_or("?"),
                    truncate(&input.title, 24),
                    input.state.as_deref().unwrap_or("-"),
                    input
                        .volume
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".into())
                );
            }
            for mix in &state.mixes {
                println!(
                    "микс {}: активный {:?}, превью {:?}",
                    mix.number.map(|n| n.to_string()).unwrap_or_else(|| "-".into()),
                    mix.active,
                    mix.preview
                );
            }
            for bus in &state.audio {
                println!(
                    "аудио {}: громкость {:?}, mute {:?}",
                    bus.name, bus.volume, bus.muted
                );
            }
            println!(
                "запись: {}, стрим: {}, внешний: {}, плейлист: {}, multicorder: {}, fadeToBlack: {}",
                state.recording,
                state.streaming,
                state.external,
                state.playlist,
                state.multicorder,
                state.fade_to_black
            );
        }

        Command::Call {
            function,
            params,
            host,
            port,
            login,
            password,
            dry_run,
        } => {
            let client = client(host, port, login, password);
            let params: Vec<(&str, &str)> = params
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let url = client.function_url(&function, &params);
            if dry_run {
                println!("{url}");
            } else {
                let response = client.send_function(&function, &params)?;
                println!("{url} -> {}", response.trim());
            }
        }

        Command::Demo { file, title, kind } => {
            let mut vmc = Vmc::empty();
            let mut palette: Vec<_> = vmix_config::WidgetKind::PALETTE.to_vec();
            if let Some(kind) = &kind {
                let wanted = kind.to_ascii_lowercase();
                palette.retain(|item| {
                    item.type_name().to_ascii_lowercase().contains(&wanted)
                        || item.label().to_ascii_lowercase().contains(&wanted)
                });
                if palette.is_empty() {
                    return Err(anyhow::anyhow!("неизвестный тип виджета: {kind}"));
                }
            }

            // раскладываем виджеты по сетке, чтобы всё было видно
            let mut column = 0usize;
            let mut row = 0usize;
            let mut index;
            for kind in &palette {
                let left = 12.0 + column as f64 * 250.0;
                let top = 12.0 + row as f64 * 110.0;
                index = vmc.create_widget(kind, left, top)?;
                match kind {
                    vmix_config::WidgetKind::Volume => {
                        vmc.set_widget_extra(index, "Target", "Input")?;
                        vmc.set_widget_extra(index, "InputKey", "n1")?;
                        vmc.set_widget_extra(index, "ShowSlider", "true")?;
                    }
                    vmix_config::WidgetKind::TBar => {
                        vmc.set_widget_extra(index, "Mode", "Fader")?;
                        vmc.set_widget_extra(index, "Style", "Horizontal")?;
                    }
                    vmix_config::WidgetKind::Clock => {
                        vmc.set_widget_extra(index, "ShowSeconds", "true")?;
                        vmc.set_widget_extra(index, "ShowDate", "true")?;
                        // пример расписания: будни в 09:30 запускают ссылку
                        vmc.set_widget_events(
                            index,
                            &[vmix_config::ScheduledEvent {
                                time: "09:30".into(),
                                command: "Play.Execute".into(),
                                days: 0b0001_1111,
                            }],
                        )?;
                    }
                    vmix_config::WidgetKind::VariableViewer => {
                        vmc.set_widget_extra(index, "Variable", "@TestVariable1")?;
                        vmc.set_widget_extra(index, "ShowVariableName", "true")?;
                    }
                    _ => {}
                }
                column += 1;
                if column == 3 {
                    column = 0;
                    row += 1;
                }
            }

            vmc.set_global_variable("@TestVariable1", "4")?;
            vmc.set_global_variable("Отбивка", "Готово")?;
            let _ = title;
            std::fs::write(&file, vmc.to_bytes())
                .with_context(|| format!("запись {}", file.display()))?;
            println!(
                "создан {} — виджетов: {}",
                file.display(),
                vmc.widgets_data().len()
            );
        }

        Command::Verify { dir, verbose } => {
            let catalogue = Catalogue::discover().context("каталог функций")?;
            let report = vmixrtc_cli::verify_dir(&dir, &catalogue)?;
            print!("{}", report.summary());
            if verbose {
                for issue in &report.issues {
                    println!("  ✗ {}: {}", issue.file, issue.message);
                }
            }
            if !report.issues.is_empty() {
                println!("проблем: {}", report.issues.len());
            }
        }

        Command::Container {
            file,
            index,
            source,
        } => {
            let mut vmc = Vmc::parse(
                &std::fs::read(&file).with_context(|| format!("чтение {}", file.display()))?,
            )?;
            let bytes = std::fs::read(&source)
                .with_context(|| format!("чтение {}", source.display()))?;
            let count = vmc.import_into_container(index, &bytes)?;
            std::fs::write(&file, vmc.to_bytes())
                .with_context(|| format!("запись {}", file.display()))?;
            println!(
                "в контейнер #{index} импортировано виджетов: {count} (из {})",
                source.display()
            );
        }

        Command::Press {
            file,
            index,
            host,
            port,
            dry_run,
        } => {
            let bytes = std::fs::read(&file)
                .with_context(|| format!("чтение {}", file.display()))?;
            let vmc = Vmc::parse(&bytes)?;
            let widget = vmc
                .widgets_data()
                .into_iter()
                .find(|widget| widget.index == index)
                .ok_or_else(|| anyhow::anyhow!("виджета с индексом {index} нет"))?;
            let commands = commands_of_widget(&widget);
            println!(
                "виджет #{} «{}» — команд: {}",
                index,
                widget.name,
                commands.len()
            );

            let client = VmixClient::new(host, port);
            let state = if dry_run {
                None
            } else {
                client
                    .fetch_state_xml()
                    .ok()
                    .and_then(|xml| vmix_api::VmixState::parse(&xml).ok())
            };

            let catalogue = Catalogue::discover().ok();
            let mut runner = ScriptRunner::new(&commands, state.as_ref().map(|state| &state.raw));
            runner.catalogue = catalogue.as_ref();
            runner.is_pushed = widget.active;

            let outcome = if dry_run {
                runner.run(&DryRunSender)?
            } else {
                runner.run(&VmixSender { client: &client })?
            };

            for line in &outcome.log {
                println!("  {line}");
            }
            if outcome.page_delta != 0 || outcome.page.is_some() {
                println!(
                    "  страница: {}{}",
                    outcome.page.map(|page| page.to_string()).unwrap_or_else(|| "—".into()),
                    if outcome.page_delta == 0 {
                        String::new()
                    } else {
                        format!(" (сдвиг {})", outcome.page_delta)
                    }
                );
            }
            for (name, value) in &outcome.globals {
                println!("  глобальная переменная {name} = {value}");
            }
        }

        Command::Vmc { file } => {
            let bytes = std::fs::read(&file)
                .with_context(|| format!("чтение {}", file.display()))?;
            let vmc = Vmc::parse(&bytes)?;
            let widgets = vmc.widgets();

            if let Some(settings) = vmc.window_settings() {
                println!(
                    "окно: {}x{} @ {},{}, vMix {}:{}, infinite canvas: {}, locked: {}",
                    settings.width.unwrap_or(0.0),
                    settings.height.unwrap_or(0.0),
                    settings.left.unwrap_or(0.0),
                    settings.top.unwrap_or(0.0),
                    settings.ip.as_deref().unwrap_or("?"),
                    settings.port.as_deref().unwrap_or("?"),
                    settings.use_infinite_canvas,
                    settings.locked
                );
            }
            println!("виджетов: {}", widgets.len());
            for (kind, count) in vmc.widget_types() {
                println!("  {count:>3}  {kind}");
            }
            println!("имена:");
            for widget in widgets.iter().take(20) {
                println!(
                    "  {:<28} {}",
                    widget.name().unwrap_or("(без имени)"),
                    widget.type_name().unwrap_or("?")
                );
            }
            if widgets.len() > 20 {
                println!("  … и ещё {}", widgets.len() - 20);
            }
            let globals = vmc.global_variables();
            if !globals.is_empty() {
                println!("глобальные переменные:");
                for (key, value) in globals {
                    println!("  {key} = {value}");
                }
            }
        }
    }
    Ok(())
}

fn client(host: String, port: u16, login: Option<String>, password: Option<String>) -> VmixClient {
    let client = VmixClient::new(host, port);
    match (login, password) {
        (Some(login), Some(password)) => client.with_credentials(login, password),
        _ => client,
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}
