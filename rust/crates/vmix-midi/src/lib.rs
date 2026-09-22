//! MIDI: сообщения, отображения «сообщение → ссылка виджета» и источники событий.
//!
//! Соответствие оригиналу (`vMixControlMidiInterface` + `MidiInterfaceKey`):
//! запись отображения — это `Quadriple<int A, int B, string C, MidiEventType D>`, то есть
//! «канал, номер (нота/контроллер), ссылка, тип события». Пришедшее сообщение ищет
//! совпадающие записи и отправляет `HotkeyLinkMessage { Link, Parameter = значение }`,
//! а ссылку уже разбирает `ProcessHotkey` — то есть MIDI ничего не знает о виджетах.
//!
//! Источник событий — трейт [`MidiSource`]:
//! * [`TestSource`] — заранее заданные сообщения (тесты и демонстрация без железа);
//! * [`runtime::RuntimeSource`] — реальный MIDI-вход через `midir`.

use serde::{Deserialize, Serialize};

/// Тип MIDI-события (подмножество `MidiEventType`, которое использует оригинал).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MidiKind {
    NoteOn,
    NoteOff,
    NoteAftertouch,
    ControlChange,
    ProgramChange,
    PitchBend,
}

impl MidiKind {
    /// Разбор значения из `.vmc` (`NoteOn`, `ControlChange`, …).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "NoteOn" | "NoteOnEvent" => Some(Self::NoteOn),
            "NoteOff" | "NoteOffEvent" => Some(Self::NoteOff),
            "NoteAftertouch" | "NoteAftertouchEvent" => Some(Self::NoteAftertouch),
            "ControlChange" | "ControlChangeEvent" => Some(Self::ControlChange),
            "ProgramChange" | "ProgramChangeEvent" => Some(Self::ProgramChange),
            "PitchBend" | "PitchBendEvent" => Some(Self::PitchBend),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NoteOn => "NoteOn",
            Self::NoteOff => "NoteOff",
            Self::NoteAftertouch => "NoteAftertouch",
            Self::ControlChange => "ControlChange",
            Self::ProgramChange => "ProgramChange",
            Self::PitchBend => "PitchBend",
        }
    }
}

/// Пришедшее MIDI-сообщение.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiMessage {
    pub kind: MidiKind,
    /// Канал 0…15 (в оригинале — как есть из библиотеки).
    pub channel: u8,
    /// Номер ноты/контроллера/программы.
    pub number: u8,
    /// Значение (velocity, значение контроллера, давление…).
    pub value: u8,
}

/// Отображение «сообщение → ссылка виджета».
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiMapping {
    pub kind: MidiKind,
    pub channel: u8,
    pub number: u8,
    /// Имя ссылки (`Hotkey.Link`), с которой связано сообщение.
    pub link: String,
}

impl MidiMapping {
    /// Совпадает ли сообщение с отображением (канал, тип и номер).
    pub fn matches(&self, message: &MidiMessage) -> bool {
        self.kind == message.kind
            && self.channel == message.channel
            && self.number == message.number
    }

    /// Подходит ли сообщение «по смыслу» — для режима обучения: берём первое событие.
    pub fn learn_from(message: &MidiMessage, link: &str) -> Self {
        Self {
            kind: message.kind,
            channel: message.channel,
            number: message.number,
            link: link.to_string(),
        }
    }
}

/// Отправить сообщение по отображениям: возвращает ссылки, которые нужно разобрать.
/// (Порт цикла `foreach (var item in Midis)` из `Device_EventReceived`.)
pub fn dispatch(mappings: &[MidiMapping], message: &MidiMessage) -> Vec<(String, u8)> {
    mappings
        .iter()
        .filter(|mapping| mapping.matches(message))
        .map(|mapping| (mapping.link.clone(), message.value))
        .collect()
}

/// Источник MIDI-событий.
pub trait MidiSource: Send {
    /// Забрать накопившиеся сообщения (не блокирует).
    fn poll(&mut self) -> Vec<MidiMessage>;

    /// Человекочитаемое описание источника для интерфейса.
    fn describe(&self) -> String;
}

/// Заранее заданные сообщения — тесты и демонстрация без MIDI-железа.
pub struct TestSource {
    queue: Vec<MidiMessage>,
    description: String,
}

impl TestSource {
    pub fn new(messages: Vec<MidiMessage>) -> Self {
        Self {
            queue: messages,
            description: "тестовый MIDI".to_string(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}

impl MidiSource for TestSource {
    fn poll(&mut self) -> Vec<MidiMessage> {
        std::mem::take(&mut self.queue)
    }

    fn describe(&self) -> String {
        self.description.clone()
    }
}

/// Разобрать сырые байты MIDI-сообщения (как это делает `midir`).
pub fn parse_bytes(bytes: &[u8]) -> Option<MidiMessage> {
    let status = *bytes.first()?;
    let channel = status & 0x0F;
    let payload = |index: usize| bytes.get(index).copied().unwrap_or_default();
    match status & 0xF0 {
        0x80 => Some(MidiMessage {
            kind: MidiKind::NoteOff,
            channel,
            number: payload(1),
            value: payload(2),
        }),
        // NoteOn с нулевой velocity — это NoteOff по стандарту MIDI
        0x90 => {
            let number = payload(1);
            let value = payload(2);
            Some(MidiMessage {
                kind: if value == 0 {
                    MidiKind::NoteOff
                } else {
                    MidiKind::NoteOn
                },
                channel,
                number,
                value,
            })
        }
        0xA0 => Some(MidiMessage {
            kind: MidiKind::NoteAftertouch,
            channel,
            number: payload(1),
            value: payload(2),
        }),
        0xB0 => Some(MidiMessage {
            kind: MidiKind::ControlChange,
            channel,
            number: payload(1),
            value: payload(2),
        }),
        0xC0 => Some(MidiMessage {
            kind: MidiKind::ProgramChange,
            channel,
            number: payload(1),
            value: 0,
        }),
        0xE0 => {
            // 14 бит: младшие 7 и старшие 7; в интерфейс отдаём старшие 7, как байт в оригинале
            let value = payload(2);
            Some(MidiMessage {
                kind: MidiKind::PitchBend,
                channel,
                number: 0,
                value,
            })
        }
        _ => None,
    }
}

/// Заглушка для платформ без MIDI (Android/iOS): честно сообщает об ограничении.
#[cfg(not(feature = "midi"))]
pub mod runtime {
    use anyhow::{anyhow, Result};


    /// Список MIDI-входов недоступен на этой платформе.
    pub fn input_ports() -> Result<Vec<String>> {
        Err(anyhow!(
            "MIDI не поддержан на этой платформе (соберите с feature = \"midi\" на десктопе)"
        ))
    }
}

#[cfg(feature = "midi")]
pub mod runtime {
    //! Реальный MIDI-вход через `midir`.

    use super::{parse_bytes, MidiMessage, MidiSource};
    use anyhow::{anyhow, Result};
    use midir::{MidiInput, MidiInputConnection};
    use std::sync::{Arc, Mutex};

    /// Список доступных MIDI-входов.
    pub fn input_ports() -> Result<Vec<String>> {
        let input = MidiInput::new("vMixRTC")
            .map_err(|error| anyhow!("MIDI недоступен: {error}"))?;
        Ok(input
            .ports()
            .iter()
            .filter_map(|port| input.port_name(port).ok())
            .collect())
    }

    /// Открытый MIDI-вход: сообщения копятся в буфере и забираются `poll`.
    pub struct RuntimeSource {
        /// Соединение живёт, пока живёт источник: закрывать его нельзя.
        #[allow(dead_code)]
        connection: MidiInputConnection<()>,
        buffer: Arc<Mutex<Vec<MidiMessage>>>,
        port_name: String,
    }

    impl RuntimeSource {
        /// Подключиться к входу по имени (или к первому, если имя пустое).
        pub fn connect(port_name: &str) -> Result<Self> {
            let mut input = MidiInput::new("vMixRTC")
                .map_err(|error| anyhow!("MIDI недоступен: {error}"))?;
            input.ignore(midir::Ignore::None);

            let ports = input.ports();
            let port = ports
                .iter()
                .find(|port| {
                    input
                        .port_name(port)
                        .map(|name| name == port_name)
                        .unwrap_or(false)
                })
                .or_else(|| ports.first())
                .ok_or_else(|| anyhow!("MIDI-входы не найдены"))?
                .clone();
            let name = input.port_name(&port).unwrap_or_else(|_| "MIDI".to_string());

            let buffer: Arc<Mutex<Vec<MidiMessage>>> = Arc::new(Mutex::new(Vec::new()));
            let sink = buffer.clone();
            let connection = input
                .connect(
                    &port,
                    "vmixutc-read",
                    move |_stamp, bytes, _| {
                        if let Some(message) = parse_bytes(bytes) {
                            if let Ok(mut queue) = sink.lock() {
                                queue.push(message);
                            }
                        }
                    },
                    (),
                )
                .map_err(|error| anyhow!("не удалось открыть MIDI-вход: {error}"))?;

            Ok(Self {
                connection,
                buffer,
                port_name: name,
            })
        }
    }

    impl MidiSource for RuntimeSource {
        fn poll(&mut self) -> Vec<MidiMessage> {
            self.buffer
                .lock()
                .map(|mut queue| std::mem::take(&mut *queue))
                .unwrap_or_default()
        }

        fn describe(&self) -> String {
            format!("MIDI: {}", self.port_name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note_on(channel: u8, note: u8, velocity: u8) -> Vec<u8> {
        vec![0x90 | channel, note, velocity]
    }

    #[test]
    fn parses_raw_midi_bytes() {
        let message = parse_bytes(&note_on(2, 36, 100)).unwrap();
        assert_eq!(message.kind, MidiKind::NoteOn);
        assert_eq!(message.channel, 2);
        assert_eq!(message.number, 36);
        assert_eq!(message.value, 100);

        // NoteOn с нулевой velocity — это NoteOff
        let off = parse_bytes(&note_on(0, 36, 0)).unwrap();
        assert_eq!(off.kind, MidiKind::NoteOff);

        let cc = parse_bytes(&[0xB0, 7, 64]).unwrap();
        assert_eq!(cc.kind, MidiKind::ControlChange);
        assert_eq!((cc.number, cc.value), (7, 64));

        let program = parse_bytes(&[0xC3, 12]).unwrap();
        assert_eq!(program.kind, MidiKind::ProgramChange);
        assert_eq!((program.channel, program.number), (3, 12));

        let bend = parse_bytes(&[0xE0, 0, 96]).unwrap();
        assert_eq!(bend.kind, MidiKind::PitchBend);
        assert_eq!(bend.value, 96);

        assert!(parse_bytes(&[0xF8]).is_none(), "служебные байты не события");
        assert!(parse_bytes(&[]).is_none());
    }

    #[test]
    fn mapping_matches_channel_number_and_kind() {
        let mapping = MidiMapping {
            kind: MidiKind::ControlChange,
            channel: 0,
            number: 7,
            link: "Button1.Execute".into(),
        };
        assert!(mapping.matches(&MidiMessage {
            kind: MidiKind::ControlChange,
            channel: 0,
            number: 7,
            value: 1,
        }));
        assert!(!mapping.matches(&MidiMessage {
            kind: MidiKind::ControlChange,
            channel: 1,
            number: 7,
            value: 1,
        }));
        assert!(!mapping.matches(&MidiMessage {
            kind: MidiKind::NoteOn,
            channel: 0,
            number: 7,
            value: 1,
        }));
    }

    #[test]
    fn dispatch_returns_links_with_value() {
        let mappings = vec![
            MidiMapping {
                kind: MidiKind::NoteOn,
                channel: 0,
                number: 36,
                link: "Play.Execute".into(),
            },
            MidiMapping {
                kind: MidiKind::NoteOn,
                channel: 0,
                number: 36,
                link: "Light.On".into(),
            },
            MidiMapping {
                kind: MidiKind::NoteOn,
                channel: 0,
                number: 40,
                link: "Другое".into(),
            },
        ];
        let message = MidiMessage {
            kind: MidiKind::NoteOn,
            channel: 0,
            number: 36,
            value: 127,
        };
        let links = dispatch(&mappings, &message);
        assert_eq!(
            links,
            vec![
                ("Play.Execute".to_string(), 127),
                ("Light.On".to_string(), 127)
            ]
        );
    }

    #[test]
    fn learn_takes_the_first_event() {
        let message = MidiMessage {
            kind: MidiKind::ControlChange,
            channel: 9,
            number: 21,
            value: 0,
        };
        let mapping = MidiMapping::learn_from(&message, "Fader.Set");
        assert_eq!(mapping.channel, 9);
        assert_eq!(mapping.number, 21);
        assert_eq!(mapping.kind, MidiKind::ControlChange);
        assert!(mapping.matches(&message));
    }

    #[test]
    fn test_source_gives_messages_once() {
        let mut source = TestSource::new(vec![MidiMessage {
            kind: MidiKind::NoteOn,
            channel: 0,
            number: 60,
            value: 100,
        }])
        .with_description("демо");
        assert_eq!(source.poll().len(), 1);
        assert!(source.poll().is_empty(), "сообщения отдаются один раз");
        assert_eq!(source.describe(), "демо");
    }

    #[cfg(feature = "midi")]
    #[test]
    fn runtime_ports_do_not_panic() {
        // на машине без MIDI-устройств список просто пустой
        match runtime::input_ports() {
            Ok(ports) => eprintln!("MIDI-входов: {}", ports.len()),
            Err(error) => eprintln!("MIDI недоступен: {error}"),
        }
    }
}
