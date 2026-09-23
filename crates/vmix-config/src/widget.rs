//! Типизированное представление виджетов и правка `.vmc`.
//!
//! Виджет в файле — это узел `<vMixControl xsi:type="…">` с общим набором свойств
//! (`Name`, `Left/Top/Width/Height`, `ZIndex`, `Page`, `Color`, `BorderColor`,
//! `IsCaptionVisible`, `Locked`, …) и свойствами конкретного типа. Наборы свойств
//! сняты с реальных файлов из `examples`, поэтому созданные портом
//! виджеты читаются оригинальным приложением (`XmlSerializer` игнорирует отсутствующие
//! элементы, оставляя значения по умолчанию).

use crate::{Vmc, CONTROLS_PATH};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use vmix_xml::Node;

// --------------------------------------------------------------- типы виджетов

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WidgetKind {
    Region,
    Button,
    NewButton,
    TextField,
    Score,
    Timer,
    List,
    Playlist,
    Label,
    TBar,
    Clock,
    Volume,
    /// Устаревший алиас громкости (`vMixControlSlider : vMixControlVolume`).
    Slider,
    VariableViewer,
    Container,
    MidiInterface,
    StreamDeck,
    ExternalData,
    MultiState,
    Other(String),
}

impl WidgetKind {
    /// Что предлагать в меню «добавить виджет» на рабочей поверхности.
    pub const PALETTE: [WidgetKind; 14] = [
        WidgetKind::Region,
        WidgetKind::Button,
        WidgetKind::TextField,
        WidgetKind::Score,
        WidgetKind::Timer,
        WidgetKind::List,
        WidgetKind::Playlist,
        WidgetKind::Clock,
        WidgetKind::Volume,
        WidgetKind::TBar,
        WidgetKind::VariableViewer,
        WidgetKind::Container,
        WidgetKind::MidiInterface,
        WidgetKind::StreamDeck,
    ];

    pub fn from_type_name(name: &str) -> Self {
        match name {
            "vMixControlRegion" => Self::Region,
            "vMixControlButton" => Self::Button,
            "vMixControlNewButton" => Self::NewButton,
            "vMixControlTextField" => Self::TextField,
            "vMixControlScore" => Self::Score,
            "vMixControlTimer" => Self::Timer,
            "vMixControlList" => Self::List,
            "vMixControlPlaylist" => Self::Playlist,
            "vMixControlLabel" => Self::Label,
            "vMixControlTBar" => Self::TBar,
            "vMixControlClock" => Self::Clock,
            "vMixControlVolume" => Self::Volume,
            "vMixControlVariableViewer" => Self::VariableViewer,
            "vMixControlSlider" => Self::Slider,
            "vMixControlContainer" => Self::Container,
            "vMixControlMidiInterface" => Self::MidiInterface,
            "vMixControlStreamDeck" => Self::StreamDeck,
            "vMixControlExternalData" => Self::ExternalData,
            "vMixControlMultiState" => Self::MultiState,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn type_name(&self) -> String {
        match self {
            Self::Region => "vMixControlRegion".into(),
            Self::Button => "vMixControlButton".into(),
            Self::NewButton => "vMixControlNewButton".into(),
            Self::TextField => "vMixControlTextField".into(),
            Self::Score => "vMixControlScore".into(),
            Self::Timer => "vMixControlTimer".into(),
            Self::List => "vMixControlList".into(),
            Self::Playlist => "vMixControlPlaylist".into(),
            Self::Label => "vMixControlLabel".into(),
            Self::TBar => "vMixControlTBar".into(),
            Self::Clock => "vMixControlClock".into(),
            Self::Volume => "vMixControlVolume".into(),
            Self::Slider => "vMixControlSlider".into(),
            Self::VariableViewer => "vMixControlVariableViewer".into(),
            Self::Container => "vMixControlContainer".into(),
            Self::MidiInterface => "vMixControlMidiInterface".into(),
            Self::StreamDeck => "vMixControlStreamDeck".into(),
            Self::ExternalData => "vMixControlExternalData".into(),
            Self::MultiState => "vMixControlMultiState".into(),
            Self::Other(name) => name.clone(),
        }
    }

    /// Ключ локализации подписи (`kind.button` и т. п.) — строки живут в словарях UI.
    pub fn localization_key(&self) -> &'static str {
        match self {
            Self::Region => "kind.region",
            Self::Button => "kind.button",
            Self::NewButton => "kind.newButton",
            Self::TextField => "kind.textField",
            Self::Score => "kind.score",
            Self::Timer => "kind.timer",
            Self::List => "kind.list",
            Self::Playlist => "kind.playlist",
            Self::Label => "kind.label",
            Self::TBar => "kind.tbar",
            Self::Clock => "kind.clock",
            Self::Volume | Self::Slider => "kind.volume",
            Self::VariableViewer => "kind.variableViewer",
            Self::Container => "kind.container",
            Self::MidiInterface => "kind.midi",
            Self::StreamDeck => "kind.streamDeck",
            Self::ExternalData => "kind.externalData",
            Self::MultiState => "kind.other",
            Self::Other(_) => "kind.other",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Region => "Область".into(),
            Self::Button => "Кнопка".into(),
            Self::NewButton => "Кнопка (новая)".into(),
            Self::TextField => "Текст".into(),
            Self::Score => "Счёт".into(),
            Self::Timer => "Таймер".into(),
            Self::List => "Список".into(),
            Self::Playlist => "Плейлист".into(),
            Self::Label => "Надпись".into(),
            Self::Clock => "Часы".into(),
            Self::Volume | Self::Slider => "Громкость".into(),
            Self::TBar => "TBar".into(),
            Self::VariableViewer => "Переменные".into(),
            Self::Container => "Контейнер".into(),
            Self::MidiInterface => "MIDI".into(),
            Self::StreamDeck => "Stream Deck".into(),
            Self::ExternalData => "Внешние данные".into(),
            Self::MultiState => "Мультистейт".into(),
            Self::Other(name) => name.trim_start_matches("vMixControl").to_string(),
        }
    }

    pub fn default_size(&self) -> (f64, f64) {
        match self {
            Self::Region => (200.0, 100.0),
            Self::Button | Self::NewButton => (120.0, 80.0),
            Self::TextField | Self::Score | Self::Timer | Self::Clock => (200.0, 60.0),
            Self::Volume | Self::Slider => (120.0, 300.0),
            Self::TBar => (320.0, 60.0),
            Self::VariableViewer => (220.0, 80.0),
            Self::Container => (320.0, 200.0),
            Self::MidiInterface => (240.0, 140.0),
            Self::StreamDeck => (200.0, 160.0),
            Self::List => (200.0, 120.0),
            Self::Playlist => (240.0, 160.0),
            Self::Label => (160.0, 40.0),
            _ => (160.0, 80.0),
        }
    }

    pub fn default_name(&self) -> String {
        self.label()
    }

    pub fn default_color(&self) -> Rgba {
        match self {
            Self::Region => Rgba::new(26, 60, 117),
            Self::Button | Self::NewButton => Rgba::new(61, 139, 253),
            Self::Score => Rgba::new(30, 35, 40),
            _ => Rgba::new(35, 43, 51),
        }
    }

    pub fn default_border_color(&self) -> Rgba {
        match self {
            Self::Region => Rgba::new(24, 72, 140),
            Self::Button | Self::NewButton => Rgba::new(42, 110, 208),
            Self::Score => Rgba::new(224, 167, 42),
            _ => Rgba::new(70, 84, 98),
        }
    }
}

// --------------------------------------------------------------- цвет

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { a: 255, r, g, b }
    }

    /// `#RRGGBB` для интерфейса.
    /// Строка A/R/G/B — для отпечатков и сравнений кэша.
    pub fn to_argb(&self) -> String {
        format!("{}/{}/{}/{}", self.a, self.r, self.g, self.b)
    }

    pub fn to_css(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Так же, как WPF-`Color` в `.vmc`: байты + scRGB-компоненты.
    fn write_into(&self, name: &str, node: &mut Node) {
        let mut color = Node::new(name);
        color.set_child_text("A", &self.a.to_string());
        color.set_child_text("R", &self.r.to_string());
        color.set_child_text("G", &self.g.to_string());
        color.set_child_text("B", &self.b.to_string());
        color.set_child_text("ScA", "1");
        color.set_child_text("ScR", &format!("{}", self.r as f64 / 255.0));
        color.set_child_text("ScG", &format!("{}", self.g as f64 / 255.0));
        color.set_child_text("ScB", &format!("{}", self.b as f64 / 255.0));
        if let Some(existing) = node.child_mut(name) {
            *existing = color;
        } else {
            node.children.push(color);
        }
    }

    fn read_from(node: &Node, name: &str) -> Option<Self> {
        let color = node.child(name)?;
        let get = |part: &str| color.child_text(part)?.parse::<u8>().ok();
        Some(Self {
            a: get("A").unwrap_or(255),
            r: get("R")?,
            g: get("G")?,
            b: get("B")?,
        })
    }
}

// --------------------------------------------------------------- данные виджета

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetData {
    /// Позиция в файле — идентификатор для правок из интерфейса.
    pub index: usize,
    pub kind: WidgetKind,
    pub type_name: String,
    pub label: String,
    pub name: String,
    pub page: i64,
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
    pub z_index: i64,
    pub locked: bool,
    pub caption_visible: bool,
    pub caption_on: bool,
    pub caption_height: f64,
    pub scale: f64,
    pub text: String,
    /// Строки списка (`<Items><string>`): формат `ключ|текст`, как в оригинале.
    pub items: Vec<String>,
    pub color: Rgba,
    pub border_color: Rgba,
    /// `Style` кнопки (`Momentary`, `Toggle`, …) и её «нажатое» состояние.
    pub style: String,
    pub active: bool,
    /// Команды кнопки — исполняются по нажатию.
    pub commands: Vec<WidgetCommand>,
    /// Настройки внешних данных (только у соответствующего виджета).
    pub external: Option<ExternalData>,
    /// Свойства конкретных типов виджетов, которые порт уже использует
    /// (`Target`/`InputKey` у громкости, `Mode` у TBar, `Variable` у просмотра переменных…).
    pub extras: BTreeMap<String, String>,
    /// Вложенные виджеты контейнера (`vMixControlContainer.Controls`).
    pub children: Vec<WidgetData>,
    /// Горячие клавиши и ссылки виджета (`Hotkey`): по ссылке их находят MIDI, Stream Deck
    /// и другие виджеты (`ProcessHotkey` в оригинале).
    pub hotkeys: Vec<WidgetHotkey>,
    /// MIDI-отображения виджета (`Midis`): канал, номер, ссылка, тип события.
    pub midi_map: Vec<MidiMapEntry>,
    /// Кнопки Stream Deck (`Keys`): индекс кнопки → ссылка.
    pub deck_keys: Vec<DeckKey>,
    /// Расписание часов (`Events`): время, дни недели и ссылка на запуск.
    pub events: Vec<ScheduledEvent>,
}

/// Событие расписания часов (`ScheduledEvent` в оригинале).
///
/// В C# это `{ DateTime TimeOfDay, string Command, DaysOfWeek Days }`, где `Command` —
/// **имя ссылки**, а дни недели — флаги (`Monday = 1 … Sunday = 64`, `Everyday = 127`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledEvent {
    /// Время в виде `HH:MM` (из `<TimeOfDay>` берём только время суток).
    pub time: String,
    /// Имя ссылки, которую запускает событие.
    pub command: String,
    /// Битовая маска дней недели (пн = 1 … вс = 64).
    pub days: u8,
}

/// Маска «каждый день».
pub const EVERY_DAY: u8 = 0b0111_1111;

impl ScheduledEvent {
    fn from_node(node: &Node) -> Self {
        let raw_time = node.child_text("TimeOfDay").unwrap_or_default().trim();
        // .NET пишет `2024-05-01T09:30:00`, но встречается и просто `09:30:00`
        let time = raw_time
            .split_once('T')
            .map(|(_, time)| time)
            .unwrap_or(raw_time)
            .chars()
            .take(5)
            .collect::<String>();
        Self {
            time,
            command: node.child_text("Command").unwrap_or_default().to_string(),
            days: parse_days(node.child_text("Days").unwrap_or_default()),
        }
    }

    /// Минуты от начала суток (для сравнения с текущим временем).
    pub fn minutes(&self) -> Option<u32> {
        let (hours, minutes) = self.time.split_once(':')?;
        let hours: u32 = hours.trim().parse().ok()?;
        let minutes: u32 = minutes.trim().parse().ok()?;
        (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
    }

    /// Запланировано ли событие на указанный день (`chrono::Weekday`).
    pub fn runs_on(&self, weekday: chrono::Weekday) -> bool {
        use chrono::Weekday::*;
        let bit = match weekday {
            Mon => 1,
            Tue => 2,
            Wed => 4,
            Thu => 8,
            Fri => 16,
            Sat => 32,
            Sun => 64,
        };
        self.days == 0 || self.days & bit != 0
    }
}

/// Разбор `Days`: имена (`Monday, Wednesday`), `Everyday` или число.
pub fn parse_days(value: &str) -> u8 {
    let value = value.trim();
    if value.is_empty() {
        return EVERY_DAY;
    }
    if let Ok(number) = value.parse::<u8>() {
        return number;
    }
    let mut mask = 0u8;
    for part in value.split(',') {
        mask |= match part.trim().to_ascii_lowercase().as_str() {
            "monday" => 1,
            "tuesday" => 2,
            "wednesday" => 4,
            "thursday" => 8,
            "friday" => 16,
            "saturday" => 32,
            "sunday" => 64,
            "everyday" | "every day" | "all" => EVERY_DAY,
            _ => 0,
        };
    }
    if mask == 0 {
        EVERY_DAY
    } else {
        mask
    }
}

/// Имена дней для интерфейса.
pub fn days_label(mask: u8) -> String {
    if mask == EVERY_DAY || mask == 0 {
        return "каждый день".to_string();
    }
    let names = [
        (1, "пн"),
        (2, "вт"),
        (4, "ср"),
        (8, "чт"),
        (16, "пт"),
        (32, "сб"),
        (64, "вс"),
    ];
    names
        .iter()
        .filter(|(bit, _)| mask & bit != 0)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Запись `<StreamDeckKey>`: в C# — `Quadriple<string A, string B, int C, int D>`,
/// где `A` — контекст кнопки, `B` — ссылка (`execLink`), `C`/`D` — служебные числа.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckKey {
    /// Контекст из оригинала (у прямого HID — индекс кнопки строкой).
    pub context: String,
    /// Имя ссылки, которую запускает кнопка.
    pub link: String,
    pub index: i64,
    pub extra: i64,
}

impl DeckKey {
    fn from_node(node: &Node) -> Self {
        let number = |tag: &str| {
            node.child_text(tag)
                .and_then(|value| value.trim().parse::<i64>().ok())
                .unwrap_or(0)
        };
        // A — контекст, но у прямого HID это может быть номер кнопки
        let context = node.child_text("A").unwrap_or_default().to_string();
        Self {
            index: context
                .trim()
                .parse::<i64>()
                .unwrap_or_else(|_| number("C")),
            context,
            link: node.child_text("B").unwrap_or_default().to_string(),
            extra: number("D"),
        }
    }
}

/// Запись `<MidiInterfaceKey>` у виджета MIDI-интерфейса
/// (в C# — `Quadriple<int A, int B, string C, MidiEventType D>`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiMapEntry {
    pub channel: u8,
    pub number: u8,
    pub link: String,
    pub kind: String,
}

impl MidiMapEntry {
    fn from_node(node: &Node) -> Self {
        let number = |tag: &str| {
            node.child_text(tag)
                .and_then(|value| value.trim().parse::<u8>().ok())
                .unwrap_or(0)
        };
        Self {
            channel: number("A"),
            number: number("B"),
            link: node.child_text("C").unwrap_or_default().to_string(),
            kind: node
                .child_text("D")
                .unwrap_or("NoteOn")
                .trim()
                .to_string(),
        }
    }
}

/// Одна запись `<Hotkey>` внутри виджета.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetHotkey {
    pub name: String,
    /// Имя ссылки: по нему сообщение (MIDI/Stream Deck/скрипт) находит действие.
    pub link: String,
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub on_press: bool,
    pub active: bool,
}

impl WidgetHotkey {
    fn from_node(node: &Node) -> Self {
        let text = |tag: &str| node.child_text(tag).unwrap_or_default().to_string();
        let flag = |tag: &str, fallback: bool| match node.child_text(tag) {
            Some("true") | Some("True") | Some("1") => true,
            Some("false") | Some("False") | Some("0") => false,
            _ => fallback,
        };
        Self {
            name: text("Name"),
            link: text("Link"),
            // ключ в оригинале — перечисление WPF (`None`, `F1`, `A`…)
            key: text("Key"),
            ctrl: flag("Ctrl", false),
            alt: flag("Alt", false),
            shift: flag("Shift", false),
            on_press: flag("OnPress", true),
            active: flag("Active", false),
        }
    }
}

/// Теги, которые порт читает как «дополнительные свойства» виджета.
/// Список явный: иначе в него попали бы огромные блобы вроде `DataProviderContent`.
pub const EXTRA_TAGS: &[&str] = &[
    "Target",
    "InputKey",
    "Style",
    "Mode",
    "ShowMeter",
    "ShowMeters",
    "ShowSlider",
    "Variable",
    "ShowVariableName",
    "ShowSeconds",
    "ShowDate",
    "UseUTC",
    "IsVertical",
    "IsVolumeOnly",
    "NextEventAt",
    // ретранслятор (vMixControlMultiState)
    "IP",
    "Port",
    "Login",
    "Password",
    "Enabled",
    // контейнер (vMixControlContainer)
    "FilePath",
];

impl WidgetData {
    pub fn from_node(index: usize, node: &Node) -> Self {
        let kind = node
            .attr_local("type")
            .map(WidgetKind::from_type_name)
            .unwrap_or(WidgetKind::Other("unknown".into()));
        let number = |name: &str, fallback: f64| {
            node.child_text(name)
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(fallback)
        };
        let flag = |name: &str, fallback: bool| match node.child_text(name) {
            Some("true") | Some("True") | Some("1") => true,
            Some("false") | Some("False") | Some("0") => false,
            _ => fallback,
        };
        Self {
            index,
            type_name: kind.type_name(),
            label: kind.label(),
            kind: kind.clone(),
            name: node.child_text("Name").unwrap_or_default().to_string(),
            page: number("Page", 0.0) as i64,
            left: number("Left", 0.0),
            top: number("Top", 0.0),
            width: number("Width", kind.default_size().0),
            height: number("Height", kind.default_size().1),
            z_index: number("ZIndex", 0.0) as i64,
            locked: flag("Locked", false),
            caption_visible: flag("IsCaptionVisible", true),
            caption_on: flag("IsCaptionOn", true),
            caption_height: number("CaptionHeight", 0.0),
            scale: number("Scale", 1.0),
            text: node.child_text("Text").unwrap_or_default().to_string(),
            items: node
                .child("Items")
                .map(|items| {
                    items
                        .children
                        .iter()
                        .filter(|child| child.name == "string")
                        .map(|child| child.text.clone())
                        .collect()
                })
                .unwrap_or_default(),
            color: Rgba::read_from(node, "Color").unwrap_or_else(|| kind.default_color()),
            border_color: Rgba::read_from(node, "BorderColor")
                .unwrap_or_else(|| kind.default_border_color()),
            style: node.child_text("Style").unwrap_or_default().to_string(),
            active: flag("Active", false),
            commands: node
                .child("Commands")
                .map(|commands| {
                    commands
                        .children
                        .iter()
                        .map(WidgetCommand::from_node)
                        .collect()
                })
                .unwrap_or_default(),
            external: matches!(kind, WidgetKind::ExternalData | WidgetKind::List)
                .then(|| ExternalData::from_node(node)),
            events: node
                .child("Events")
                .map(|events| {
                    events
                        .children_named("ScheduledEvent")
                        .map(ScheduledEvent::from_node)
                        .collect()
                })
                .unwrap_or_default(),
            deck_keys: node
                .child("Keys")
                .map(|keys| {
                    keys.children
                        .iter()
                        .map(DeckKey::from_node)
                        .collect()
                })
                .unwrap_or_default(),
            midi_map: node
                .child("Midis")
                .map(|midis| {
                    midis
                        .children
                        .iter()
                        .map(MidiMapEntry::from_node)
                        .collect()
                })
                .unwrap_or_default(),
            hotkeys: node
                .child("Hotkey")
                .map(|hotkeys| {
                    hotkeys
                        .children_named("Hotkey")
                        .map(WidgetHotkey::from_node)
                        .collect()
                })
                .unwrap_or_default(),
            children: match kind {
                WidgetKind::Container => node
                    .child("Controls")
                    .map(|controls| {
                        controls
                            .children
                            .iter()
                            .enumerate()
                            .map(|(position, child)| WidgetData::from_node(position, child))
                            .collect()
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            },
            extras: EXTRA_TAGS
                .iter()
                .filter_map(|tag| {
                    node.child_text(tag)
                        .filter(|value| !value.is_empty())
                        .map(|value| ((*tag).to_string(), value.to_string()))
                })
                .collect(),
        }
    }
}

/// Куда ретранслировать функции vMix (`vMixControlMultiState`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayTarget {
    pub ip: String,
    pub port: u16,
    pub login: String,
    pub password: String,
}

/// Настройки виджета внешних данных (`vMixControlExternalData`).
///
/// `DataProviderContent` (встроенная .NET-сборка) портом не читается и не пишется —
/// узел остаётся в файле как есть, поэтому `.vmc` совместим с оригиналом в обе стороны.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalData {
    pub is_live: bool,
    pub is_table: bool,
    /// В `.vmc` (и в интерфейсе) поле называется `IsMappedToGUID`: serde сам такое
    /// сокращение не соберёт (`isMappedToGuid`), поэтому имя задано явно.
    #[serde(rename = "isMappedToGUID")]
    pub is_mapped_to_guid: bool,
    pub text: String,
    pub enabled: bool,
    pub restart_data: bool,
    pub period_ms: i64,
    pub provider_path: String,
    pub provider_properties: Vec<String>,
    /// Пары «вход (ключ) → имя элемента тайтла» из `<Paths>`.
    pub paths: Vec<(String, String)>,
    /// Ссылка на источник данных у виджетов-потребителей (`DataSource`):
    /// `(имя виджета-провайдера, имя набора)`.
    pub source_name: String,
    pub source_data: String,
}

impl ExternalData {
    pub fn from_node(node: &Node) -> Self {
        let text = |name: &str| node.child_text(name).unwrap_or_default().to_string();
        let flag = |name: &str, fallback: bool| match node.child_text(name) {
            Some("true") | Some("True") | Some("1") => true,
            Some("false") | Some("False") | Some("0") => false,
            _ => fallback,
        };
        Self {
            is_live: flag("IsLive", false),
            is_table: flag("IsTable", false),
            is_mapped_to_guid: flag("IsMappedToGUID", false),
            text: text("Text"),
            enabled: flag("Enabled", true),
            restart_data: flag("RestartData", true),
            period_ms: text("Period").parse().unwrap_or(1000),
            provider_path: text("DataProviderPath"),
            provider_properties: node
                .child("DataProviderProperties")
                .map(|properties| {
                    properties
                        .children
                        .iter()
                        .map(|item| item.text.clone())
                        .collect()
                })
                .unwrap_or_default(),
            source_name: node
                .child("DataSource")
                .and_then(|source| source.child_text("A"))
                .unwrap_or_default()
                .to_string(),
            source_data: node
                .child("DataSource")
                .and_then(|source| source.child_text("B"))
                .unwrap_or_default()
                .to_string(),
            paths: node
                .child("Paths")
                .map(|paths| {
                    paths
                        .children_named("PairOfStringString")
                        .map(|pair| {
                            (
                                pair.child_text("A").unwrap_or_default().to_string(),
                                pair.child_text("B").unwrap_or_default().to_string(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Провайдер, определённый по пути из `.vmc`.
    pub fn provider_kind(&self) -> &str {
        &self.provider_path
    }

    /// Записать настройки в узел виджета (встроенную сборку не трогаем).
    pub fn write_into(&self, node: &mut Node) {
        node.set_child_text("DataProviderPath", &self.provider_path);
        let mut properties = Node::new("DataProviderProperties");
        for value in &self.provider_properties {
            let mut item = Node::new("anyType");
            item.text = value.clone();
            properties.children.push(item);
        }
        match node.child_mut("DataProviderProperties") {
            Some(existing) => *existing = properties,
            None => node.children.push(properties),
        }
        node.set_child_text("Period", &self.period_ms.to_string());
        node.set_child_text("Enabled", if self.enabled { "true" } else { "false" });
        node.set_child_text(
            "RestartData",
            if self.restart_data { "true" } else { "false" },
        );
    }
}

/// Команда кнопки — то, что в оригинале лежит в `<Commands>` и исполняется по нажатию.
/// Поля соответствуют `vMixControlButtonCommand` / `vMixControlNewButtonCommand`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetCommand {
    pub function: String,
    pub description: String,
    /// `Action.FormatString` из `.vmc` — по нему строится запрос к vMix.
    pub format_string: String,
    pub native: bool,
    pub input: Option<i64>,
    pub input_key: Option<String>,
    pub parameter: Option<String>,
    pub string_parameter: Option<String>,
    pub float_parameter: Option<String>,
    pub mix: Option<String>,
    pub channel: Option<String>,
    /// Дополнительные параметры (`Condition` держит в них сравниваемые выражения).
    pub additional_parameters: Vec<String>,
    pub collapsed: bool,
    pub executable: bool,
    pub use_in_active_state: bool,
    /// Путь состояния (устаревшая форма) и XPath по состоянию vMix.
    pub active_state_path: String,
    pub active_state_xpath: String,
    pub active_state_value: String,
    /// У части функций путь зависит от параметра.
    pub active_state_xpath_int_dependence: Vec<String>,
}

impl WidgetCommand {
    /// Прочитать команду из узла `.vmc` (старый или новый тип кнопки).
    pub fn from_node(node: &Node) -> Self {
        let action = node.child("Action");
        let text = |name: &str| node.child_text(name).map(str::to_string);
        let action_text = |name: &str| {
            action
                .and_then(|node| node.child_text(name))
                .map(str::to_string)
        };
        let flag = |name: &str, fallback: bool| match node.child_text(name) {
            Some("true") | Some("True") | Some("1") => true,
            Some("false") | Some("False") | Some("0") => false,
            _ => fallback,
        };
        let optional = |value: Option<String>| value.filter(|text| !text.is_empty());
        Self {
            function: action_text("Function").unwrap_or_default(),
            description: action_text("Description").unwrap_or_default(),
            format_string: action_text("FormatString").unwrap_or_default(),
            native: matches!(action_text("Native").as_deref(), Some("true") | Some("True")),
            // в .vmc вход хранится числом, 0 означает «не задан»
            input: optional(text("Input").or_else(|| text("InputNumber")))
                .and_then(|value| value.parse::<i64>().ok())
                .filter(|value| *value != 0),
            input_key: optional(text("InputKey")),
            parameter: optional(text("Parameter").or_else(|| text("SelectedIndex"))),
            string_parameter: optional(text("StringParameter").or_else(|| text("Value"))),
            float_parameter: optional(text("FloatParameter")),
            mix: optional(text("Mix")),
            channel: optional(text("Channel")),
            additional_parameters: node
                .child("AdditionalParameters")
                .map(|parameters| {
                    parameters
                        .children_named("OneOfString")
                        .map(|item| item.child_text("A").unwrap_or_default().to_string())
                        .collect()
                })
                .unwrap_or_default(),
            collapsed: flag("Collapsed", false),
            executable: flag("IsExecutable", true),
            use_in_active_state: flag("UseInActiveState", true),
            active_state_path: action_text("ActiveStatePath").unwrap_or_default(),
            active_state_xpath: action_text("ActiveStateXPath").unwrap_or_default(),
            active_state_value: action_text("ActiveStateValue").unwrap_or_default(),
            active_state_xpath_int_dependence: action
                .and_then(|action| action.child("ActiveStateXPathIntDependence"))
                .map(|dependence| {
                    dependence
                        .children_named("string")
                        .map(|item| item.text.clone())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Записать команду в узел `.vmc` вместе с сигнатурой функции (`Action`),
    /// чтобы оригинальное приложение могло её исполнить без нашего каталога.
    pub fn write_into(&self, kind: &WidgetKind) -> Node {
        let new_style = matches!(kind, WidgetKind::NewButton);
        let mut node = Node::new(if new_style {
            "vMixControlNewButtonCommand"
        } else {
            "vMixControlButtonCommand"
        });

        let mut action = Node::new("Action");
        action.set_child_text("Timeout", "5000");
        if !self.description.is_empty() {
            action.set_child_text("Description", &self.description);
        }
        action.set_child_text("Function", &self.function);
        action.set_child_text("Native", if self.native { "true" } else { "false" });
        action.set_child_text("FormatString", &self.format_string);
        // состояние по данным vMix — по нему кнопка подсвечивается
        if !self.active_state_path.is_empty() {
            action.set_child_text("ActiveStatePath", &self.active_state_path);
        }
        if !self.active_state_xpath.is_empty() {
            action.set_child_text("ActiveStateXPath", &self.active_state_xpath);
        }
        if !self.active_state_value.is_empty() {
            action.set_child_text("ActiveStateValue", &self.active_state_value);
        }
        if !self.active_state_xpath_int_dependence.is_empty() {
            let mut dependence = Node::new("ActiveStateXPathIntDependence");
            for template in &self.active_state_xpath_int_dependence {
                let mut item = Node::new("string");
                item.text = template.clone();
                dependence.children.push(item);
            }
            action.children.push(dependence);
        }
        node.children.push(action);

        if new_style {
            node.set_child_text("Value", self.string_parameter.as_deref().unwrap_or(""));
            node.set_child_text("SelectedIndex", self.parameter.as_deref().unwrap_or(""));
            node.set_child_text("InputNumber", &self.input.unwrap_or(0).to_string());
            node.set_child_text("InputKey", self.input_key.as_deref().unwrap_or(""));
            node.set_child_text("Duration", "");
            node.set_child_text("Mix", self.mix.as_deref().unwrap_or(""));
            node.set_child_text("Channel", self.channel.as_deref().unwrap_or(""));
        } else {
            node.set_child_text("Parameter", self.parameter.as_deref().unwrap_or(""));
            node.set_child_text("FloatParameter", self.float_parameter.as_deref().unwrap_or(""));
            node.set_child_text("Input", &self.input.unwrap_or(0).to_string());
            node.set_child_text("InputKey", self.input_key.as_deref().unwrap_or(""));
            node.set_child_text("StringParameter", self.string_parameter.as_deref().unwrap_or(""));
            let mut additional = Node::new("AdditionalParameters");
            for value in &self.additional_parameters {
                let mut item = Node::new("OneOfString");
                item.set_child_text("A", value);
                additional.children.push(item);
            }
            node.children.push(additional);
            node.set_child_text("NoInputAssigned", if self.input.is_some() { "false" } else { "true" });
        }
        node.set_child_text("Collapsed", if self.collapsed { "true" } else { "false" });
        node.set_child_text(
            "UseInActiveState",
            if self.use_in_active_state { "true" } else { "false" },
        );
        node.set_child_text("IsExecutable", if self.executable { "true" } else { "false" });
        node
    }
}

/// Частичное обновление виджета (из интерфейса приходит только изменённое).
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetPatch {
    pub name: Option<String>,
    pub left: Option<f64>,
    pub top: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub page: Option<i64>,
    pub z_index: Option<i64>,
    pub locked: Option<bool>,
    pub caption_visible: Option<bool>,
    pub caption_on: Option<bool>,
    pub text: Option<String>,
    pub color: Option<Rgba>,
    pub border_color: Option<Rgba>,
}

// --------------------------------------------------------------- операции

impl Vmc {
    pub fn widget_array(&self) -> Option<&Node> {
        self.root.path(&CONTROLS_PATH)
    }

    pub fn widget_array_mut(&mut self) -> Result<&mut Node> {
        self.root
            .path_mut(&CONTROLS_PATH)
            .ok_or_else(|| anyhow!("в файле нет Controls/ArrayOfVMixControl"))
    }

    pub fn widget_node(&self, index: usize) -> Option<&Node> {
        self.widget_array()?.children.get(index)
    }

    /// Типизированный снимок для интерфейса.
    pub fn widgets_data(&self) -> Vec<WidgetData> {
        self.widget_array()
            .map(|array| {
                array
                    .children
                    .iter()
                    .enumerate()
                    .map(|(index, node)| WidgetData::from_node(index, node))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Отсортировать виджеты по ZIndex, как это делает оригинал при отрисовке.
    pub fn z_sorted(&self) -> Vec<WidgetData> {
        let mut widgets = self.widgets_data();
        widgets.sort_by_key(|w| (w.z_index, w.index));
        widgets
    }

    /// Создать виджет и вернуть его индекс.
    pub fn create_widget(&mut self, kind: &WidgetKind, left: f64, top: f64) -> Result<usize> {
        let existing = self.widget_array().map(|a| a.children.len()).unwrap_or(0);
        let name = format!("{}{}", kind.default_name(), existing + 1);
        let node = build_widget(kind, &name, left, top, existing as i64);
        let array = self.widget_array_mut()?;
        array.children.push(node);
        Ok(array.children.len() - 1)
    }

    /// Цели ссылки: `(индекс виджета, индекс горячей клавиши)` среди **активных**.
    /// Порт `ProcessHotkey`: ссылку слушают все виджеты с таким `Link`.
    pub fn hotkey_targets(&self, link: &str) -> Vec<(usize, usize)> {
        let link = link.trim();
        if link.is_empty() {
            return Vec::new();
        }
        self.widgets_data()
            .into_iter()
            .flat_map(|widget| {
                widget
                    .hotkeys
                    .iter()
                    .enumerate()
                    .filter(|(_, hotkey)| hotkey.active && hotkey.link == link)
                    .map(|(position, _)| (widget.index, position))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Записать расписание часов (`Events`).
    pub fn set_widget_events(&mut self, index: usize, events: &[ScheduledEvent]) -> Result<()> {
        let node = self
            .root
            .path_mut(&CONTROLS_PATH)
            .and_then(|array| array.children.get_mut(index))
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;
        let mut collection = Node::new("Events");
        for event in events {
            let mut item = Node::new("ScheduledEvent");
            item.set_child_text("TimeOfDay", &format!("{}:00", event.time));
            item.set_child_text("Command", &event.command);
            item.set_child_text("Days", &event.days.to_string());
            collection.children.push(item);
        }
        match node.child_mut("Events") {
            Some(existing) => *existing = collection,
            None => node.children.push(collection),
        }
        Ok(())
    }

    /// Все запланированные события документа: `(индекс виджета, позиция, событие)`.
    pub fn scheduled_events(&self) -> Vec<(usize, usize, ScheduledEvent)> {
        self.widgets_data()
            .into_iter()
            .flat_map(|widget| {
                widget
                    .events
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(position, event)| (widget.index, position, event))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Записать кнопки Stream Deck (обучение: «нажали на устройстве — привязали ссылку»).
    pub fn set_widget_deck_keys(&mut self, index: usize, keys: &[DeckKey]) -> Result<()> {
        let node = self
            .root
            .path_mut(&CONTROLS_PATH)
            .and_then(|array| array.children.get_mut(index))
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;
        let mut collection = Node::new("Keys");
        for key in keys {
            let mut item = Node::new("StreamDeckKey");
            item.set_child_text("A", &key.context);
            item.set_child_text("B", &key.link);
            item.set_child_text("C", &key.index.to_string());
            item.set_child_text("D", &key.extra.to_string());
            collection.children.push(item);
        }
        match node.child_mut("Keys") {
            Some(existing) => *existing = collection,
            None => node.children.push(collection),
        }
        Ok(())
    }

    /// Записать MIDI-отображения виджета (для обучения и редактора).
    pub fn set_widget_midi_mappings(&mut self, index: usize, mappings: &[MidiMapEntry]) -> Result<()> {
        let node = self
            .root
            .path_mut(&CONTROLS_PATH)
            .and_then(|array| array.children.get_mut(index))
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;
        let mut midis = Node::new("Midis");
        for mapping in mappings {
            let mut item = Node::new("MidiInterfaceKey");
            item.set_child_text("A", &mapping.channel.to_string());
            item.set_child_text("B", &mapping.number.to_string());
            item.set_child_text("C", &mapping.link);
            item.set_child_text("D", &mapping.kind);
            midis.children.push(item);
        }
        match node.child_mut("Midis") {
            Some(existing) => *existing = midis,
            None => node.children.push(midis),
        }
        Ok(())
    }

    /// Записать ссылку горячей клавиши виджета (для MIDI/Stream Deck learn).
    pub fn set_widget_hotkey_link(
        &mut self,
        index: usize,
        position: usize,
        link: &str,
    ) -> Result<()> {
        self.set_widget_hotkey(index, position, link, true)
    }

    /// Записать ссылку и активность горячей клавиши.
    pub fn set_widget_hotkey(
        &mut self,
        index: usize,
        position: usize,
        link: &str,
        active: bool,
    ) -> Result<()> {
        let node = self
            .root
            .path_mut(&CONTROLS_PATH)
            .and_then(|array| array.children.get_mut(index))
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;
        let hotkeys = node
            .child_mut("Hotkey")
            .ok_or_else(|| anyhow!("у виджета {index} нет горячих клавиш"))?;
        let hotkey = hotkeys
            .children
            .iter_mut()
            .filter(|child| child.name == "Hotkey")
            .nth(position)
            .ok_or_else(|| anyhow!("у виджета {index} нет горячей клавиши {position}"))?;
        hotkey.set_child_text("Link", link);
        hotkey.set_child_text("Active", if active { "true" } else { "false" });
        Ok(())
    }

    /// Включённые ретрансляторы (`vMixControlMultiState`): шлют все функции во второй vMix.
    pub fn relay_targets(&self) -> Vec<RelayTarget> {
        self.widgets_data()
            .into_iter()
            .filter(|widget| widget.kind == WidgetKind::MultiState)
            .filter(|widget| {
                matches!(
                    widget.extras.get("Enabled").map(String::as_str),
                    Some("true") | Some("True") | Some("1")
                )
            })
            .map(|widget| RelayTarget {
                ip: widget
                    .extras
                    .get("IP")
                    .cloned()
                    .unwrap_or_else(|| "127.0.0.1".to_string()),
                port: widget
                    .extras
                    .get("Port")
                    .and_then(|value| value.trim().parse().ok())
                    .unwrap_or(8088),
                login: widget.extras.get("Login").cloned().unwrap_or_default(),
                password: widget.extras.get("Password").cloned().unwrap_or_default(),
            })
            .collect()
    }

    /// Импортировать виджеты другого `.vmc` в контейнер — порт `AfterPropertiesChanged`
    /// у `vMixControlContainer`: координаты сдвигаются к `(0,0)`, ширина подгоняется
    /// под содержимое, виджеты вкладываются в `<Controls>`.
    pub fn import_into_container(&mut self, index: usize, source: &[u8]) -> Result<usize> {
        let other = Vmc::parse(source)?;
        let nodes: Vec<Node> = other.widget_nodes().into_iter().cloned().collect();
        if nodes.is_empty() {
            return Err(anyhow!("в импортируемом файле нет виджетов"));
        }

        let number = |node: &Node, tag: &str| -> f64 {
            node.child_text(tag)
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0.0)
        };
        let min_x = nodes.iter().map(|node| number(node, "Left")).fold(f64::MAX, f64::min);
        let min_y = nodes.iter().map(|node| number(node, "Top")).fold(f64::MAX, f64::min);
        let max_right = nodes
            .iter()
            .map(|node| number(node, "Left") + number(node, "Width"))
            .fold(f64::MIN, f64::max);

        let mut controls = Node::new("Controls");
        for mut node in nodes {
            node.set_child_text("Left", &(number(&node, "Left") - min_x).to_string());
            node.set_child_text("Top", &(number(&node, "Top") - min_y).to_string());
            node.set_child_text("Locked", "false");
            controls.children.push(node);
        }

        let container = self
            .root
            .path_mut(&CONTROLS_PATH)
            .and_then(|array| array.children.get_mut(index))
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;
        container.set_child_text("Width", &(max_right - min_x + 8.0).to_string());
        let count = controls.children.len();
        match container.child_mut("Controls") {
            Some(existing) => *existing = controls,
            None => container.children.push(controls),
        }
        Ok(count)
    }

    /// Записать дополнительное свойство виджета (`InputKey`, `Target`, `Variable`, …).
    pub fn set_widget_extra(&mut self, index: usize, tag: &str, value: &str) -> Result<()> {
        let node = self
            .root
            .path_mut(&CONTROLS_PATH)
            .and_then(|array| array.children.get_mut(index))
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;
        node.set_child_text(tag, value);
        Ok(())
    }

    pub fn remove_widget(&mut self, index: usize) -> Result<()> {
        let array = self.widget_array_mut()?;
        if index >= array.children.len() {
            return Err(anyhow!("виджета с индексом {index} нет"));
        }
        array.children.remove(index);
        Ok(())
    }

    /// Дублировать виджет со сдвигом, как «Duplicate» в оригинале.
    pub fn duplicate_widget(&mut self, index: usize) -> Result<usize> {
        let original = self
            .widget_node(index)
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?
            .clone();
        let mut copy = original;
        let offset = 16.0;
        let shift = |node: &mut Node, name: &str, delta: f64| {
            let value = node
                .child_text(name)
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0);
            node.set_child_text(name, &format!("{}", value + delta));
        };
        shift(&mut copy, "Left", offset);
        shift(&mut copy, "Top", offset);
        let z = copy
            .child_text("ZIndex")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        copy.set_child_text("ZIndex", &(z + 1).to_string());
        if let Some(name) = copy.child_text("Name").map(str::to_string) {
            copy.set_child_text("Name", &format!("{name} (копия)"));
        }
        copy.set_child_text("Selected", "false");

        let array = self.widget_array_mut()?;
        array.children.push(copy);
        Ok(array.children.len() - 1)
    }

    pub fn update_widget(&mut self, index: usize, patch: &WidgetPatch) -> Result<()> {
        let node = self
            .root
            .path_mut(&CONTROLS_PATH)
            .and_then(|array| array.children.get_mut(index))
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;

        let mut put_number = |name: &str, value: Option<f64>| {
            if let Some(value) = value {
                node.set_child_text(name, &format!("{value}"));
            }
        };
        put_number("Left", patch.left);
        put_number("Top", patch.top);
        put_number("Width", patch.width);
        put_number("Height", patch.height);

        if let Some(page) = patch.page {
            node.set_child_text("Page", &page.to_string());
        }
        if let Some(z) = patch.z_index {
            node.set_child_text("ZIndex", &z.to_string());
        }
        if let Some(name) = &patch.name {
            node.set_child_text("Name", name);
        }
        if let Some(text) = &patch.text {
            node.set_child_text("Text", text);
        }
        if let Some(flag) = patch.locked {
            node.set_child_text("Locked", if flag { "true" } else { "false" });
        }
        if let Some(flag) = patch.caption_visible {
            node.set_child_text("IsCaptionVisible", if flag { "true" } else { "false" });
        }
        if let Some(flag) = patch.caption_on {
            node.set_child_text("IsCaptionOn", if flag { "true" } else { "false" });
        }
        if let Some(color) = &patch.color {
            color.write_into("Color", node);
        }
        if let Some(color) = &patch.border_color {
            color.write_into("BorderColor", node);
        }
        Ok(())
    }

    /// Заменить команды кнопки (пустой список — очистить).
    pub fn set_widget_commands(&mut self, index: usize, commands: &[WidgetCommand]) -> Result<()> {
        let kind = self
            .widget_node(index)
            .and_then(|node| node.attr_local("type"))
            .map(WidgetKind::from_type_name)
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;
        let node = self
            .root
            .path_mut(&CONTROLS_PATH)
            .and_then(|array| array.children.get_mut(index))
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;

        let mut container = Node::new("Commands");
        for command in commands {
            container.children.push(command.write_into(&kind));
        }
        match node.child_mut("Commands") {
            Some(existing) => *existing = container,
            None => node.children.push(container),
        }
        Ok(())
    }

    /// Записать настройки внешних данных виджета.
    pub fn set_widget_external(&mut self, index: usize, external: &ExternalData) -> Result<()> {
        let node = self
            .root
            .path_mut(&CONTROLS_PATH)
            .and_then(|array| array.children.get_mut(index))
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;
        external.write_into(node);
        Ok(())
    }

    /// Записать строки списка (`<Items>`): правка списка из интерфейса.
    pub fn set_widget_items(&mut self, index: usize, items: &[String]) -> Result<()> {
        let node = self
            .root
            .path_mut(&CONTROLS_PATH)
            .and_then(|array| array.children.get_mut(index))
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;

        // <Items> может отсутствовать (например, у плейлиста) — создаём
        if node.child("Items").is_none() {
            node.children.push(crate::Node::new("Items"));
        }
        let container = node
            .children
            .iter_mut()
            .find(|child| child.name == "Items")
            .expect("Items только что создан");
        // старые строки убираем, прочие элементы <Items> не трогаем
        container.children.retain(|child| child.name != "string");
        for item in items {
            let mut entry = crate::Node::new("string");
            entry.text = item.clone();
            container.children.push(entry);
        }
        Ok(())
    }

    /// Отметить кнопку нажатой/отпущенной (для `Style` = Toggle).
    pub fn set_widget_active(&mut self, index: usize, active: bool) -> Result<()> {
        let node = self
            .root
            .path_mut(&CONTROLS_PATH)
            .and_then(|array| array.children.get_mut(index))
            .ok_or_else(|| anyhow!("виджета с индексом {index} нет"))?;
        node.set_child_text("Active", if active { "true" } else { "false" });
        node.set_child_text("IsPushed", if active { "true" } else { "false" });
        Ok(())
    }

    /// XML виджета — для отладки и проверок совместимости.
    pub fn widget_xml(&self, index: usize) -> Option<String> {
        self.widget_node(index).map(Node::to_xml_string)
    }

    /// Записать документ обратно в файл (с BOM и декларацией, как оригинал).
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        std::fs::write(path.as_ref(), self.to_bytes())
            .map_err(|e| anyhow!("запись {}: {e}", path.as_ref().display()))
    }
}

/// Собрать узел нового виджета: общий набор свойств + свойства типа.
/// Стандартные горячие клавиши типа виджета (как в `.vmc`, созданных оригиналом).
fn default_hotkeys(kind: &WidgetKind) -> &'static [&'static str] {
    match kind {
        WidgetKind::Button | WidgetKind::NewButton => {
            &["Execute", "Reset", "Clear Variables", "Press", "Release"]
        }
        WidgetKind::MultiState => &["Toggle Enabled"],
        _ => &[],
    }
}

fn build_widget(kind: &WidgetKind, name: &str, left: f64, top: f64, z: i64) -> Node {
    let (width, height) = kind.default_size();
    let mut node = Node::new("vMixControl");
    node.set_attr("xsi:type", &kind.type_name());

    let names = default_hotkeys(kind);
    if !names.is_empty() {
        let mut hotkeys = Node::new("Hotkey");
        for hotkey_name in names {
            let mut hotkey = Node::new("Hotkey");
            hotkey.set_child_text("Name", hotkey_name);
            hotkey.set_child_text("Link", "");
            hotkey.set_child_text("Key", "None");
            for flag in ["Ctrl", "Alt", "Shift", "OnPress", "Active"] {
                hotkey.set_child_text(flag, "false");
            }
            hotkeys.children.push(hotkey);
        }
        node.children.push(hotkeys);
    }

    let mut window = Node::new("WindowProperties");
    for part in ["A", "B", "C", "D"] {
        window.set_child_text(part, "0");
    }
    node.children.push(window);

    node.set_child_text("IsPasswordLockable", "false");
    node.set_child_text("IsPasswordLocked", "false");
    node.set_child_text("Locked", "false");
    node.set_child_text("IsCaptionVisible", "true");
    node.set_child_text("IsCaptionOn", "true");
    node.set_child_text("Name", name);
    kind.default_color().write_into("Color", &mut node);
    kind.default_border_color().write_into("BorderColor", &mut node);
    node.set_child_text("Top", &format!("{top}"));
    node.set_child_text("Left", &format!("{left}"));
    node.set_child_text("Width", &format!("{width}"));
    node.set_child_text("Height", &format!("{height}"));
    node.set_child_text("ZIndex", &z.to_string());
    node.set_child_text("Selected", "false");
    node.set_child_text("CaptionHeight", "0");
    node.set_child_text("Hotkey", "");
    node.set_child_text("IsTemplate", "false");
    node.set_child_text("Scale", "1");
    node.set_child_text("Page", "0");

    match kind {
        WidgetKind::Region => {
            node.set_child_text("Text", "");
        }
        WidgetKind::Button | WidgetKind::NewButton => {
            kind.default_border_color()
                .write_into("BlinkBorderColor", &mut node);
            // пустые коллекции обязаны присутствовать: иначе в оригинале это null
            node.children.push(Node::new("Commands"));
            node.set_child_text("AutoStart", "false");
            node.set_child_text("IsStateDependent", "false");
            node.set_child_text("IsColorized", "false");
            node.children.push(Node::new("Image"));
            node.set_child_text("ImageMax", "1");
            node.set_child_text("ImageNumber", "0");
            node.set_child_text("IsPushed", "false");
            node.set_child_text("Style", "Momentary");
            node.set_child_text("Text", name);
        }
        WidgetKind::TextField | WidgetKind::Label => {
            node.set_child_text("IsLive", "true");
            node.set_child_text("IsTable", "false");
            node.set_child_text("IsMappedToGUID", "false");
            node.set_child_text("Text", name);
            node.children.push(Node::new("Paths"));
            node.set_child_text("Template", "false");
        }
        WidgetKind::Score => {
            node.set_child_text("IsLive", "true");
            node.set_child_text("IsTable", "false");
            node.set_child_text("IsMappedToGUID", "false");
            node.set_child_text("Text", "0");
            node.children.push(Node::new("Paths"));
            node.set_child_text("Template", "false");
            node.set_child_text("Style", "Basic");
        }
        WidgetKind::Timer => {
            node.set_child_text("IsLive", "true");
            node.set_child_text("IsTable", "false");
            node.set_child_text("IsMappedToGUID", "false");
            node.set_child_text("Text", "00:00");
            node.children.push(Node::new("Paths"));
            node.set_child_text("Template", "false");
            node.set_child_text("Format", "mm\\:ss");
            node.children.push(Node::new("Links"));
            node.set_child_text("Reverse", "false");
            node.set_child_text("TimeTicks", "0");
            node.set_child_text("DefaultTimeTicks", "3000000000");
        }
        WidgetKind::List => {
            node.set_child_text("IsLive", "true");
            node.set_child_text("IsTable", "false");
            node.set_child_text("IsMappedToGUID", "false");
            node.set_child_text("Text", "");
            node.children.push(Node::new("Paths"));
            node.set_child_text("Template", "false");
            node.children.push(Node::new("Items"));
            let mut source = Node::new("DataSource");
            source.set_child_text("A", "");
            source.set_child_text("B", "");
            source.set_child_text("C", "false");
            node.children.push(source);
        }
        _ => {}
    }

    node
}
