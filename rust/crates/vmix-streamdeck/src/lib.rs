//! Stream Deck: прямая работа с устройством по HID.
//!
//! **Отличие от оригинала (осознанное).** В C# контроллер не общается с устройством напрямую:
//! он подключается к **плагину Elgato** (`vMixUTCStreamDeck.exe`) через WebSocket, а тот уже
//! работает с устройством и присылает `KeyDown`/`KeyUp` с «контекстом» кнопки. Порт общается
//! с устройством **сам, по HID** — это ближе к цели «только Rust»: не нужны ни Stream Deck
//! Software, ни .NET-плагин. Поэтому поле `Context` из `.vmc` сохраняется для совместимости,
//! но в работе не участвует: роль контекста играет индекс кнопки.
//!
//! Что делает крейт: определяет модель по `product_id`, разбирает входные отчёты в события
//! (с фронтами «нажал/отпустил»), умеет задавать яркость. Источник событий — трейт
//! [`DeckSource`]: [`TestSource`] для тестов и [`runtime::RuntimeSource`] для реального HID.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Идентификатор производителя Elgato.
pub const ELGATO_VENDOR_ID: u16 = 0x0fd9;

/// Модель устройства и её параметры (определяется по `product_id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckModel {
    pub product_id: u16,
    pub name: &'static str,
    pub keys: usize,
    /// Длина входного отчёта вместе с идентификатором.
    pub report_len: usize,
    /// Есть ли поворотные ручки (Stream Deck +).
    pub dials: usize,
}

/// Известные модели. Значения — из публичной документации HID-протокола Stream Deck.
pub const MODELS: &[DeckModel] = &[
    DeckModel {
        product_id: 0x0060,
        name: "Stream Deck (15)",
        keys: 15,
        report_len: 17,
        dials: 0,
    },
    DeckModel {
        product_id: 0x0063,
        name: "Stream Deck Mini (6)",
        keys: 6,
        report_len: 17,
        dials: 0,
    },
    DeckModel {
        product_id: 0x006d,
        name: "Stream Deck MK.2 (15)",
        keys: 15,
        report_len: 17,
        dials: 0,
    },
    DeckModel {
        product_id: 0x0061,
        name: "Stream Deck XL (32)",
        keys: 32,
        report_len: 1024,
        dials: 0,
    },
    DeckModel {
        product_id: 0x0080,
        name: "Stream Deck MK.2 XL (32)",
        keys: 32,
        report_len: 1024,
        dials: 0,
    },
    DeckModel {
        product_id: 0x0084,
        name: "Stream Deck + (8)",
        keys: 8,
        report_len: 1024,
        dials: 4,
    },
];

/// Модель по `product_id`.
pub fn model_for(product_id: u16) -> Option<DeckModel> {
    MODELS
        .iter()
        .copied()
        .find(|model| model.product_id == product_id)
}

/// Что произошло на устройстве.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeckEventKind {
    KeyDown,
    KeyUp,
    DialDown,
    DialUp,
}

/// Событие устройства: кнопка (или ручка) с номером.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckEvent {
    pub kind: DeckEventKind,
    pub index: usize,
}

/// Состояние устройства: превращает сырые отчёты в события-фронты.
pub struct DeckState {
    model: DeckModel,
    keys: Vec<bool>,
    dials: Vec<bool>,
}

impl DeckState {
    pub fn new(model: DeckModel) -> Self {
        Self {
            keys: vec![false; model.keys],
            dials: vec![false; model.dials],
            model,
        }
    }

    pub fn model(&self) -> DeckModel {
        self.model
    }

    /// Разобрать очередной входной отчёт.
    ///
    /// Раскладка (публичный HID-протокол): `report[0]` — идентификатор отчёта `0x01`,
    /// далее по байту на кнопку: `0x01` — нажата, `0x00` — отпущена. У Stream Deck +
    /// после кнопок идут состояния ручек, у XL отчёт длиной 1024 байта.
    pub fn apply(&mut self, report: &[u8]) -> Vec<DeckEvent> {
        if report.is_empty() {
            return Vec::new();
        }
        let mut events = Vec::new();

        for index in 0..self.model.keys {
            let pressed = report.get(1 + index).copied().unwrap_or(0) != 0;
            if pressed != self.keys[index] {
                self.keys[index] = pressed;
                events.push(DeckEvent {
                    kind: if pressed {
                        DeckEventKind::KeyDown
                    } else {
                        DeckEventKind::KeyUp
                    },
                    index,
                });
            }
        }

        // у Stream Deck + состояния ручек идут сразу после кнопок
        for index in 0..self.model.dials {
            let pressed = report.get(1 + self.model.keys + index).copied().unwrap_or(0) != 0;
            if pressed != self.dials[index] {
                self.dials[index] = pressed;
                events.push(DeckEvent {
                    kind: if pressed {
                        DeckEventKind::DialDown
                    } else {
                        DeckEventKind::DialUp
                    },
                    index,
                });
            }
        }

        events
    }

    /// Отчёт установки яркости (0…100).
    pub fn brightness_report(&self, percent: u8) -> Vec<u8> {
        let mut report = vec![0u8; self.model.report_len];
        report[0] = 0x03; // SET_BRIGHTNESS
        report[1] = 0x01; // команда «установить»
        report[2] = percent.min(100);
        report
    }
}

/// Источник событий Stream Deck.
pub trait DeckSource: Send {
    /// Забрать накопившиеся события (не блокирует).
    fn poll(&mut self) -> Vec<DeckEvent>;

    /// Описание устройства для интерфейса.
    fn describe(&self) -> String;

    /// Яркость подсветки, 0…100.
    fn set_brightness(&mut self, _percent: u8) -> Result<()> {
        Ok(())
    }
}

/// Заранее заданные события — тесты и демонстрация без устройства.
pub struct TestSource {
    events: Vec<DeckEvent>,
    description: String,
    brightness: u8,
}

impl TestSource {
    pub fn new(events: Vec<DeckEvent>) -> Self {
        Self {
            events,
            description: "тестовый Stream Deck".to_string(),
            brightness: 100,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}

impl DeckSource for TestSource {
    fn poll(&mut self) -> Vec<DeckEvent> {
        std::mem::take(&mut self.events)
    }

    fn describe(&self) -> String {
        self.description.clone()
    }

    fn set_brightness(&mut self, percent: u8) -> Result<()> {
        self.brightness = percent.min(100);
        Ok(())
    }
}

/// Заглушка для платформ без HID (Android/iOS): честно сообщает об ограничении.
#[cfg(not(feature = "hid"))]
pub mod runtime {
    use anyhow::{anyhow, Result};
    use super::DeckModel;

    /// Устройства недоступны на этой платформе.
    pub fn devices() -> Result<Vec<(String, DeckModel)>> {
        Err(anyhow!(
            "Stream Deck по HID не поддержан на этой платформе (соберите с feature = \"hid\")"
        ))
    }
}

#[cfg(feature = "hid")]
pub mod runtime {
    //! Реальное устройство через `hidapi`.

    use super::{model_for, DeckEvent, DeckModel, DeckSource, DeckState, ELGATO_VENDOR_ID};
    use anyhow::{anyhow, Result};
    use hidapi::{HidApi, HidDevice};
    use std::time::Duration;

    /// Найденные Stream Deck: `(путь, модель)`.
    pub fn devices() -> Result<Vec<(String, DeckModel)>> {
        let api = HidApi::new().map_err(|error| anyhow!("HID недоступен: {error}"))?;
        Ok(api
            .device_list()
            .filter(|info| info.vendor_id() == ELGATO_VENDOR_ID)
            .filter_map(|info| {
                model_for(info.product_id()).map(|model| {
                    let serial = info.serial_number().unwrap_or("без номера").to_string();
                    (format!("{} · {serial}", model.name), model)
                })
            })
            .collect())
    }

    /// Открытое устройство: читаем отчёты и превращаем их в события.
    pub struct RuntimeSource {
        device: HidDevice,
        state: DeckState,
        buffer_len: usize,
        description: String,
    }

    impl RuntimeSource {
        /// Открыть первое найденное устройство (или то, чьё описание совпало).
        pub fn open(wanted: &str) -> Result<Self> {
            let api = HidApi::new().map_err(|error| anyhow!("HID недоступен: {error}"))?;
            let info = api
                .device_list()
                .filter(|info| info.vendor_id() == ELGATO_VENDOR_ID)
                .find(|info| {
                    if wanted.trim().is_empty() {
                        return true;
                    }
                    model_for(info.product_id())
                        .map(|model| {
                            wanted.contains(model.name)
                                || info
                                    .serial_number()
                                    .map(|serial| wanted.contains(serial))
                                    .unwrap_or(false)
                        })
                        .unwrap_or(false)
                })
                .ok_or_else(|| anyhow!("Stream Deck не найден (искали: {wanted})"))?;

            let product_id = info.product_id();
            let model = model_for(product_id)
                .ok_or_else(|| anyhow!("неизвестная модель Stream Deck: {product_id:#06x}"))?;
            let description = format!(
                "{} · {}",
                model.name,
                info.serial_number().unwrap_or("без номера")
            );
            let device = info
                .open_device(&api)
                .map_err(|error| anyhow!("не удалось открыть устройство: {error}"))?;

            Ok(Self {
                device,
                state: DeckState::new(model),
                buffer_len: model.report_len.max(17),
                description,
            })
        }
    }

    impl DeckSource for RuntimeSource {
        fn poll(&mut self) -> Vec<DeckEvent> {
            let mut events = Vec::new();
            let mut buffer = vec![0u8; self.buffer_len];
            // читаем всё, что успело накопиться, но не блокируемся надолго
            for _ in 0..16 {
                match self.device.read_timeout(&mut buffer, 5) {
                    Ok(read) if read > 0 => events.extend(self.state.apply(&buffer[..read])),
                    _ => break,
                }
            }
            events
        }

        fn describe(&self) -> String {
            self.description.clone()
        }

        fn set_brightness(&mut self, percent: u8) -> Result<()> {
            let report = self.state.brightness_report(percent);
            self.device
                .write(&report)
                .map_err(|error| anyhow!("яркость: {error}"))?;
            let _ = Duration::from_millis(0);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classic() -> DeckModel {
        model_for(0x0060).unwrap()
    }

    /// Отчёт классического Stream Deck: `0x01` + состояния кнопок.
    fn report(model: DeckModel, pressed: &[usize]) -> Vec<u8> {
        let mut report = vec![0u8; model.report_len];
        report[0] = 0x01;
        for index in pressed {
            report[1 + index] = 1;
        }
        report
    }

    #[test]
    fn identifies_models_by_product_id() {
        assert_eq!(classic().keys, 15);
        assert_eq!(model_for(0x0063).unwrap().keys, 6);
        assert_eq!(model_for(0x0061).unwrap().keys, 32);
        assert_eq!(model_for(0x0061).unwrap().report_len, 1024);
        let plus = model_for(0x0084).unwrap();
        assert_eq!((plus.keys, plus.dials), (8, 4));
        assert!(model_for(0x1234).is_none(), "чужие устройства игнорируем");
    }

    #[test]
    fn reports_become_press_and_release_edges() {
        let model = classic();
        let mut state = DeckState::new(model);

        // нажали кнопку 3
        let events = state.apply(&report(model, &[3]));
        assert_eq!(
            events,
            vec![DeckEvent {
                kind: DeckEventKind::KeyDown,
                index: 3
            }]
        );

        // тот же отчёт повторно — событий нет (важно: устройство шлёт отчёты постоянно)
        assert!(state.apply(&report(model, &[3])).is_empty());

        // нажали ещё и 7, отпустили 3
        let events = state.apply(&report(model, &[7]));
        assert_eq!(
            events,
            vec![
                DeckEvent {
                    kind: DeckEventKind::KeyUp,
                    index: 3
                },
                DeckEvent {
                    kind: DeckEventKind::KeyDown,
                    index: 7
                },
            ]
        );

        // всё отпустили
        let events = state.apply(&report(model, &[]));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, DeckEventKind::KeyUp);
        assert_eq!(events[0].index, 7);
    }

    #[test]
    fn xl_uses_long_reports() {
        let model = model_for(0x0061).unwrap();
        let mut state = DeckState::new(model);
        let events = state.apply(&report(model, &[31]));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].index, 31);
        assert_eq!(state.brightness_report(50).len(), 1024);
    }

    #[test]
    fn plus_dials_are_reported_separately() {
        let model = model_for(0x0084).unwrap();
        let mut state = DeckState::new(model);
        let mut report = vec![0u8; model.report_len];
        report[0] = 0x01;
        report[1 + 2] = 1; // кнопка 2
        report[1 + model.keys] = 1; // первая ручка
        let events = state.apply(&report);
        assert_eq!(
            events,
            vec![
                DeckEvent {
                    kind: DeckEventKind::KeyDown,
                    index: 2
                },
                DeckEvent {
                    kind: DeckEventKind::DialDown,
                    index: 0
                },
            ]
        );
    }

    #[test]
    fn brightness_report_is_well_formed() {
        let state = DeckState::new(classic());
        let report = state.brightness_report(150);
        assert_eq!(report.len(), 17);
        assert_eq!(&report[..3], &[0x03, 0x01, 100], "яркость ограничивается сотней");
    }

    #[test]
    fn test_source_gives_events_once() {
        let mut source = TestSource::new(vec![DeckEvent {
            kind: DeckEventKind::KeyUp,
            index: 5,
        }])
        .with_description("демо");
        assert_eq!(source.poll().len(), 1);
        assert!(source.poll().is_empty());
        assert_eq!(source.describe(), "демо");
        assert!(source.set_brightness(40).is_ok());
    }

    #[cfg(feature = "hid")]
    #[test]
    fn runtime_devices_do_not_panic() {
        match runtime::devices() {
            Ok(devices) => eprintln!("найдено Stream Deck: {}", devices.len()),
            Err(error) => eprintln!("HID недоступен: {error}"),
        }
    }
}
