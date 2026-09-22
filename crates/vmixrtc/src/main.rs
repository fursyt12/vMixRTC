//! Тач-панель vMixUTC на Tauri: рабочая поверхность (виджеты `.vmc`) + управление vMix.
//!
//! Слой интерфейса тонкий: работа с vMix и `.vmc` живёт в `vmix-api` / `vmix-config`,
//! здесь только состояние приложения и IPC-команды. Фронтенд — статические файлы
//! (`app.withGlobalTauri = true`), сборщик не нужен.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use chrono::{Datelike, Timelike};

mod i18n;
use i18n::I18n;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;
use vmix_api::{VmixClient, VmixState, DEFAULT_PORT};
use vmix_config::{Vmc, WidgetCommand, WidgetData, WidgetKind, WidgetPatch};
use vmix_functions::{evaluate, Catalogue, Command as FunctionCommand, FunctionRef, InputMaps};
use vmix_midi::{dispatch as midi_dispatch, MidiMessage, MidiSource, TestSource};
use vmix_ndi::{Backend, MjpegServer, RuntimeBackend, TestBackend};
use vmix_providers::{
    ExcelDataProvider, GoogleSheetsDataProvider, JsonDataProvider, NdiDataProvider, ProviderKind,
    XmlDataProvider,
};
use vmix_script::{commands_of_widget, FunctionSender, ScriptOutcome, ScriptRunner, Value};
use vmix_streamdeck::{DeckEvent, DeckEventKind, DeckSource, TestSource as DeckTestSource};
use vmix_xml::Node;

/// Страницы по умолчанию — как в оригинале (`MainViewModel`, `Utils`).
const DEFAULT_PAGES: [&str; 7] = ["MAIN", "DATA", "PAGE1", "PAGE2", "PAGE3", "PAGE4", "PAGE5"];

// ------------------------------------------------------------ состояние

#[derive(Default)]
struct Document {
    path: Option<PathBuf>,
    vmc: Option<Vmc>,
}

#[derive(Default)]
struct AppState {
    document: Mutex<Document>,
    /// Путь из аргумента командной строки — как «Command line argument to load .vmc» в оригинале.
    startup_path: Option<PathBuf>,
    /// Каталог функций vMix загружается один раз (459 + 811 записей).
    catalogue: Mutex<Option<Catalogue>>,
    /// Виджет, который нужно выбрать после старта: `vmixrtc файл.vmc 3`.
    startup_select: Mutex<Option<usize>>,
    /// Рантайм скриптов: локальные переменные и отслеженные значения по виджетам.
    /// Ключ — `widget_key` (у вложенных виджетов контейнеров он свой).
    runtime: Mutex<HashMap<u64, WidgetRuntime>>,
    /// Открыть редактор скрипта после старта (`vmixrtc файл.vmc 12 script`).
    startup_script: Mutex<bool>,
    /// Открыть просмотр строк источника (`vmixrtc файл.vmc 2 rows`).
    startup_rows: Mutex<bool>,
    /// Сразу запустить MIDI-вход (`vmixrtc файл.vmc 0 midi`).
    startup_midi: Mutex<bool>,
    /// Сразу запустить Stream Deck (`vmixrtc файл.vmc 0 deck`).
    startup_deck: Mutex<bool>,
    /// Прокрутить панель свойств к расписанию (`vmixrtc файл.vmc 7 schedule`).
    startup_schedule: Mutex<bool>,
    /// Открыть меню поверхности при старте (`vmixrtc файл.vmc 0 palette`).
    startup_palette: Mutex<bool>,
    /// Dev: запрос для поиска функций при старте.
    startup_picker: Mutex<Option<String>>,
    /// Провайдеры внешних данных по виджетам.
    externals: Mutex<HashMap<usize, ExternalRuntime>>,
    /// MJPEG-потоки NDI по виджетам (в webview они идут в `<img>`).
    ndi: Mutex<HashMap<usize, MjpegServer>>,
    /// MIDI-вход (или тестовый источник, если устройств нет).
    midi: Mutex<MidiRuntime>,
    /// Stream Deck (или тестовый источник, если устройства нет).
    deck: Mutex<DeckRuntime>,
    /// Планировщик событий часов.
    schedule: Mutex<ScheduleState>,
    /// Язык интерфейса и сообщений.
    i18n: Mutex<I18n>,
}

/// Что уже сработало: сбрасывается при смене суток.
#[derive(Default)]
struct ScheduleState {
    day: String,
    fired: std::collections::HashSet<(usize, usize)>,
}

/// Какие события пора запускать. Чистая функция: время передаётся снаружи,
/// поэтому её можно проверить тестом (порт цикла `Timer_Tick` из `vMixControlClock`).
fn schedule_due(
    events: &[(usize, usize, vmix_config::ScheduledEvent)],
    state: &mut ScheduleState,
    date: &str,
    minutes: u32,
    weekday: chrono::Weekday,
    force: bool,
) -> Vec<(usize, usize, String)> {
    // новый день — забываем вчерашние срабатывания
    if state.day != date {
        state.day = date.to_string();
        state.fired.clear();
    }

    let mut due = Vec::new();
    for (widget, position, event) in events {
        let Some(planned) = event.minutes() else {
            continue;
        };
        if event.command.trim().is_empty() {
            continue;
        }
        let key = (*widget, *position);
        let already = state.fired.contains(&key);
        if event.runs_on(weekday) && (force || minutes >= planned) && !already {
            state.fired.insert(key);
            due.push((*widget, *position, event.command.clone()));
        }
    }
    due
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SchedulePoll {
    /// Сколько событий запущено этим тиком.
    fired: usize,
    log: Vec<String>,
}

/// Проверить расписание часов и запустить ссылки событий.
#[tauri::command]
fn schedule_poll(
    force: Option<bool>,
    connection: Connection,
    state: tauri::State<'_, AppState>,
) -> Result<SchedulePoll, String> {
    let events = {
        let document = state.document.lock().map_err(|e| e.to_string())?;
        document
            .vmc
            .as_ref()
            .map(|vmc| vmc.scheduled_events())
            .unwrap_or_default()
    };
    if events.is_empty() {
        return Ok(SchedulePoll {
            fired: 0,
            log: Vec::new(),
        });
    }

    let now = chrono::Local::now();
    let due = {
        let mut schedule = state.schedule.lock().map_err(|e| e.to_string())?;
        schedule_due(
            &events,
            &mut schedule,
            &now.format("%Y-%m-%d").to_string(),
            now.hour() * 60 + now.minute(),
            now.weekday(),
            force.unwrap_or(false),
        )
    };

    let mut log = Vec::new();
    if !due.is_empty() {
        let catalogue = ensure_catalogue(&state)?;
        let client = connection.client();
        let state_xml = client.fetch_state_xml().ok();
        let state_node = state_xml
            .as_deref()
            .and_then(|xml| vmix_xml::parse(xml.as_bytes()).ok());
        for (widget, position, command) in &due {
            log.push(tr(
                &state,
                "schedule.log",
                &[&widget.to_string(), &(position + 1).to_string()],
            ));
            log.extend(dispatch_link(
                &state,
                &client,
                &catalogue,
                state_node.as_ref(),
                command,
                None,
            ));
        }
    }

    Ok(SchedulePoll {
        fired: due.len(),
        log,
    })
}

/// Состояние Stream Deck.
#[derive(Default)]
struct DeckRuntime {
    source: Option<Box<dyn DeckSource>>,
    device: String,
    /// Последняя нажатая кнопка — для режима обучения.
    last: Option<DeckEvent>,
    brightness: u8,
}

/// Состояние MIDI-входа.
#[derive(Default)]
struct MidiRuntime {
    source: Option<Box<dyn MidiSource>>,
    device: String,
    /// Последнее сообщение — для режима обучения.
    last: Option<MidiMessage>,
}

/// Провайдер, собранный под конкретный виджет.
#[derive(Debug)]
enum RuntimeProvider {
    Xml(XmlDataProvider),
    Json(JsonDataProvider),
    Excel(ExcelDataProvider),
    GoogleSheets(GoogleSheetsDataProvider),
    Ndi(NdiDataProvider),
    Unsupported(ProviderKind),
}

#[derive(Debug, Default)]
struct ExternalRuntime {
    provider: Option<RuntimeProvider>,
    provider_path: String,
    last_refresh: Option<Instant>,
    values: Vec<String>,
    error: Option<String>,
}

impl ExternalRuntime {
    fn ensure(&mut self, external: &vmix_config::ExternalData) {
        if self.provider.is_some() && self.provider_path == external.provider_path {
            return;
        }
        self.provider_path = external.provider_path.clone();
        self.values.clear();
        self.error = None;
        let kind = ProviderKind::from_path(&external.provider_path);
        self.provider = Some(match kind {
            ProviderKind::Xml => {
                let mut provider = XmlDataProvider::from_properties(&external.provider_properties);
                provider.set_period_ms(external.period_ms.max(100) as u64);
                RuntimeProvider::Xml(provider)
            }
            ProviderKind::Json => {
                let mut provider = JsonDataProvider::from_properties(&external.provider_properties);
                provider.set_period_ms(external.period_ms.max(100) as u64);
                RuntimeProvider::Json(provider)
            }
            ProviderKind::Excel => {
                let mut provider = ExcelDataProvider::from_properties(&external.provider_properties);
                provider.set_period_ms(external.period_ms.max(100) as u64);
                RuntimeProvider::Excel(provider)
            }
            ProviderKind::GoogleSheets => {
                let mut provider =
                    GoogleSheetsDataProvider::from_properties(&external.provider_properties);
                if provider.period_ms == 0 {
                    provider.period_ms = 5000;
                }
                RuntimeProvider::GoogleSheets(provider)
            }
            ProviderKind::NdiMonitor => {
                let mut provider = NdiDataProvider::from_properties(&external.provider_properties);
                provider.set_period_ms(external.period_ms.max(100) as u64);
                RuntimeProvider::Ndi(provider)
            }
            other => RuntimeProvider::Unsupported(other),
        });
    }

    fn label(&self) -> String {
        if self.provider_path.is_empty() {
            // виджет-потребитель: значения приходят от источника по имени
            return "источник".to_string();
        }
        match &self.provider {
            Some(RuntimeProvider::Xml(_)) => "XML".to_string(),
            Some(RuntimeProvider::Json(_)) => "JSON".to_string(),
            Some(RuntimeProvider::Excel(_)) => "Excel".to_string(),
            Some(RuntimeProvider::GoogleSheets(_)) => "Google Sheets".to_string(),
            Some(RuntimeProvider::Ndi(_)) => "NDI".to_string(),
            Some(RuntimeProvider::Unsupported(kind)) => format!("{} (не поддержан)", kind.label()),
            None => "—".to_string(),
        }
    }

    fn refresh(&mut self, state: Option<&vmix_xml::Node>) {
        match &mut self.provider {
            Some(RuntimeProvider::Xml(provider)) => match provider.refresh() {
                Ok(()) => {
                    self.values = provider.values().to_vec();
                    self.error = provider.error().map(str::to_string);
                }
                Err(error) => self.error = Some(format!("{error:#}")),
            },
            Some(RuntimeProvider::Json(provider)) => match provider.refresh() {
                Ok(()) => {
                    self.values = provider.values().to_vec();
                    self.error = provider.error().map(str::to_string);
                }
                Err(error) => self.error = Some(format!("{error:#}")),
            },
            Some(RuntimeProvider::Excel(provider)) => match provider.refresh() {
                Ok(()) => {
                    self.values = provider.values().to_vec();
                    self.error = provider.error().map(str::to_string);
                }
                Err(error) => self.error = Some(format!("{error:#}")),
            },
            Some(RuntimeProvider::GoogleSheets(provider)) => match provider.refresh() {
                Ok(()) => {
                    self.values = provider.values().to_vec();
                    self.error = provider.error().map(str::to_string);
                }
                Err(error) => self.error = Some(format!("{error:#}")),
            },
            Some(RuntimeProvider::Ndi(provider)) => {
                match provider.refresh_with_state(state) {
                    Ok(()) => {
                        self.values = provider.values().to_vec();
                        self.error = provider.error().map(str::to_string);
                    }
                    Err(error) => self.error = Some(format!("{error:#}")),
                }
            }
            Some(RuntimeProvider::Unsupported(_)) => {
                self.error = Some("провайдер из .vmc — .NET-сборка, порт её не исполняет".into())
            }
            None => {}
        }
        self.last_refresh = Some(Instant::now());
    }
}

/// Строки внешних данных для интерфейса.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalRows {
    index: usize,
    provider: String,
    values: Vec<String>,
    error: Option<String>,
    /// URL видеопотока для NDI-монитора (MJPEG).
    stream: Option<String>,
}

/// То, что в оригинале живёт в полях `vMixControlButton` между запусками скрипта.
#[derive(Debug, Default, Clone)]
struct WidgetRuntime {
    locals: BTreeMap<String, Value>,
    tracked: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentView {
    path: Option<String>,
    pages: Vec<String>,
    widgets: Vec<WidgetData>,
    /// `WindowSettings.Locked`: в оригинале при заблокированном контроллере виджеты
    /// исполняются, а при разблокированном — редактируются.
    locked: bool,
    /// Запрошенный при старте виджет (отдаётся один раз).
    selected: Option<usize>,
    /// Просьба открыть редактор скрипта (отдаётся один раз).
    open_script: bool,
    /// Просьба открыть просмотр строк источника (отдаётся один раз).
    open_rows: bool,
    /// Просьба сразу запустить MIDI.
    start_midi: bool,
    /// Просьба сразу запустить Stream Deck.
    start_deck: bool,
    /// Просьба показать расписание часов.
    show_schedule: bool,
    /// Просьба открыть меню поверхности (`vmixrtc файл.vmc 0 palette`).
    show_palette: bool,
    /// Dev: сразу набрать запрос в поиске функций (`vmixrtc файл.vmc 12 picker`).
    show_picker_query: Option<String>,
    /// Глобальные переменные контроллера — их показывают виджеты-просмотрщики.
    globals: Vec<(String, String)>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PaletteItem {
    /// Ключ локализации вида (`kind.button`).
    kind_key: String,
    type_name: String,
    label: String,
    width: f64,
    height: f64,
    color: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FunctionInfo {
    function: String,
    description: String,
    category: String,
    native: bool,
    new_style: bool,
    has_input: bool,
    has_int: bool,
    has_string: bool,
    has_float: bool,
    input_description: String,
    int_description: String,
    string_description: String,
    float_description: String,
    int_values: Vec<String>,
    string_values: Vec<String>,
    additional_count: usize,
}

impl FunctionInfo {
    fn from_ref(function: &FunctionRef) -> Self {
        Self {
            function: function.function.clone(),
            description: function.description.clone(),
            category: function.category.clone(),
            native: function.native,
            new_style: function.new_style,
            has_input: function.has_input,
            has_int: function.has_int,
            has_string: function.has_string,
            has_float: function.has_float,
            input_description: function.input_description.clone(),
            int_description: function.int_description.clone(),
            string_description: function.string_description.clone(),
            float_description: function.float_description.clone(),
            int_values: function.int_values.clone(),
            string_values: function.string_values.clone(),
            additional_count: function.additional_count,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogueInfo {
    count: usize,
    functions: Vec<FunctionInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PressResult {
    /// Что именно ушло в vMix и что он ответил.
    log: Vec<String>,
    active: bool,
    /// Команды страниц: интерфейс применяет их сам.
    page_delta: i32,
    page: Option<usize>,
}

/// Ответ опроса: состояние vMix и пересчитанная «нажатость» виджетов.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PollResult {
    state: VmixState,
    widgets: Vec<WidgetData>,
    externals: Vec<ExternalRows>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection {
    host: String,
    port: Option<u16>,
    login: Option<String>,
    password: Option<String>,
}

impl Connection {
    fn client(&self) -> VmixClient {
        let client = VmixClient::new(self.host.clone(), self.port.unwrap_or(DEFAULT_PORT));
        match (&self.login, &self.password) {
            (Some(login), Some(password)) if !login.is_empty() => {
                client.with_credentials(login.clone(), password.clone())
            }
            _ => client,
        }
    }
}

fn view(document: &Document) -> Result<DocumentView, String> {
    let vmc = document
        .vmc
        .as_ref()
        .ok_or_else(|| "документ не открыт".to_string())?;
    Ok(DocumentView {
        path: document.path.as_ref().map(|p| p.display().to_string()),
        pages: DEFAULT_PAGES.iter().map(|p| p.to_string()).collect(),
        widgets: vmc.widgets_data(),
        locked: vmc
            .window_settings()
            .map(|settings| settings.locked)
            .unwrap_or(false),
        selected: None,
        open_script: false,
        open_rows: false,
        start_midi: false,
        start_deck: false,
        show_schedule: false,
        show_palette: false,
        show_picker_query: None,
        globals: document
            .vmc
            .as_ref()
            .map(|vmc| vmc.global_variables())
            .unwrap_or_default(),
    })
}

/// Каталог функций: загружается при первом обращении и кэшируется.
fn ensure_catalogue(state: &AppState) -> Result<Catalogue, String> {
    let mut slot = state.catalogue.lock().map_err(|e| e.to_string())?;
    if let Some(catalogue) = slot.as_ref() {
        return Ok(catalogue.clone());
    }
    let catalogue = Catalogue::discover().map_err(|e| format!("{e:#}"))?;
    *slot = Some(catalogue.clone());
    Ok(catalogue)
}

/// Отправка запросов из скрипта: vMix API, ретрансляторы и произвольные URL (`API`/`APIPOST`).
struct VmixSender<'a> {
    client: &'a VmixClient,
    /// Включённые ретрансляторы (`vMixControlMultiState`): получают те же функции.
    relays: &'a [(String, VmixClient)],
    notes: RefCell<Vec<String>>,
}

impl<'a> VmixSender<'a> {
    fn new(client: &'a VmixClient, relays: &'a [(String, VmixClient)]) -> Self {
        Self {
            client,
            relays,
            notes: RefCell::new(Vec::new()),
        }
    }

    fn relay(&self, query: &str) {
        for (label, client) in self.relays {
            match client.send_query(query) {
                Ok(_) => self.notes.borrow_mut().push(format!("↔ {label}: ок")),
                Err(error) => self
                    .notes
                    .borrow_mut()
                    .push(format!("↔ {label}: {error:#}")),
            }
        }
    }

    fn notes(&self) -> Vec<String> {
        self.notes.borrow().clone()
    }
}

impl FunctionSender for VmixSender<'_> {
    fn send_query(&self, query: &str) -> anyhow::Result<String> {
        let answer = self.client.send_query(query)?;
        self.relay(query);
        Ok(answer)
    }

    fn fetch_url(&self, url: &str, post: bool) -> anyhow::Result<String> {
        self.client.fetch_url(url, post)
    }
}

/// Клиенты включённых ретрансляторов из документа.
fn relay_clients(state: &AppState) -> Vec<(String, VmixClient)> {
    let Ok(document) = state.document.lock() else {
        return Vec::new();
    };
    let Some(vmc) = document.vmc.as_ref() else {
        return Vec::new();
    };
    vmc.relay_targets()
        .into_iter()
        .map(|target| {
            let label = format!("{}:{}", target.ip, target.port);
            let client = VmixClient::new(&target.ip, target.port)
                .with_credentials(target.login, target.password);
            (label, client)
        })
        .collect()
}

/// Прогнать скрипт виджета: возвращает результат, а также обновлённые локальные
/// переменные и отслеженные значения (их нужно сохранить между нажатиями).
fn run_script(
    client: &VmixClient,
    relays: &[(String, VmixClient)],
    catalogue: &Catalogue,
    commands: &[FunctionCommand],
    state_node: Option<&Node>,
    is_pushed: bool,
    locals: BTreeMap<String, Value>,
    tracked: BTreeMap<String, Value>,
) -> anyhow::Result<(ScriptOutcome, BTreeMap<String, Value>, BTreeMap<String, Value>)> {
    let mut runner = ScriptRunner::new(commands, state_node);
    runner.catalogue = Some(catalogue);
    runner.is_pushed = is_pushed;
    runner.locals = locals;
    runner.tracked = tracked;
    let sender = VmixSender::new(client, relays);
    let mut outcome = runner.run(&sender)?;
    // ретрансляция тоже попадает в журнал
    outcome.log.extend(sender.notes());
    Ok((outcome, runner.locals, runner.tracked))
}

// ------------------------------------------------------------ команды: документ

#[tauri::command]
fn vmc_new(state: tauri::State<'_, AppState>) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    document.path = None;
    document.vmc = Some(Vmc::empty());
    view(&document)
}

/// Первый документ при старте: `.vmc` из аргумента командной строки либо пустой.
#[tauri::command]
fn vmc_startup(state: tauri::State<'_, AppState>) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    if document.vmc.is_some() {
        return view(&document);
    }
    match &state.startup_path {
        Some(path) => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let vmc =
                Vmc::parse(&bytes).map_err(|e| format!("{}: {e:#}", path.display()))?;
            document.path = Some(path.clone());
            document.vmc = Some(vmc);
        }
        None => {
            document.path = None;
            document.vmc = Some(Vmc::empty());
        }
    }
    let mut document_view = view(&document)?;
    // индекс виджета из аргумента командной строки отдаём один раз
    let mut slot = state.startup_select.lock().map_err(|e| e.to_string())?;
    document_view.selected = slot.take();
    let mut script_slot = state.startup_script.lock().map_err(|e| e.to_string())?;
    document_view.open_script = std::mem::take(&mut *script_slot);
    let mut rows_slot = state.startup_rows.lock().map_err(|e| e.to_string())?;
    document_view.open_rows = std::mem::take(&mut *rows_slot);
    let mut midi_slot = state.startup_midi.lock().map_err(|e| e.to_string())?;
    document_view.start_midi = std::mem::take(&mut *midi_slot);
    let mut deck_slot = state.startup_deck.lock().map_err(|e| e.to_string())?;
    document_view.start_deck = std::mem::take(&mut *deck_slot);
    let mut schedule_slot = state.startup_schedule.lock().map_err(|e| e.to_string())?;
    document_view.show_schedule = std::mem::take(&mut *schedule_slot);
    let mut palette_slot = state.startup_palette.lock().map_err(|e| e.to_string())?;
    document_view.show_palette = std::mem::take(&mut *palette_slot);
    let mut picker_slot = state.startup_picker.lock().map_err(|e| e.to_string())?;
    document_view.show_picker_query = picker_slot.take();
    Ok(document_view)
}

#[tauri::command]
fn vmc_open(path: String, state: tauri::State<'_, AppState>) -> Result<DocumentView, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let vmc = Vmc::parse(&bytes).map_err(|e| format!("{path}: {e:#}"))?;
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    document.path = Some(PathBuf::from(&path));
    document.vmc = Some(vmc);
    view(&document)
}

#[tauri::command]
fn vmc_save(
    path: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    if let Some(path) = path.filter(|p| !p.trim().is_empty()) {
        document.path = Some(PathBuf::from(path));
    }
    let target = document
        .path
        .clone()
        .ok_or_else(|| "не задан путь для сохранения".to_string())?;
    let vmc = document
        .vmc
        .as_ref()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.save(&target).map_err(|e| format!("{e:#}"))?;
    view(&document)
}

#[tauri::command]
fn vmc_palette() -> Vec<PaletteItem> {
    WidgetKind::PALETTE
        .iter()
        .map(|kind| {
            let (width, height) = kind.default_size();
            PaletteItem {
                kind_key: kind.localization_key().to_string(),
                type_name: kind.type_name(),
                label: kind.label(),
                width,
                height,
                color: kind.default_color().to_css(),
            }
        })
        .collect()
}

#[tauri::command]
fn vmc_add(
    type_name: String,
    left: f64,
    top: f64,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    let kind = WidgetKind::from_type_name(&type_name);
    vmc.create_widget(&kind, left, top)
        .map_err(|e| format!("{e:#}"))?;
    view(&document)
}

#[tauri::command]
fn vmc_update(
    index: usize,
    patch: WidgetPatch,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.update_widget(index, &patch).map_err(|e| format!("{e:#}"))?;
    view(&document)
}

#[tauri::command]
fn vmc_remove(index: usize, state: tauri::State<'_, AppState>) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.remove_widget(index).map_err(|e| format!("{e:#}"))?;
    view(&document)
}

#[tauri::command]
fn vmc_duplicate(index: usize, state: tauri::State<'_, AppState>) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.duplicate_widget(index).map_err(|e| format!("{e:#}"))?;
    view(&document)
}

#[tauri::command]
fn vmc_set_locked(locked: bool, state: tauri::State<'_, AppState>) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.set_window_locked(locked).map_err(|e| format!("{e:#}"))?;
    view(&document)
}

// ------------------------------------------------------------ команды: функции

#[tauri::command]
fn catalogue_info(state: tauri::State<'_, AppState>) -> Result<CatalogueInfo, String> {
    let catalogue = ensure_catalogue(&state)?;
    let mut names = catalogue.callable_names(false);
    names.extend(catalogue.callable_names(true));
    names.sort_unstable();
    names.dedup();

    let functions = names
        .into_iter()
        .filter_map(|name| catalogue.find(name))
        .map(FunctionInfo::from_ref)
        .collect::<Vec<_>>();

    Ok(CatalogueInfo {
        count: catalogue.len(),
        functions,
    })
}

/// Скрипт виджета в текстовом виде — по строке на команду (как `ToString` в оригинале).
#[tauri::command]
fn vmc_export_script(index: usize, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_ref()
        .ok_or_else(|| "документ не открыт".to_string())?;
    let widget = vmc
        .widgets_data()
        .into_iter()
        .find(|widget| widget.index == index)
        .ok_or_else(|| format!("виджета с индексом {index} нет"))?;
    Ok(commands_of_widget(&widget)
        .iter()
        .map(|command| command.to_text())
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Импорт скрипта из текста: каждая строка — команда, параметры разбираются
/// по сигнатуре функции из каталога (как `FromString` в оригинале).
#[tauri::command]
fn vmc_import_script(
    index: usize,
    text: String,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let catalogue = ensure_catalogue(&state)?;
    let mut commands: Vec<WidgetCommand> = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let parsed = vmix_functions::Command::parse_with_signature(line, &catalogue)
            .map_err(|e| format!("строка {}: {e:#}", line_number + 1))?;
        commands.push(widget_command_from(&parsed));
    }

    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.set_widget_commands(index, &commands)
        .map_err(|e| format!("{e:#}"))?;
    view(&document)
}

/// Команда скриптового слоя → команда виджета (для импорта).
fn widget_command_from(command: &FunctionCommand) -> WidgetCommand {
    WidgetCommand {
        function: command.function.clone(),
        description: command.description.clone(),
        format_string: command.format_string.clone().unwrap_or_default(),
        native: command.native,
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
        active_state_path: command.active_state_path.clone(),
        active_state_xpath: command.active_state_xpath.clone(),
        active_state_value: command.active_state_value.clone(),
        active_state_xpath_int_dependence: command.active_state_xpath_int_dependence.clone(),
    }
}

/// Заменить команды кнопки. Если у команды нет `FormatString`, он берётся из каталога —
/// так `.vmc` остаётся исполняемым и в оригинальном приложении. Оттуда же берутся
/// поля состояния (`ActiveState*`), по которым кнопка подсвечивается.
#[tauri::command]
fn vmix_set_commands(
    index: usize,
    commands: Vec<WidgetCommand>,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let catalogue = ensure_catalogue(&state).ok();
    let mut commands = commands;
    if let Some(catalogue) = &catalogue {
        for command in commands.iter_mut() {
            if let Some(function) = catalogue.find(&command.function) {
                if command.format_string.is_empty() {
                    command.format_string = function.format_string.clone();
                }
                if command.description.is_empty() {
                    command.description = function.description.clone();
                }
                if command.active_state_path.is_empty() {
                    command.active_state_path = function.active_state_path.clone();
                }
                if command.active_state_xpath.is_empty() {
                    command.active_state_xpath = function.active_state_xpath.clone();
                }
                if command.active_state_value.is_empty() {
                    command.active_state_value = function.active_state_value.clone();
                }
                if command.active_state_xpath_int_dependence.is_empty() {
                    command.active_state_xpath_int_dependence = function
                        .active_state_xpath_int_dependence
                        .clone()
                        .unwrap_or_default();
                }
                command.native = function.native;
            }
        }
    }

    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.set_widget_commands(index, &commands)
        .map_err(|e| format!("{e:#}"))?;
    view(&document)
}

/// Сигнатура функции для оценки состояния: сначала из самой команды (так лежит в `.vmc`),
/// затем из каталога.
fn command_function_ref(command: &WidgetCommand, catalogue: &Catalogue) -> Option<FunctionRef> {
    if !command.active_state_xpath.is_empty()
        || !command.active_state_path.is_empty()
        || !command.active_state_value.is_empty()
        || !command.active_state_xpath_int_dependence.is_empty()
    {
        return Some(FunctionRef {
            function: command.function.clone(),
            active_state_path: command.active_state_path.clone(),
            active_state_xpath: command.active_state_xpath.clone(),
            active_state_value: command.active_state_value.clone(),
            active_state_xpath_int_dependence: Some(
                command.active_state_xpath_int_dependence.clone(),
            )
            .filter(|items| !items.is_empty()),
            ..Default::default()
        });
    }
    catalogue.find(&command.function).cloned()
}

// ------------------------------------------------------------ команды: vMix

#[tauri::command]
fn vmix_state(connection: Connection) -> Result<VmixState, String> {
    connection.client().state().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn vmix_call(
    connection: Connection,
    function: String,
    params: Option<Vec<(String, String)>>,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let params = params.unwrap_or_default();
    let borrowed: Vec<(&str, &str)> = params
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let answer = connection
        .client()
        .send_function(&function, &borrowed)
        .map_err(|e| format!("{e:#}"))?;

    // ретрансляторы (`vMixControlMultiState`) получают ту же функцию
    let mut notes = Vec::new();
    for (label, client) in relay_clients(&state) {
        match client.send_function(&function, &borrowed) {
            Ok(_) => notes.push(format!("↔ {label}: ок")),
            Err(error) => notes.push(format!("↔ {label}: {error:#}")),
        }
    }
    if notes.is_empty() {
        Ok(answer)
    } else {
        Ok(format!("{answer}\n{}", notes.join("\n")))
    }
}

/// Опрос vMix: состояние для панели и «нажатость» виджетов — одним запросом.
#[tauri::command]
fn vmix_poll(
    connection: Connection,
    state: tauri::State<'_, AppState>,
) -> Result<PollResult, String> {
    let client = connection.client();
    let xml = client.fetch_state_xml().map_err(|e| format!("{e:#}"))?;
    let vmix_state = VmixState::parse(&xml).map_err(|e| format!("{e:#}"))?;
    let catalogue = ensure_catalogue(&state).unwrap_or_default();

    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let widgets = match document.vmc.as_mut() {
        Some(vmc) => widget_states(vmc, &catalogue, &vmix_state.raw),
        None => Vec::new(),
    };
    drop(document);

    let externals = refresh_externals(&state, &widgets, Some(&vmix_state.raw));
    Ok(PollResult {
        state: vmix_state,
        widgets,
        externals,
    })
}

/// Обновить провайдеры внешних данных (каждый — не чаще своего `Period`).
fn refresh_externals(
    app: &AppState,
    widgets: &[WidgetData],
    state_node: Option<&Node>,
) -> Vec<ExternalRows> {
    let mut runtime = match app.externals.lock() {
        Ok(runtime) => runtime,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut rows = Vec::new();
    for widget in widgets {
        let Some(external) = &widget.external else {
            continue;
        };
        let entry = runtime.entry(widget.index).or_default();
        entry.ensure(external);

        let period = external.period_ms.max(100) as u64;
        let due = entry
            .last_refresh
            .map(|moment| moment.elapsed().as_millis() as u64 >= period)
            .unwrap_or(true);
        if external.enabled && due {
            entry.refresh(state_node);
        }
        rows.push(ExternalRows {
            index: widget.index,
            provider: entry.label(),
            values: entry.values.clone(),
            error: entry.error.clone(),
            stream: None,
        });
    }

    // виджеты-потребители (`List`) берут значения у провайдера по имени источника
    let by_name: HashMap<String, (String, Vec<String>, Option<String>)> = widgets
        .iter()
        .filter_map(|widget| {
            let name = widget.name.clone();
            let external = widget.external.as_ref()?;
            if external.provider_path.is_empty() {
                return None;
            }
            let entry = runtime.get(&widget.index)?;
            Some((
                name,
                (entry.label(), entry.values.clone(), entry.error.clone()),
            ))
        })
        .collect();

    for widget in widgets {
        let Some(external) = &widget.external else {
            continue;
        };
        if external.source_name.is_empty() {
            continue;
        }
        let Some((provider, values, error)) = by_name.get(&external.source_name) else {
            continue;
        };
        if let Some(row) = rows.iter_mut().find(|row| row.index == widget.index) {
            row.provider = provider.clone();
            row.values = values.clone();
            row.error = error.clone();
        }
    }

    // NDI-монитор: поднимаем видеопоток (NDI runtime, а без него — тестовый сигнал)
    for widget in widgets {
        let Some(external) = &widget.external else {
            continue;
        };
        if !external.provider_path.to_ascii_lowercase().contains("ndi") {
            continue;
        }
        let Some(row) = rows.iter_mut().find(|row| row.index == widget.index) else {
            continue;
        };
        match ensure_ndi_stream(app, widget.index, external) {
            Ok((url, description)) => {
                row.provider = description;
                row.stream = Some(url);
            }
            Err(error) => row.error = Some(format!("{error:#}")),
        }
    }
    rows
}

/// Поднять (один раз) MJPEG-поток для виджета: NDI runtime, иначе тестовый сигнал.
fn ensure_ndi_stream(
    app: &AppState,
    index: usize,
    external: &vmix_config::ExternalData,
) -> anyhow::Result<(String, String)> {
    let mut streams = match app.ndi.lock() {
        Ok(streams) => streams,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(server) = streams.get(&index) {
        return Ok((server.url(), "NDI".to_string()));
    }

    let source = external
        .provider_properties
        .first()
        .map(String::as_str)
        .unwrap_or_default();
    let (backend, description): (Box<dyn Backend>, String) =
        match RuntimeBackend::connect(Some(source).filter(|name| !name.trim().is_empty())) {
            Ok(backend) => {
                let description = backend.describe();
                (Box::new(backend), description)
            }
            Err(error) => (
                Box::new(TestBackend::new(640, 360).with_label("тестовый сигнал")),
                format!("NDI (нет runtime: {error})"),
            ),
        };

    let server = MjpegServer::start(backend, 75, 20)?;
    let url = server.url();
    streams.insert(index, server);
    Ok((url, description))
}

/// Остановить видеопоток виджета.
#[tauri::command]
fn ndi_stop(index: usize, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut streams = state.ndi.lock().map_err(|e| e.to_string())?;
    if let Some(server) = streams.remove(&index) {
        server.stop();
    }
    Ok(())
}

/// Применить строку внешних данных к тайтлам vMix — порт `UpdateText`.
/// Пары из `Paths`: «вход → имя элемента тайтла»; значения берутся позиционно,
/// а строка, начинающаяся с `@[cmd]`, отправляется как команда.
#[tauri::command]
fn external_apply(
    index: usize,
    row: usize,
    connection: Connection,
    state: tauri::State<'_, AppState>,
) -> Result<PressResult, String> {
    let (external, values) = {
        let document = state.document.lock().map_err(|e| e.to_string())?;
        let vmc = document
            .vmc
            .as_ref()
            .ok_or_else(|| "документ не открыт".to_string())?;
        let widget = vmc
            .widgets_data()
            .into_iter()
            .find(|widget| widget.index == index)
            .ok_or_else(|| format!("виджета с индексом {index} нет"))?;
        let external = widget
            .external
            .ok_or_else(|| "у виджета нет внешних данных".to_string())?;
        let values = state
            .externals
            .lock()
            .map_err(|e| e.to_string())?
            .get(&index)
            .map(|runtime| runtime.values.clone())
            .unwrap_or_default();
        (external, values)
    };

    if values.is_empty() {
        return Err("провайдер не вернул значений".into());
    }
    let selected = values.get(row).cloned().unwrap_or_default();

    let catalogue = ensure_catalogue(&state)?;
    let client = connection.client();
    let log = apply_external_row(&client, &catalogue, &external, &values, row);

    // выбранная строка запоминается в виджете, как `Text` в оригинале
    if let Ok(mut document) = state.document.lock() {
        if let Some(vmc) = document.vmc.as_mut() {
            let _ = vmc.update_widget(
                index,
                &WidgetPatch {
                    text: Some(selected.clone()),
                    ..Default::default()
                },
            );
        }
    }

    Ok(PressResult {
        log,
        active: true,
        page_delta: 0,
        page: None,
    })
}

/// Ядро применения строки: пары `Paths` → запросы к vMix (или команды для `@[cmd]`).
fn apply_external_row(
    client: &VmixClient,
    catalogue: &Catalogue,
    external: &vmix_config::ExternalData,
    values: &[String],
    row: usize,
) -> Vec<String> {
    let state_xml = client.fetch_state_xml().ok();
    let state_node = state_xml
        .as_deref()
        .and_then(|xml| vmix_xml::parse(xml.as_bytes()).ok());
    let _ = row;

    let mut log = Vec::new();
    for (position, (input_key, element_name)) in external.paths.iter().enumerate() {
        let value = if position >= values.len() && !external.restart_data {
            String::new()
        } else {
            values[position % values.len()].clone()
        };
        if value.is_empty() {
            continue;
        }

        // хук: значение вида @[cmd] — это команда vMix, а не текст
        if let Some(command) = value.strip_prefix("@[cmd]") {
            let command = command.replace("{0}", input_key).replace("{1}", element_name);
            if command.trim().is_empty() {
                continue;
            }
            match client.send_query(&command) {
                Ok(_) => log.push(format!("{command} → ок")),
                Err(error) => log.push(format!("{command} → {error:#}")),
            }
            continue;
        }

        let Some(state_node) = state_node.as_ref() else {
            log.push(format!("{element_name}: нет состояния vMix"));
            continue;
        };
        let Some(element_index) = element_index(
            state_node,
            input_key,
            element_name,
            external.is_mapped_to_guid,
        ) else {
            log.push(format!("{element_name}: элемент не найден у входа {input_key}"));
            continue;
        };

        let command = FunctionCommand {
            function: "SetText".into(),
            input_key: Some(input_key.clone()),
            parameter: Some(element_index.to_string()),
            string_parameter: Some(value.clone()),
            executable: true,
            use_in_active_state: true,
            ..Default::default()
        };
        match command.render(Some(&catalogue)) {
            Ok(query) => match client.send_query(&query) {
                Ok(_) => log.push(format!("{query} → ок")),
                Err(error) => log.push(format!("{query} → {error:#}")),
            },
            Err(error) => log.push(format!("SetText → {error:#}")),
        }
    }

    log
}

/// Номер (`index`) элемента тайтла с указанным именем.
///
/// `IsMappedToGUID` в оригинале только хранится в `.vmc` и нигде не читается (объявление
/// свойства и всё). Порт трактует его по смыслу: `true` — `Paths` ссылается на вход по ключу
/// (GUID), `false` — по номеру. Для номеров остаётся и поиск по ключу как запасной вариант.
fn element_index(
    state: &Node,
    input_key: &str,
    element_name: &str,
    mapped_to_guid: bool,
) -> Option<i64> {
    let inputs = state.path(&["inputs"])?;
    let input = if mapped_to_guid {
        inputs
            .children_named("input")
            .find(|input| input.attr("key") == Some(input_key))
    } else {
        inputs
            .children_named("input")
            .find(|input| input.attr("number") == Some(input_key))
            .or_else(|| {
                inputs
                    .children_named("input")
                    .find(|input| input.attr("key") == Some(input_key))
            })
    }?;
    input
        .children
        .iter()
        .filter(|child| child.name == "text" || child.name == "image")
        .find(|child| child.attr("name") == Some(element_name))
        .and_then(|child| child.attr("index"))
        .and_then(|index| index.parse().ok())
}

/// Импортировать другой `.vmc` внутрь контейнера (порт `AfterPropertiesChanged`).
#[tauri::command]
fn vmc_container_import(
    index: usize,
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let bytes = std::fs::read(&path).map_err(|error| format!("{path}: {error}"))?;
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.import_into_container(index, &bytes)
        .map_err(|e| format!("{e:#}"))?;
    view(&document)
}

/// Записать настройки внешних данных (источник, XPath, период).
#[tauri::command]
fn vmc_set_external(
    index: usize,
    external: vmix_config::ExternalData,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.set_widget_external(index, &external)
        .map_err(|e| format!("{e:#}"))?;
    // настройки изменились — провайдер пересоберётся при следующем опросе
    if let Ok(mut runtime) = state.externals.lock() {
        runtime.remove(&index);
    }
    view(&document)
}

/// Пересчитать «нажатость» виджетов по состоянию vMix (порт `CalculateStateDependency`).
fn widget_states(vmc: &mut Vmc, catalogue: &Catalogue, state_node: &Node) -> Vec<WidgetData> {
    let maps = InputMaps::from_state(state_node);
    for widget in vmc.widgets_data() {
        if widget.commands.is_empty() {
            continue;
        }
        let commands = commands_of_widget(&widget);
        let active = commands
            .iter()
            .zip(widget.commands.iter())
            .any(|(function_command, command)| match command_function_ref(command, catalogue) {
                Some(function) => {
                    evaluate(function_command, &function, state_node, &maps) == Some(true)
                }
                None => false,
            });
        let _ = vmc.set_widget_active(widget.index, active);
    }
    vmc.widgets_data()
}

/// Обработать ссылку (`Hotkey.Link`): запустить действия всех виджетов, которые её слушают.
/// Порт `ProcessHotkey` из `MainViewModel`.
/// Перевод строки по ключу. Мьютекс берётся ровно на время подстановки и сразу отпускается:
/// диспетчер ссылок вызывает себя рекурсивно, и удерживать блокировку нельзя (иначе дедлок).
fn tr(app: &AppState, key: &str, args: &[&str]) -> String {
    match app.i18n.lock() {
        Ok(i18n) => i18n.tf(key, args),
        Err(poisoned) => poisoned.into_inner().tf(key, args),
    }
}

/// Предел длины цепочки ссылок (аналог `ScriptExecutionLoopGuard` в оригинале).
const MAX_LINK_DEPTH: usize = 16;

fn dispatch_link(
    app: &AppState,
    client: &VmixClient,
    catalogue: &Catalogue,
    state_node: Option<&Node>,
    link: &str,
    value: Option<u8>,
) -> Vec<String> {
    dispatch_link_at(app, client, catalogue, state_node, link, value, 0)
}

fn dispatch_link_at(
    app: &AppState,
    client: &VmixClient,
    catalogue: &Catalogue,
    state_node: Option<&Node>,
    link: &str,
    value: Option<u8>,
    depth: usize,
) -> Vec<String> {
    let mut log = Vec::new();
    if depth > MAX_LINK_DEPTH {
        log.push(tr(app, "link.tooLong", &[link]));
        return log;
    }
    let targets = {
        let Ok(document) = app.document.lock() else {
            return log;
        };
        let Some(vmc) = document.vmc.as_ref() else {
            return log;
        };
        vmc.hotkey_targets(link)
    };
    if targets.is_empty() {
        log.push(tr(app, "link.none", &[link]));
        return log;
    }

    let relays = relay_clients(app);
    for (index, position) in targets {
        let widget = {
            let Ok(document) = app.document.lock() else { break };
            let Some(vmc) = document.vmc.as_ref() else { break };
            vmc.widgets_data()
                .into_iter()
                .find(|widget| widget.index == index)
        };
        let Some(widget) = widget else { continue };
        let action = widget
            .hotkeys
            .get(position)
            .map(|hotkey| hotkey.name.clone())
            .unwrap_or_default();

        // Кнопки: «Execute» прогоняет скрипт (как нажатие), остальные действия — журналируем
        if matches!(widget.kind, WidgetKind::Button | WidgetKind::NewButton)
            && action.eq_ignore_ascii_case("Execute")
        {
            let commands = commands_of_widget(&widget);
            let mut locales = BTreeMap::new();
            let mut tracked = BTreeMap::new();
            {
                let runtime = match app.runtime.lock() {
                    Ok(runtime) => runtime,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if let Some(entry) = runtime.get(&(index as u64)) {
                    locales = entry.locals.clone();
                    tracked = entry.tracked.clone();
                }
            }
            // значение MIDI доступно выражению как `_param`
            if let Some(value) = value {
                locales.insert("_param".to_string(), Value::Number(value as f64));
            }
            match run_script(
                client,
                &relays,
                catalogue,
                &commands,
                state_node,
                widget.active,
                locales,
                tracked,
            ) {
                Ok((outcome, locals, tracked)) => {
                    log.push(format!("«{link}» → {}:", widget.name));
                    log.extend(outcome.log);
                    if let Ok(mut runtime) = app.runtime.lock() {
                        runtime.insert(index as u64, WidgetRuntime { locals, tracked });
                    }
                    // `ExecLink` внутри скрипта запускает другие ссылки
                    for nested in &outcome.links {
                        log.extend(dispatch_link_at(
                            app,
                            client,
                            catalogue,
                            state_node,
                            nested,
                            value,
                            depth + 1,
                        ));
                    }
                }
                Err(error) => log.push(format!("«{link}» → {}: {error:#}", widget.name)),
            }
        } else {
            log.push(tr(app, "link.unsupported", &[link, &widget.name, &action]));
        }
    }
    log
}

/// Обработать ссылку вручную (из интерфейса или теста).
#[tauri::command]
fn vmix_dispatch_link(
    link: String,
    value: Option<u8>,
    connection: Connection,
    state: tauri::State<'_, AppState>,
) -> Result<PressResult, String> {
    let catalogue = ensure_catalogue(&state)?;
    let client = connection.client();
    let state_xml = client.fetch_state_xml().ok();
    let state_node = state_xml
        .as_deref()
        .and_then(|xml| vmix_xml::parse(xml.as_bytes()).ok());
    let log = dispatch_link(
        &state,
        &client,
        &catalogue,
        state_node.as_ref(),
        &link,
        value,
    );
    Ok(PressResult {
        log,
        active: false,
        page_delta: 0,
        page: None,
    })
}

/// Текущий язык интерфейса.
#[tauri::command]
fn language(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let i18n = state.i18n.lock().map_err(|e| e.to_string())?;
    Ok(i18n.language().to_string())
}

/// Сменить язык интерфейса (сохраняется в настройках платформы).
#[tauri::command]
fn set_language(lang: String, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let mut i18n = state.i18n.lock().map_err(|e| e.to_string())?;
    if !i18n.set_language(&lang) {
        return Err(format!("язык «{lang}» не поддержан"));
    }
    Ok(i18n.language().to_string())
}

/// Записать одно свойство виджета (`Target`, `InputKey`, `Variable`, `Style`, …).
#[tauri::command]
fn vmc_set_extra(
    index: usize,
    tag: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.set_widget_extra(index, &tag, &value)
        .map_err(|e| format!("{e:#}"))?;
    view(&document)
}

/// Записать расписание часов виджета.
#[tauri::command]
fn vmc_set_events(
    index: usize,
    events: Vec<vmix_config::ScheduledEvent>,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.set_widget_events(index, &events)
        .map_err(|e| format!("{e:#}"))?;
    view(&document)
}

/// Правка ссылки горячей клавиши виджета (её слушают MIDI, Stream Deck и скрипты).
#[tauri::command]
fn vmc_set_hotkey(
    index: usize,
    position: usize,
    link: String,
    active: bool,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    vmc.set_widget_hotkey(index, position, &link, active)
        .map_err(|e| format!("{e:#}"))?;
    view(&document)
}

/// Ссылки, которые запускает событие Stream Deck.
///
/// Оригинал реагирует именно на **отпускание** кнопки (`StreamDeckEvent.KeyUp`) — порт
/// сохраняет это поведение.
fn deck_link_targets(vmc: &Vmc, event: &DeckEvent) -> Vec<String> {
    if event.kind != DeckEventKind::KeyUp {
        return Vec::new();
    }
    vmc.widgets_data()
        .into_iter()
        .flat_map(|widget| widget.deck_keys)
        .filter(|key| key.index as usize == event.index && !key.link.trim().is_empty())
        .map(|key| key.link)
        .collect()
}

/// Найденные Stream Deck.
#[tauri::command]
fn streamdeck_devices() -> Vec<String> {
    vmix_streamdeck::runtime::devices()
        .map(|devices| devices.into_iter().map(|(name, _)| name).collect())
        .unwrap_or_default()
}

/// Начать слушать Stream Deck (без устройства — тестовый источник для проверки цепочки).
#[tauri::command]
fn streamdeck_start(
    device: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let wanted = device.unwrap_or_default();
    let mut runtime = state.deck.lock().map_err(|e| e.to_string())?;
    match vmix_streamdeck::runtime::RuntimeSource::open(&wanted) {
        Ok(source) => {
            runtime.device = source.describe();
            runtime.source = Some(Box::new(source));
        }
        Err(error) => {
            let demo = vec![
                DeckEvent {
                    kind: DeckEventKind::KeyDown,
                    index: 0,
                },
                DeckEvent {
                    kind: DeckEventKind::KeyUp,
                    index: 0,
                },
            ];
            runtime.device = format!("тестовый Stream Deck (устройство недоступно: {error:#})");
            runtime.source = Some(Box::new(DeckTestSource::new(demo)));
        }
    }
    Ok(runtime.device.clone())
}

/// Остановить Stream Deck.
#[tauri::command]
fn streamdeck_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut runtime = state.deck.lock().map_err(|e| e.to_string())?;
    runtime.source = None;
    Ok(())
}

/// Яркость подсветки, 0…100.
#[tauri::command]
fn streamdeck_brightness(
    percent: u8,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut runtime = state.deck.lock().map_err(|e| e.to_string())?;
    runtime.brightness = percent.min(100);
    match runtime.source.as_mut() {
        Some(source) => source.set_brightness(percent).map_err(|e| format!("{e:#}")),
        None => Ok(()),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeckPoll {
    device: String,
    events: Vec<DeckEvent>,
    log: Vec<String>,
}

/// Забрать события Stream Deck и разобрать ссылки нажатых кнопок.
#[tauri::command]
fn streamdeck_poll(
    connection: Connection,
    state: tauri::State<'_, AppState>,
) -> Result<DeckPoll, String> {
    let events = {
        let mut runtime = state.deck.lock().map_err(|e| e.to_string())?;
        let events = runtime
            .source
            .as_mut()
            .map(|source| source.poll())
            .unwrap_or_default();
        if let Some(last) = events
            .iter()
            .find(|event| event.kind == DeckEventKind::KeyUp)
            .or_else(|| events.last())
        {
            runtime.last = Some(*last);
        }
        events
    };

    let mut log = Vec::new();
    if !events.is_empty() {
        let links: Vec<String> = {
            let document = state.document.lock().map_err(|e| e.to_string())?;
            document
                .vmc
                .as_ref()
                .map(|vmc| {
                    events
                        .iter()
                        .flat_map(|event| deck_link_targets(vmc, event))
                        .collect()
                })
                .unwrap_or_default()
        };
        if links.is_empty() {
            log.push(format!("Stream Deck: событий {}, ссылок нет", events.len()));
        } else {
            let catalogue = ensure_catalogue(&state)?;
            let client = connection.client();
            let state_xml = client.fetch_state_xml().ok();
            let state_node = state_xml
                .as_deref()
                .and_then(|xml| vmix_xml::parse(xml.as_bytes()).ok());
            for link in links {
                log.extend(dispatch_link(
                    &state,
                    &client,
                    &catalogue,
                    state_node.as_ref(),
                    &link,
                    None,
                ));
            }
        }
    }

    let device = state
        .deck
        .lock()
        .map(|runtime| runtime.device.clone())
        .unwrap_or_default();
    Ok(DeckPoll {
        device,
        events,
        log,
    })
}

/// Привязать последнюю нажатую кнопку Stream Deck к ссылке (обучение).
#[tauri::command]
fn streamdeck_learn(
    index: usize,
    link: String,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let event = state
        .deck
        .lock()
        .map_err(|e| e.to_string())?
        .last
        .ok_or_else(|| "кнопок ещё не нажимали".to_string())?;

    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    let widget = vmc
        .widgets_data()
        .into_iter()
        .find(|widget| widget.index == index)
        .ok_or_else(|| format!("виджета с индексом {index} нет"))?;
    let mut keys = widget.deck_keys.clone();
    keys.retain(|key| key.link != link && key.index as usize != event.index);
    keys.push(vmix_config::DeckKey {
        context: event.index.to_string(),
        link,
        index: event.index as i64,
        extra: 0,
    });
    vmc.set_widget_deck_keys(index, &keys)
        .map_err(|e| format!("{e:#}"))?;
    view(&document)
}

/// Доступные MIDI-входы.
#[tauri::command]
fn midi_ports() -> Vec<String> {
    vmix_midi::runtime::input_ports().unwrap_or_default()
}

/// Начать слушать MIDI (без устройств — тестовый источник, чтобы проверить цепочку).
#[tauri::command]
fn midi_start(device: Option<String>, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let wanted = device.unwrap_or_default();
    let mut runtime = state.midi.lock().map_err(|e| e.to_string())?;
    match vmix_midi::runtime::RuntimeSource::connect(&wanted) {
        Ok(source) => {
            runtime.device = source.describe();
            runtime.source = Some(Box::new(source));
        }
        Err(error) => {
            // без железа показываем тестовые сообщения, чтобы цепочка была проверяема
            let demo = vec![
                MidiMessage {
                    kind: vmix_midi::MidiKind::NoteOn,
                    channel: 0,
                    number: 36,
                    value: 127,
                },
                MidiMessage {
                    kind: vmix_midi::MidiKind::ControlChange,
                    channel: 0,
                    number: 7,
                    value: 64,
                },
            ];
            runtime.device = format!("тестовый MIDI (устройство недоступно: {error:#})");
            runtime.source = Some(Box::new(TestSource::new(demo)));
        }
    }
    Ok(runtime.device.clone())
}

/// Остановить MIDI.
#[tauri::command]
fn midi_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut runtime = state.midi.lock().map_err(|e| e.to_string())?;
    runtime.source = None;
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MidiPoll {
    device: String,
    messages: Vec<MidiMessage>,
    log: Vec<String>,
}

/// Забрать накопившиеся MIDI-сообщения и разобрать их ссылки.
#[tauri::command]
fn midi_poll(
    connection: Connection,
    state: tauri::State<'_, AppState>,
) -> Result<MidiPoll, String> {
    let messages = {
        let mut runtime = state.midi.lock().map_err(|e| e.to_string())?;
        let messages = runtime
            .source
            .as_mut()
            .map(|source| source.poll())
            .unwrap_or_default();
        if let Some(last) = messages.last() {
            runtime.last = Some(*last);
        }
        messages
    };

    let mut log = Vec::new();
    if !messages.is_empty() {
        let mappings = {
            let document = state.document.lock().map_err(|e| e.to_string())?;
            document
                .vmc
                .as_ref()
                .map(|vmc| {
                    vmc.widgets_data()
                        .into_iter()
                        .flat_map(|widget| {
                            widget
                                .midi_map
                                .iter()
                                .filter_map(|entry| {
                                    vmix_midi::MidiKind::parse(&entry.kind).map(|kind| {
                                        vmix_midi::MidiMapping {
                                            kind,
                                            channel: entry.channel,
                                            number: entry.number,
                                            link: entry.link.clone(),
                                        }
                                    })
                                })
                                .collect::<Vec<_>>()
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };

        if !mappings.is_empty() {
            let catalogue = ensure_catalogue(&state)?;
            let client = connection.client();
            let state_xml = client.fetch_state_xml().ok();
            let state_node = state_xml
                .as_deref()
                .and_then(|xml| vmix_xml::parse(xml.as_bytes()).ok());
            for message in &messages {
                for (link, value) in midi_dispatch(&mappings, message) {
                    log.extend(dispatch_link(
                        &state,
                        &client,
                        &catalogue,
                        state_node.as_ref(),
                        &link,
                        Some(value),
                    ));
                }
            }
        } else {
            log.push(format!("MIDI: получено сообщений {}, отображений нет", messages.len()));
        }
    }

    let device = state
        .midi
        .lock()
        .map(|runtime| runtime.device.clone())
        .unwrap_or_default();
    Ok(MidiPoll {
        device,
        messages,
        log,
    })
}

/// Связать последнее MIDI-сообщение со ссылкой (режим обучения).
#[tauri::command]
fn midi_learn(
    index: usize,
    link: String,
    state: tauri::State<'_, AppState>,
) -> Result<DocumentView, String> {
    let last = state
        .midi
        .lock()
        .map_err(|e| e.to_string())?
        .last
        .ok_or_else(|| "MIDI-сообщений ещё не было".to_string())?;

    let mut document = state.document.lock().map_err(|e| e.to_string())?;
    let vmc = document
        .vmc
        .as_mut()
        .ok_or_else(|| "документ не открыт".to_string())?;
    let widget = vmc
        .widgets_data()
        .into_iter()
        .find(|widget| widget.index == index)
        .ok_or_else(|| format!("виджета с индексом {index} нет"))?;
    let mut mappings = widget.midi_map.clone();
    mappings.retain(|entry| entry.link != link);
    mappings.push(vmix_config::MidiMapEntry {
        channel: last.channel,
        number: last.number,
        link,
        kind: last.kind.as_str().to_string(),
    });
    vmc.set_widget_midi_mappings(index, &mappings)
        .map_err(|e| format!("{e:#}"))?;
    view(&document)
}

/// Найти виджет: у контейнера — среди вложенных (`container` = индекс контейнера).
fn find_widget(vmc: &Vmc, index: usize, container: Option<usize>) -> Option<WidgetData> {
    let widgets = vmc.widgets_data();
    match container {
        Some(parent) => widgets
            .into_iter()
            .find(|widget| widget.index == parent)
            .and_then(|widget| {
                widget
                    .children
                    .into_iter()
                    .find(|child| child.index == index)
            }),
        None => widgets.into_iter().find(|widget| widget.index == index),
    }
}

/// Ключ рантайма скриптов: вложенные виджеты контейнеров не пересекаются с верхними.
fn widget_key(index: usize, container: Option<usize>) -> u64 {
    match container {
        Some(parent) => (parent as u64 + 1) * 1_000_000 + index as u64,
        None => index as u64,
    }
}

/// Нажатие на виджет: прогнать его скрипт (условия, задержки, переменные, вызовы vMix).
#[tauri::command]
fn vmix_press(
    index: usize,
    container: Option<usize>,
    connection: Connection,
    state: tauri::State<'_, AppState>,
) -> Result<PressResult, String> {
    let (commands, active, runtime_key) = {
        let document = state.document.lock().map_err(|e| e.to_string())?;
        let vmc = document
            .vmc
            .as_ref()
            .ok_or_else(|| "документ не открыт".to_string())?;
        let widget = find_widget(vmc, index, container).ok_or_else(|| {
            format!("виджета с индексом {index} нет{}", match container {
                Some(parent) => format!(" в контейнере {parent}"),
                None => String::new(),
            })
        })?;
        let commands = commands_of_widget(&widget);
        (commands, widget.active, widget_key(index, container))
    };
    let relays = relay_clients(&state);

    let catalogue = ensure_catalogue(&state)?;
    let client = connection.client();
    // состояние нужно выражениям `_('...')` и подстановкам ключей
    let vmix_state = client
        .fetch_state_xml()
        .ok()
        .and_then(|xml| VmixState::parse(&xml).ok());

    let (locals, tracked) = {
        let runtime = state.runtime.lock().map_err(|e| e.to_string())?;
        let entry = runtime.get(&runtime_key).cloned().unwrap_or_default();
        (entry.locals, entry.tracked)
    };

    let (outcome, locals, tracked) = run_script(
        &client,
        &relays,
        &catalogue,
        &commands,
        vmix_state.as_ref().map(|state| &state.raw),
        active,
        locals,
        tracked,
    )
    .map_err(|e| format!("{e:#}"))?;

    {
        let mut runtime = state.runtime.lock().map_err(|e| e.to_string())?;
        runtime.insert(
            runtime_key,
            WidgetRuntime {
                locals,
                tracked,
            },
        );
    }

    // глобальные переменные, выставленные скриптом, попадают в документ
    if !outcome.globals.is_empty() {
        let mut document = state.document.lock().map_err(|e| e.to_string())?;
        if let Some(vmc) = document.vmc.as_mut() {
            for (name, value) in &outcome.globals {
                let _ = vmc.set_global_variable(name, value);
            }
        }
    }

    Ok(PressResult {
        log: outcome.log,
        active,
        page_delta: outcome.page_delta,
        page: outcome.page,
    })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let startup_path = args.next().map(PathBuf::from).filter(|path| path.exists());
    let startup_select = args.next().and_then(|value| value.parse::<usize>().ok());
    let startup_flag = args.next().unwrap_or_default();
    // picker:<запрос> — dev-режим: открыть редактор скрипта и сразу набрать запрос
    let startup_script = startup_flag == "script" || startup_flag.starts_with("picker:");
    let startup_rows = startup_flag == "rows";
    let startup_midi = startup_flag == "midi";
    let startup_deck = startup_flag == "deck";
    let startup_schedule = startup_flag == "schedule";
    let startup_palette = startup_flag == "palette";
    let startup_picker = startup_flag
        .strip_prefix("picker:")
        .map(str::to_string);

    tauri::Builder::default()
        .manage(AppState {
            startup_path,
            startup_select: Mutex::new(startup_select),
            startup_script: Mutex::new(startup_script),
            startup_rows: Mutex::new(startup_rows),
            startup_midi: Mutex::new(startup_midi),
            startup_deck: Mutex::new(startup_deck),
            startup_schedule: Mutex::new(startup_schedule),
            startup_palette: Mutex::new(startup_palette),
            startup_picker: Mutex::new(startup_picker),
            i18n: Mutex::new(I18n::load()),
            ..Default::default()
        })
        .invoke_handler(tauri::generate_handler![
            vmix_state,
            vmix_call,
            vmix_poll,
            vmix_press,
            vmix_set_commands,
            external_apply,
            vmc_set_external,
            vmc_container_import,
            ndi_stop,
            vmix_dispatch_link,
            midi_ports,
            midi_start,
            midi_stop,
            midi_poll,
            midi_learn,
            vmc_set_hotkey,
            vmc_set_extra,
            vmc_set_events,
            language,
            set_language,
            streamdeck_devices,
            streamdeck_start,
            streamdeck_stop,
            streamdeck_brightness,
            streamdeck_poll,
            streamdeck_learn,
            schedule_poll,
            catalogue_info,
            vmc_export_script,
            vmc_import_script,
            vmc_startup,
            vmc_new,
            vmc_open,
            vmc_save,
            vmc_set_locked,
            vmc_palette,
            vmc_add,
            vmc_update,
            vmc_remove,
            vmc_duplicate
        ])
        .run(tauri::generate_context!())
        .expect("не удалось запустить приложение");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Мок vMix: принимает один запрос, отвечает XML и возвращает строку запроса.
    fn mock_vmix() -> (u16, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("порт");
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("соединение");
            let mut buffer = [0u8; 8192];
            let read = stream.read(&mut buffer).expect("чтение");
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let body = "<vmix><version>29.0.0.1</version></vmix>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("ответ");
            request.lines().next().unwrap_or_default().to_string()
        });
        (port, handle)
    }

    fn send(client: &VmixClient, commands: &[FunctionCommand]) -> ScriptOutcome {
        run_script(
            client,
            &[],
            &Catalogue::default(),
            commands,
            None,
            false,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
        .0
    }

    #[test]
    fn press_sends_rendered_queries_to_vmix() {
        let (port, handle) = mock_vmix();
        let client = VmixClient::new("127.0.0.1", port);
        let command = FunctionCommand {
            function: "SetVolume".into(),
            input: Some(1),
            parameter: Some("55".into()),
            format_string: Some("Function=SetVolume&Input={0}&Value={1}".into()),
            executable: true,
            use_in_active_state: true,
            ..Default::default()
        };

        let outcome = send(&client, &[command]);
        assert!(
            outcome.log[0].starts_with("Function=SetVolume&Input=1&Value=55"),
            "{:?}",
            outcome.log
        );
        assert!(handle
            .join()
            .unwrap()
            .contains("Function=SetVolume&Input=1&Value=55"));
    }

    #[test]
    fn press_uses_the_catalogue_when_command_has_no_format_string() {
        let (port, handle) = mock_vmix();
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let examples = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let catalogue = Catalogue::load(&[dir.join("Functions.xml"), dir.join("NewFunctions.xml")])
            .expect("каталог");

        let command = FunctionCommand {
            function: "SetVolume".into(),
            input: Some(3),
            parameter: Some("20".into()),
            executable: true,
            use_in_active_state: true,
            ..Default::default()
        };

        let (outcome, _, _) = run_script(
            &VmixClient::new("127.0.0.1", port),
            &[],
            &catalogue,
            &[command],
            None,
            false,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        assert!(
            outcome.log[0].starts_with("Function=SetVolume&Input=3&Value=20"),
            "{:?}",
            outcome.log
        );
        assert!(handle.join().unwrap().contains("Input=3&Value=20"));
    }

    #[test]
    fn non_executable_commands_are_skipped() {
        // порт 1 недоступен: если бы команда ушла в vMix, тест упал бы на соединении
        let client = VmixClient::new("127.0.0.1", 1);
        let command = FunctionCommand {
            function: "Cut".into(),
            format_string: Some("Function=Cut".into()),
            executable: false,
            ..Default::default()
        };
        let outcome = send(&client, &[command]);
        assert!(outcome.log.is_empty());
    }

    #[test]
    fn native_functions_stay_inside_the_script_layer() {
        let client = VmixClient::new("127.0.0.1", 1);
        let condition = FunctionCommand {
            function: "Condition".into(),
            format_string: Some("Function=Condition".into()),
            executable: true,
            use_in_active_state: true,
            ..Default::default()
        };
        let condition_end = FunctionCommand {
            function: "ConditionEnd".into(),
            executable: true,
            use_in_active_state: true,
            ..Default::default()
        };
        let next_page = FunctionCommand {
            function: "NextPage".into(),
            executable: true,
            use_in_active_state: true,
            ..Default::default()
        };

        // условие без параметров ложно, поэтому команда между Condition и ConditionEnd
        // не исполняется — как в оригинале; NextPage стоит уже после блока
        let outcome = send(&client, &[condition, condition_end, next_page]);
        assert_eq!(outcome.page_delta, 1);
        assert!(
            outcome.log.iter().any(|line| line.contains("Condition")),
            "{:?}",
            outcome.log
        );
        assert!(
            outcome.log.iter().any(|line| line.contains("следующая страница")),
            "{:?}",
            outcome.log
        );
    }

    /// Реальный скрипт из Scoreboard.vmc целиком: условие по оверлею + ветка Else,
    /// и всё это уходит в мок vMix по HTTP.
    #[test]
    fn real_widget_script_reaches_vmix() {
        let (port, handle) = mock_vmix();
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let examples = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let catalogue = Catalogue::load(&[dir.join("Functions.xml"), dir.join("NewFunctions.xml")])
            .expect("каталог");
        let vmc = Vmc::parse(&std::fs::read(examples.join("Scoreboard.vmc")).unwrap()).unwrap();
        let widget = vmc
            .widgets_data()
            .into_iter()
            .find(|widget| {
                widget
                    .commands
                    .iter()
                    .any(|command| command.function == "Condition")
            })
            .expect("кнопка с условием");
        let commands = commands_of_widget(&widget);

        // оверлей показывает этот вход → условие ложно, сработает ветка Else: OverlayInput1In
        let state_xml = r#"<vmix><inputs>
<input key="e22bcc58-40e1-4770-9559-5ab1614e6574" number="3" type="GT" title="Top"/>
</inputs><overlays><overlay number="1">e22bcc58-40e1-4770-9559-5ab1614e6574</overlay></overlays></vmix>"#;
        let state = vmix_xml::parse(state_xml.as_bytes()).unwrap();

        let mut runner = ScriptRunner::new(&commands, Some(&state));
        runner.catalogue = Some(&catalogue);
        let client = VmixClient::new("127.0.0.1", port);
        let sender = VmixSender::new(&client, &[]);
        runner.run(&sender).unwrap();

        let request = handle.join().unwrap();
        assert!(
            request.contains("Function=OverlayInput1In&Input=e22bcc58-40e1-4770-9559-5ab1614e6574"),
            "в vMix должен уйти вызов оверлея: {request}"
        );
    }

    /// Состояние приложения для тестов: язык фиксирован, чтобы тексты сообщений
    /// не зависели от локали машины (на macOS и Windows раннерах она английская).
    fn test_state(vmc: Vmc) -> AppState {
        AppState {
            document: Mutex::new(Document {
                vmc: Some(vmc),
                ..Default::default()
            }),
            i18n: Mutex::new(crate::i18n::I18n::for_language("ru")),
            ..Default::default()
        }
    }

    /// Мок vMix, который отдаёт состояние и принимает N запросов.
    fn mock_vmix_multi(state_xml: &'static str, requests: usize) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("порт");
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..requests {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut buffer = [0u8; 8192];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                seen.push(request.lines().next().unwrap_or_default().to_string());
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    state_xml.len(),
                    state_xml
                );
                let _ = stream.write_all(response.as_bytes());
            }
            seen
        });
        (port, handle)
    }

    /// Реальный виджет внешних данных из InputSelector.vmc: провайдер читает состояние
    /// vMix по XPath, строка применяется к тайтлу через SetText.
    #[test]
    fn external_widget_reads_provider_and_applies_row() {
        const STATE_XML: &str = r#"<vmix><inputs>
<input key="b45b555d-4b82-4c3a-b27c-6a8aca4b2ae5" number="5" type="GT" title="Headline">
  <text index="1" name="Message.Text">старое</text>
</input>
<input key="k2" number="2" type="GT" title="Subtitle">
  <text index="1" name="Message.Text">другое</text>
</input>
</inputs></vmix>"#;

        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let examples = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let vmc = Vmc::parse(&std::fs::read(examples.join("InputSelector.vmc")).unwrap()).unwrap();
        let widgets = vmc.widgets_data();
        // провайдер живёт в виджете внешних данных…
        let provider_widget = widgets
            .iter()
            .find(|widget| {
                widget
                    .external
                    .as_ref()
                    .map(|external| !external.provider_path.is_empty())
                    .unwrap_or(false)
            })
            .expect("виджет с провайдером");
        let mut external = provider_widget.external.clone().unwrap();
        assert_eq!(
            external.provider_path.rsplit('\\').next().unwrap(),
            "XmlDataProvider.dll"
        );
        assert!(!external.provider_properties.is_empty());

        // …а пути и ссылка на источник — у виджета-списка
        let list_widget = widgets
            .iter()
            .find(|widget| {
                widget
                    .external
                    .as_ref()
                    .map(|external| !external.paths.is_empty() && !external.source_name.is_empty())
                    .unwrap_or(false)
            })
            .expect("виджет-потребитель с путями");
        let list_external = list_widget.external.clone().unwrap();
        assert_eq!(list_external.source_name, provider_widget.name);

        // провайдер и применение делают 3 запроса: состояние провайдеру, состояние для
        // поиска элементов тайтла и сам SetText
        let (port, handle) = mock_vmix_multi(STATE_XML, 3);
        external.provider_properties[0] = format!("http://127.0.0.1:{port}/api");

        let mut provider = XmlDataProvider::from_properties(&external.provider_properties);
        provider.refresh().unwrap();
        let values = provider.values().to_vec();
        assert!(!values.is_empty(), "провайдер не вернул значений");
        assert_eq!(
            values[0],
            "5|Headline",
            "первая строка должна быть «номер|название»: {values:?}"
        );

        let catalogue = Catalogue::load(&[dir.join("Functions.xml"), dir.join("NewFunctions.xml")])
            .expect("каталог");
        let client = VmixClient::new("127.0.0.1", port);
        let log = apply_external_row(&client, &catalogue, &list_external, &values, 0);
        assert!(!log.is_empty(), "журнал пуст");

        let requests = handle.join().unwrap();
        let set_text = requests
            .iter()
            .find(|line| line.contains("SetText"))
            .unwrap_or_else(|| panic!("SetText не отправлен: {requests:?}"));
        assert!(set_text.contains("SelectedIndex=1"), "{set_text}");
        assert!(set_text.contains("Input=b45b555d-4b82-4c3a-b27c-6a8aca4b2ae5"), "{set_text}");
        assert!(set_text.contains("Value=5%7CHeadline"), "значение строки: {set_text}");
    }

    /// `vMixControlMultiState`: включённый ретранслятор получает те же функции.
    #[test]
    fn relay_receives_the_same_function() {
        let (main_port, main_handle) = mock_vmix();
        let (relay_port, relay_handle) = mock_vmix();
        let relays = vec![(
            format!("127.0.0.1:{relay_port}"),
            VmixClient::new("127.0.0.1", relay_port),
        )];

        let command = FunctionCommand {
            function: "Cut".into(),
            input: Some(3),
            format_string: Some("Function=Cut&Input={0}".into()),
            executable: true,
            use_in_active_state: true,
            ..Default::default()
        };

        let (outcome, _, _) = run_script(
            &VmixClient::new("127.0.0.1", main_port),
            &relays,
            &Catalogue::default(),
            &[command],
            None,
            false,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();

        assert!(main_handle.join().unwrap().contains("Function=Cut&Input=3"));
        assert!(
            relay_handle.join().unwrap().contains("Function=Cut&Input=3"),
            "ретранслятор не получил функцию"
        );
        assert!(
            outcome.log.iter().any(|line| line.starts_with("↔ ")),
            "журнал без пометки ретрансляции: {:?}",
            outcome.log
        );
    }

    /// Ссылка (`Hotkey.Link`) запускает скрипт кнопки — как MIDI/Stream Deck в оригинале.
    #[test]
    fn hotkey_link_runs_button_script() {
        let (port, handle) = mock_vmix();
        let client = VmixClient::new("127.0.0.1", port);

        let mut vmc = Vmc::empty();
        let index = vmc.create_widget(&WidgetKind::Button, 10.0, 10.0).unwrap();
        vmc.set_widget_commands(
            index,
            &[WidgetCommand {
                function: "Cut".into(),
                format_string: "Function=Cut&Input={0}".into(),
                input: Some(3),
                executable: true,
                use_in_active_state: true,
                ..Default::default()
            }],
        )
        .unwrap();
        // включаем ссылку у действия «Execute»
        vmc.set_widget_hotkey_link(index, 0, "Play.Execute").unwrap();

        let app = test_state(vmc);

        let log = dispatch_link(
            &app,
            &client,
            &Catalogue::default(),
            None,
            "Play.Execute",
            Some(127),
        );
        assert!(
            log.iter().any(|line| line.contains("Function=Cut&Input=3")),
            "скрипт кнопки не выполнился: {log:?}"
        );
        assert!(
            handle.join().unwrap().contains("Function=Cut&Input=3"),
            "в vMix не ушла функция"
        );

        // неизвестная ссылка — понятная запись в журнале.
        // Проверяем по имени ссылки, а не по тексту сообщения: он локализован,
        // и на английской локали (macOS/Windows раннеры) формулировка другая.
        let empty = dispatch_link(
            &app,
            &client,
            &Catalogue::default(),
            None,
            "нет.такой",
            None,
        );
        assert!(
            empty.iter().any(|line| line.contains("нет.такой")),
            "в журнале нет сообщения о неизвестной ссылке: {empty:?}"
        );
    }

    /// Stream Deck: ссылку запускает **отпускание** кнопки (как `KeyUp` в оригинале).
    #[test]
    fn streamdeck_key_up_resolves_link() {
        let mut vmc = Vmc::empty();
        let index = vmc
            .create_widget(&WidgetKind::StreamDeck, 10.0, 10.0)
            .unwrap();
        vmc.set_widget_deck_keys(
            index,
            &[vmix_config::DeckKey {
                context: "5".into(),
                link: "Play.Execute".into(),
                index: 5,
                extra: 0,
            }],
        )
        .unwrap();

        let up = DeckEvent {
            kind: DeckEventKind::KeyUp,
            index: 5,
        };
        assert_eq!(deck_link_targets(&vmc, &up), vec!["Play.Execute".to_string()]);
        assert!(
            deck_link_targets(
                &vmc,
                &DeckEvent {
                    kind: DeckEventKind::KeyDown,
                    index: 5
                }
            )
            .is_empty(),
            "на нажатие ссылка не срабатывает"
        );
        assert!(
            deck_link_targets(
                &vmc,
                &DeckEvent {
                    kind: DeckEventKind::KeyUp,
                    index: 6
                }
            )
            .is_empty(),
            "другая кнопка ничего не запускает"
        );
    }

    /// Планировщик: срабатывает по дням и времени, и только один раз в сутки.
    #[test]
    fn scheduler_fires_once_per_day_on_planned_weekday() {
        use chrono::Weekday;
        use vmix_config::{ScheduledEvent, EVERY_DAY};

        let events = vec![
            (
                0usize,
                0usize,
                ScheduledEvent {
                    time: "09:30".into(),
                    command: "Play.Execute".into(),
                    days: EVERY_DAY,
                },
            ),
            (
                0usize,
                1usize,
                ScheduledEvent {
                    time: "20:00".into(),
                    command: "Evening.Execute".into(),
                    days: 0b0000_0101, // пн и ср
                },
            ),
        ];
        let mut state = ScheduleState::default();

        // понедельник, 09:00 — рано
        assert!(schedule_due(&events, &mut state, "2026-09-21", 9 * 60, Weekday::Mon, false).is_empty());

        // 09:30 — первое событие
        let due = schedule_due(&events, &mut state, "2026-09-21", 9 * 60 + 30, Weekday::Mon, false);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].2, "Play.Execute");

        // повторно в тот же день — уже не срабатывает
        assert!(schedule_due(&events, &mut state, "2026-09-21", 10 * 60, Weekday::Mon, false).is_empty());

        // 20:00 — второе событие (понедельник подходит)
        let due = schedule_due(&events, &mut state, "2026-09-21", 20 * 60, Weekday::Mon, false);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].2, "Evening.Execute");

        // вторник: второе событие не запланировано, а первое уже сработало сегодня...
        let mut tuesday = ScheduleState::default();
        assert!(schedule_due(&events, &mut tuesday, "2026-09-22", 21 * 60, Weekday::Tue, false)
            .iter()
            .all(|(_, position, _)| *position == 0));

        // новый день сбрасывает срабатывания
        let mut wednesday = ScheduleState::default();
        let due = schedule_due(&events, &mut wednesday, "2026-09-23", 21 * 60, Weekday::Wed, false);
        assert_eq!(due.len(), 2, "в среду должны сработать оба: {due:?}");
    }

    /// `ExecLink` внутри скрипта запускает ссылку другого виджета (как в оригинале).
    #[test]
    fn exec_link_runs_another_widget_script() {
        let (port, handle) = mock_vmix();
        let client = VmixClient::new("127.0.0.1", port);

        let mut vmc = Vmc::empty();
        let first = vmc.create_widget(&WidgetKind::Button, 0.0, 0.0).unwrap();
        vmc.set_widget_commands(
            first,
            &[WidgetCommand {
                function: "ExecLink".into(),
                format_string: "Function=ExecLink".into(),
                string_parameter: Some("'B.Execute'".into()),
                executable: true,
                use_in_active_state: true,
                ..Default::default()
            }],
        )
        .unwrap();
        vmc.set_widget_hotkey_link(first, 0, "A.Execute").unwrap();

        let second = vmc.create_widget(&WidgetKind::Button, 0.0, 100.0).unwrap();
        vmc.set_widget_commands(
            second,
            &[WidgetCommand {
                function: "Cut".into(),
                format_string: "Function=Cut&Input={0}".into(),
                input: Some(3),
                executable: true,
                use_in_active_state: true,
                ..Default::default()
            }],
        )
        .unwrap();
        vmc.set_widget_hotkey_link(second, 0, "B.Execute").unwrap();

        let app = test_state(vmc);

        let log = dispatch_link(&app, &client, &Catalogue::default(), None, "A.Execute", None);
        assert!(
            log.iter().any(|line| line.contains("ExecLink")),
            "ExecLink не отработал: {log:?}"
        );
        assert!(
            log.iter().any(|line| line.contains("B.Execute")),
            "цепочка ссылок не продолжилась: {log:?}"
        );
        assert!(
            handle.join().unwrap().contains("Function=Cut&Input=3"),
            "скрипт второго виджета не выполнился"
        );
    }

    #[test]
    fn widget_states_come_from_vmix_state() {
        // Команда кнопки в Scoreboard.vmc: OverlayInputX, Parameter=1,
        // InputKey=2d74ef94-…, ActiveStatePath=Overlays[{3}].ActiveInput.
        const STATE_ON: &str = r#"<vmix><inputs>
<input key="2d74ef94-182f-4688-9f98-1e2138435092" number="1" type="Video" title="Scoreboard" state="Running"/>
</inputs>
<overlays><overlay number="1">2d74ef94-182f-4688-9f98-1e2138435092</overlay></overlays>
<recording>False</recording><streaming>False</streaming><audio><master volume="100" muted="False"/></audio>
<active>1</active></vmix>"#;
        const STATE_OFF: &str = r#"<vmix><inputs>
<input key="2d74ef94-182f-4688-9f98-1e2138435092" number="1" type="Video" title="Scoreboard" state="Running"/>
</inputs>
<overlays><overlay number="1"/></overlays>
<recording>False</recording><streaming>False</streaming><audio><master volume="100" muted="False"/></audio>
<active>1</active></vmix>"#;

        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let examples = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let catalogue = Catalogue::load(&[dir.join("Functions.xml"), dir.join("NewFunctions.xml")])
            .expect("каталог");
        let mut vmc = Vmc::parse(&std::fs::read(examples.join("Scoreboard.vmc")).unwrap()).unwrap();

        let state_on = vmix_xml::parse(STATE_ON.as_bytes()).unwrap();
        let widgets = widget_states(&mut vmc, &catalogue, &state_on);
        let active: Vec<usize> = widgets
            .iter()
            .filter(|widget| widget.active)
            .map(|widget| widget.index)
            .collect();
        assert!(!active.is_empty(), "ни один виджет не подсветился");
        let lit = widgets.iter().find(|widget| widget.active).unwrap();
        assert!(
            lit.commands.iter().any(|command| command.function == "OverlayInputX"),
            "подсветился виджет без команды оверлея: {lit:?}"
        );

        // оверлей пуст → подсветки нет
        let state_off = vmix_xml::parse(STATE_OFF.as_bytes()).unwrap();
        let widgets = widget_states(&mut vmc, &catalogue, &state_off);
        assert!(
            widgets.iter().all(|widget| !widget.active),
            "осталась подсветка без активного оверлея"
        );
    }
}
