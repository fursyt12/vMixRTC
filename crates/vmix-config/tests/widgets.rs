//! Проверки правки `.vmc`: совместимость с оригиналом, сохранение, дублирование.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use vmix_config::{Rgba, Vmc, WidgetCommand, WidgetKind, WidgetPatch};

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn load(name: &str) -> Vmc {
    let path = examples_dir().join(name);
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Vmc::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Имена верхнеуровневых свойств всех виджетов данного типа в файле.
fn property_sets(file: &str) -> Vec<(String, BTreeSet<String>)> {
    let path = examples_dir().join(file);
    let root = vmix_xml::parse(&fs::read(&path).unwrap()).unwrap();
    let array = root
        .path(&["Controls", "ArrayOfVMixControl"])
        .expect("ArrayOfVMixControl");
    let mut seen: Vec<(String, BTreeSet<String>)> = Vec::new();
    for node in &array.children {
        let Some(kind) = node.attr_local("type") else {
            continue;
        };
        let names: BTreeSet<String> = node.children.iter().map(|c| c.name.clone()).collect();
        match seen.iter_mut().find(|(k, _)| k == kind) {
            Some((_, set)) => set.extend(names),
            None => seen.push((kind.to_string(), names)),
        }
    }
    seen
}

#[test]
fn created_widgets_contain_every_base_property() {
    // базовый набор = свойства, которые есть у виджетов всех типов во всех примерах
    let mut base: Option<BTreeSet<String>> = None;
    for file in ["Buttons.vmc", "Scoreboard.vmc", "InputSelector.vmc", "ProxySample.vmc", "TimerEvents.vmc", "Buttons2.vmc"] {
        for (_, props) in property_sets(file) {
            base = Some(match base {
                None => props,
                Some(current) => current.intersection(&props).cloned().collect(),
            });
        }
    }
    let base = base.expect("не удалось собрать базовый набор свойств");
    assert!(base.contains("Name") && base.contains("Left") && base.contains("ZIndex"));

    for kind in WidgetKind::PALETTE {
        let mut vmc = load("Scoreboard.vmc");
        let index = vmc.create_widget(&kind, 10.0, 20.0).unwrap();
        let node = vmc.widget_node(index).unwrap();
        let ours: BTreeSet<String> = node.children.iter().map(|c| c.name.clone()).collect();
        let missing: Vec<&String> = base.difference(&ours).collect();
        assert!(
            missing.is_empty(),
            "{}: не хватает базовых свойств {:?}",
            kind.type_name(),
            missing
        );
        assert_eq!(node.attr("xsi:type"), Some(kind.type_name().as_str()));
    }
}

#[test]
fn created_widget_matches_the_shape_of_real_ones() {
    for (file, kind) in [
        ("Buttons.vmc", WidgetKind::Region),
        ("Buttons.vmc", WidgetKind::Button),
        ("ProxySample.vmc", WidgetKind::TextField),
        ("Scoreboard.vmc", WidgetKind::Score),
        ("Scoreboard.vmc", WidgetKind::Timer),
    ] {
        let real = property_sets(file)
            .into_iter()
            .find(|(name, _)| name == &kind.type_name())
            .unwrap_or_else(|| panic!("в {file} нет {}", kind.type_name()))
            .1;
        let mut vmc = load(file);
        let index = vmc.create_widget(&kind, 0.0, 0.0).unwrap();
        let node = vmc.widget_node(index).unwrap();
        let ours: BTreeSet<String> = node.children.iter().map(|c| c.name.clone()).collect();
        let missing: Vec<&String> = real.difference(&ours).collect();
        assert!(
            missing.is_empty(),
            "{}: относительно реальных виджетов нет свойств {:?}",
            kind.type_name(),
            missing
        );
    }
}

#[test]
fn patch_survives_save_and_reload() {
    let mut vmc = load("Scoreboard.vmc");
    let before = vmc.widgets_data();
    assert!(before.len() >= 5);

    let patch = WidgetPatch {
        name: Some("Новый заголовок".into()),
        left: Some(42.5),
        top: Some(-17.0),
        width: Some(333.0),
        height: Some(77.0),
        page: Some(2),
        z_index: Some(9),
        caption_visible: Some(false),
        text: Some("Привет".into()),
        color: Some(Rgba::new(10, 20, 30)),
        border_color: Some(Rgba::new(40, 50, 60)),
        ..Default::default()
    };
    vmc.update_widget(0, &patch).unwrap();

    let path = std::env::temp_dir().join(format!("vmix-config-patch-{}.vmc", std::process::id()));
    vmc.save(&path).unwrap();
    let reloaded = Vmc::parse(&fs::read(&path).unwrap()).unwrap();
    fs::remove_file(&path).unwrap();

    let after = reloaded.widgets_data();
    assert_eq!(after.len(), before.len(), "число виджетов изменилось");
    assert_eq!(after[0].name, "Новый заголовок");
    assert_eq!(after[0].left, 42.5);
    assert_eq!(after[0].top, -17.0);
    assert_eq!(after[0].width, 333.0);
    assert_eq!(after[0].height, 77.0);
    assert_eq!(after[0].page, 2);
    assert_eq!(after[0].z_index, 9);
    assert!(!after[0].caption_visible);
    assert_eq!(after[0].text, "Привет");
    assert_eq!(after[0].color, Rgba::new(10, 20, 30));
    assert_eq!(after[0].border_color, Rgba::new(40, 50, 60));

    // соседние виджеты и глобальные переменные не пострадали
    assert_eq!(after[1], before[1]);
    assert_eq!(reloaded.global_variables(), vmc.global_variables());
}

#[test]
fn create_duplicate_remove() {
    let mut vmc = load("ProxySample.vmc");
    let start = vmc.widget_nodes().len();

    let first = vmc.create_widget(&WidgetKind::Button, 100.0, 100.0).unwrap();
    let second = vmc.create_widget(&WidgetKind::TextField, 300.0, 100.0).unwrap();
    assert_eq!(vmc.widget_nodes().len(), start + 2);

    let copy = vmc.duplicate_widget(first).unwrap();
    assert_eq!(vmc.widget_nodes().len(), start + 3);
    let original = &vmc.widgets_data()[first];
    let duplicate = &vmc.widgets_data()[copy];
    assert_eq!(duplicate.left, original.left + 16.0);
    assert_eq!(duplicate.z_index, original.z_index + 1);
    assert!(duplicate.name.ends_with("(копия)"));

    vmc.remove_widget(second).unwrap();
    assert_eq!(vmc.widget_nodes().len(), start + 2);

    assert!(vmc.remove_widget(999).is_err());
}

#[test]
fn commands_survive_save_and_reload() {
    let mut vmc = load("Buttons.vmc");
    let index = vmc
        .widgets_data()
        .iter()
        .position(|widget| widget.kind == WidgetKind::Button)
        .expect("в Buttons.vmc нет кнопки");

    let commands = vec![
        WidgetCommand {
            function: "Cut".into(),
            description: "Cut".into(),
            format_string: "Function=Cut&Input={0}".into(),
            native: true,
            input: Some(1),
            executable: true,
            use_in_active_state: true,
            ..Default::default()
        },
        WidgetCommand {
            function: "SetText".into(),
            description: "Set title text".into(),
            format_string: "Function=SetText&Value={2}&Input={0}&SelectedIndex={1}".into(),
            input: Some(2),
            parameter: Some("0".into()),
            string_parameter: Some("Гол 1:0".into()),
            executable: true,
            use_in_active_state: true,
            ..Default::default()
        },
    ];
    vmc.set_widget_commands(index, &commands).unwrap();
    vmc.set_widget_active(index, true).unwrap();

    let path = std::env::temp_dir().join(format!("vmix-config-cmd-{}.vmc", std::process::id()));
    vmc.save(&path).unwrap();
    let reloaded = Vmc::parse(&fs::read(&path).unwrap()).unwrap();
    fs::remove_file(&path).unwrap();

    let widget = &reloaded.widgets_data()[index];
    assert_eq!(widget.commands, commands);
    assert!(widget.active);

    // структура должна совпадать с той, что пишет оригинал
    let xml = reloaded.widget_xml(index).unwrap();
    assert!(xml.contains("<Commands><vMixControlButtonCommand>"), "{xml}");
    assert!(xml.contains("<Function>Cut</Function>"), "{xml}");
    assert!(
        xml.contains("<FormatString>Function=Cut&amp;Input={0}</FormatString>"),
        "{xml}"
    );
    assert!(xml.contains("<IsExecutable>true</IsExecutable>"), "{xml}");
    assert!(xml.contains("<Input>2</Input>"), "{xml}");

    // пустой список очищает команды
    let mut vmc = reloaded;
    vmc.set_widget_commands(index, &[]).unwrap();
    assert!(vmc.widgets_data()[index].commands.is_empty());
}

#[test]
fn reads_real_commands_from_the_example_controller() {
    let vmc = load("Scoreboard.vmc");
    let button = vmc
        .widgets_data()
        .into_iter()
        .find(|widget| !widget.commands.is_empty())
        .expect("в Scoreboard.vmc у кнопок есть команды");

    let command = &button.commands[0];
    assert_eq!(command.function, "OverlayInputX");
    assert!(command.format_string.contains("OverlayInput{1}"));
    assert!(command.input_key.is_some(), "{command:?}");
    assert_eq!(command.parameter.as_deref(), Some("1"));
    assert!(command.executable);

    // команды читаются и пишутся без потерь
    let mut vmc = vmc;
    let before = button.commands.clone();
    vmc.set_widget_commands(button.index, &before).unwrap();
    assert_eq!(&vmc.widgets_data()[button.index].commands, &before);

    // и то же после сохранения файла
    let path = std::env::temp_dir().join(format!("vmix-config-real-{}.vmc", std::process::id()));
    vmc.save(&path).unwrap();
    let reloaded = Vmc::parse(&fs::read(&path).unwrap()).unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(&reloaded.widgets_data()[button.index].commands, &before);
}

#[test]
fn z_sorted_matches_paint_order() {
    let vmc = load("Scoreboard.vmc");
    let sorted = vmc.z_sorted();
    for pair in sorted.windows(2) {
        assert!(pair[0].z_index <= pair[1].z_index);
    }
    assert_eq!(sorted.len(), vmc.widgets_data().len());
}

#[test]
fn reads_widget_specific_extras() {
    let mut vmc = Vmc::empty();
    let volume = vmc.create_widget(&WidgetKind::Volume, 10.0, 10.0).unwrap();
    vmc.set_widget_extra(volume, "Target", "Bus A").unwrap();
    vmc.set_widget_extra(volume, "InputKey", "key-42").unwrap();
    let tbar = vmc.create_widget(&WidgetKind::TBar, 10.0, 320.0).unwrap();
    vmc.set_widget_extra(tbar, "Mode", "Fader").unwrap();
    let variables = vmc
        .create_widget(&WidgetKind::VariableViewer, 10.0, 400.0)
        .unwrap();
    vmc.set_widget_extra(variables, "Variable", "@Score").unwrap();

    // extras читаются из файла, а не только из памяти
    let reloaded = Vmc::parse(&vmc.to_bytes()).unwrap();
    let widgets = reloaded.widgets_data();
    assert_eq!(widgets[volume].kind, WidgetKind::Volume);
    assert_eq!(widgets[volume].extras.get("Target").map(String::as_str), Some("Bus A"));
    assert_eq!(widgets[volume].extras.get("InputKey").map(String::as_str), Some("key-42"));
    assert_eq!(widgets[tbar].extras.get("Mode").map(String::as_str), Some("Fader"));
    assert_eq!(
        widgets[variables].kind,
        WidgetKind::VariableViewer,
        "тип виджета переменных должен читаться"
    );
    assert_eq!(
        widgets[variables].extras.get("Variable").map(String::as_str),
        Some("@Score")
    );
    // чужие теги в extras не попадают
    assert!(widgets[volume].extras.get("DataProviderContent").is_none());
}

#[test]
fn global_variables_are_written_and_read() {
    let mut vmc = Vmc::empty();
    vmc.set_global_variable("Отбивка", "Готово").unwrap();
    vmc.set_global_variable("@Счёт", "3").unwrap();
    let reloaded = Vmc::parse(&vmc.to_bytes()).unwrap();
    let globals = reloaded.global_variables();
    assert!(globals.contains(&("Отбивка".to_string(), "Готово".to_string())));
    assert!(globals.contains(&("@Счёт".to_string(), "3".to_string())));
}

#[test]
fn imports_controller_into_container() {
    // источник: два виджета со сдвинутыми координатами
    let mut source = Vmc::empty();
    source.create_widget(&WidgetKind::Button, 100.0, 50.0).unwrap();
    source.create_widget(&WidgetKind::Label, 160.0, 130.0).unwrap();

    let mut target = Vmc::empty();
    let container = target.create_widget(&WidgetKind::Container, 10.0, 10.0).unwrap();
    let count = target
        .import_into_container(container, &source.to_bytes())
        .unwrap();
    assert_eq!(count, 2);

    // контейнер подтянул ширину и вложил виджеты с координатами от нуля
    let reloaded = Vmc::parse(&target.to_bytes()).unwrap();
    let widget = reloaded
        .widgets_data()
        .into_iter()
        .find(|widget| widget.index == container)
        .unwrap();
    assert_eq!(widget.kind, WidgetKind::Container);
    assert_eq!(widget.children.len(), 2);
    assert_eq!(widget.children[0].left, 0.0, "координаты сдвигаются к нулю");
    assert_eq!(widget.children[0].top, 0.0);
    assert_eq!(widget.children[1].left, 60.0);
    assert_eq!(widget.children[1].top, 80.0);
    assert!(widget.width > 200.0, "ширина под содержимое: {}", widget.width);
}

#[test]
fn relays_are_collected_from_multi_state_widgets() {
    let mut vmc = Vmc::empty();
    let relay = vmc.create_widget(&WidgetKind::MultiState, 10.0, 10.0).unwrap();
    vmc.set_widget_extra(relay, "IP", "192.168.1.50").unwrap();
    vmc.set_widget_extra(relay, "Port", "8099").unwrap();
    vmc.set_widget_extra(relay, "Login", "admin").unwrap();
    vmc.set_widget_extra(relay, "Enabled", "true").unwrap();

    let off = vmc.create_widget(&WidgetKind::MultiState, 10.0, 200.0).unwrap();
    vmc.set_widget_extra(off, "IP", "10.0.0.9").unwrap();
    vmc.set_widget_extra(off, "Enabled", "false").unwrap();

    let targets = vmc.relay_targets();
    assert_eq!(targets.len(), 1, "выключенный ретранслятор не должен попадать: {targets:?}");
    assert_eq!(targets[0].ip, "192.168.1.50");
    assert_eq!(targets[0].port, 8099);
    assert_eq!(targets[0].login, "admin");
}

#[test]
fn reads_hotkeys_and_finds_targets_by_link() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/InputSelector.vmc");
    let vmc = Vmc::parse(&std::fs::read(&path).unwrap()).unwrap();

    // у кнопки QuickPlay есть записи Hotkey (Execute/Reset/…)
    let button = vmc
        .widgets_data()
        .into_iter()
        .find(|widget| widget.name == "QuickPlay")
        .expect("кнопка QuickPlay");
    assert!(!button.hotkeys.is_empty(), "горячие клавиши должны читаться");
    assert!(button.hotkeys.iter().any(|hotkey| hotkey.name == "Execute"));

    // ссылок нет — целей тоже нет
    assert!(vmc.hotkey_targets("Play.Execute").is_empty());

    // назначаем ссылку активной и находим цель
    let position = button
        .hotkeys
        .iter()
        .position(|hotkey| hotkey.name == "Execute")
        .unwrap();
    let mut vmc = vmc;
    vmc.set_widget_hotkey_link(button.index, position, "Play.Execute")
        .unwrap();
    let targets = vmc.hotkey_targets("Play.Execute");
    assert_eq!(targets, vec![(button.index, position)]);

    // переживает сохранение
    let reloaded = Vmc::parse(&vmc.to_bytes()).unwrap();
    assert_eq!(reloaded.hotkey_targets("Play.Execute").len(), 1);
}

#[test]
fn writes_and_reads_midi_mappings() {
    let mut vmc = Vmc::empty();
    let index = vmc.create_widget(&WidgetKind::MidiInterface, 10.0, 10.0).unwrap();
    vmc.set_widget_midi_mappings(
        index,
        &[
            MidiMapping {
                channel: 0,
                number: 36,
                link: "Play.Execute".into(),
                kind: "NoteOn".into(),
            },
            MidiMapping {
                channel: 1,
                number: 7,
                link: "Fader.Set".into(),
                kind: "ControlChange".into(),
            },
        ],
    )
    .unwrap();

    let reloaded = Vmc::parse(&vmc.to_bytes()).unwrap();
    let widget = reloaded.widgets_data().into_iter().find(|w| w.index == index).unwrap();
    assert_eq!(widget.kind, WidgetKind::MidiInterface);
    assert_eq!(widget.midi_map.len(), 2);
    assert_eq!(widget.midi_map[0].number, 36);
    assert_eq!(widget.midi_map[0].link, "Play.Execute");
    assert_eq!(widget.midi_map[1].kind, "ControlChange");
}

use vmix_config::MidiMapEntry as MidiMapping;

#[test]
fn new_buttons_get_standard_hotkeys() {
    let mut vmc = Vmc::empty();
    let index = vmc.create_widget(&WidgetKind::Button, 10.0, 10.0).unwrap();
    let button = vmc.widgets_data().into_iter().find(|w| w.index == index).unwrap();
    let names: Vec<String> = button.hotkeys.iter().map(|hotkey| hotkey.name.clone()).collect();
    assert_eq!(
        names,
        vec!["Execute", "Reset", "Clear Variables", "Press", "Release"],
        "кнопка должна получать штатные горячие клавиши, как в оригинале"
    );
    assert!(button.hotkeys.iter().all(|hotkey| !hotkey.active));
}

#[test]
fn writes_and_reads_streamdeck_keys() {
    let mut vmc = Vmc::empty();
    let index = vmc
        .create_widget(&WidgetKind::StreamDeck, 10.0, 10.0)
        .unwrap();
    vmc.set_widget_deck_keys(
        index,
        &[StreamDeckKey {
            context: "0".into(),
            link: "Play.Execute".into(),
            index: 0,
            extra: 0,
        },
        StreamDeckKey {
            context: "5".into(),
            link: "Fade.Execute".into(),
            index: 5,
            extra: 0,
        }],
    )
    .unwrap();

    let reloaded = Vmc::parse(&vmc.to_bytes()).unwrap();
    let widget = reloaded
        .widgets_data()
        .into_iter()
        .find(|widget| widget.index == index)
        .unwrap();
    assert_eq!(widget.kind, WidgetKind::StreamDeck);
    assert_eq!(widget.deck_keys.len(), 2);
    assert_eq!(widget.deck_keys[0].link, "Play.Execute");
    assert_eq!(widget.deck_keys[1].index, 5);
}

use vmix_config::DeckKey as StreamDeckKey;

#[test]
fn writes_and_reads_clock_events() {
    let mut vmc = Vmc::empty();
    let clock = vmc.create_widget(&WidgetKind::Clock, 10.0, 10.0).unwrap();
    vmc.set_widget_events(
        clock,
        &[
            ScheduledEvent {
                time: "09:30".into(),
                command: "Play.Execute".into(),
                days: EVERY_DAY,
            },
            ScheduledEvent {
                time: "18:00".into(),
                command: "Stop.Execute".into(),
                days: 0b0000_0101,
            },
        ],
    )
    .unwrap();

    let reloaded = Vmc::parse(&vmc.to_bytes()).unwrap();
    let widget = reloaded
        .widgets_data()
        .into_iter()
        .find(|widget| widget.index == clock)
        .unwrap();
    assert_eq!(widget.events.len(), 2);
    assert_eq!(widget.events[0].time, "09:30");
    assert_eq!(widget.events[0].command, "Play.Execute");
    assert_eq!(widget.events[0].days, EVERY_DAY);
    assert_eq!(widget.events[0].minutes(), Some(570));
    assert!(widget.events[0].runs_on(chrono::Weekday::Wed));

    // событие только по понедельникам и средам
    assert!(widget.events[1].runs_on(chrono::Weekday::Mon));
    assert!(widget.events[1].runs_on(chrono::Weekday::Wed));
    assert!(!widget.events[1].runs_on(chrono::Weekday::Tue));

    // расписание целиком
    assert_eq!(reloaded.scheduled_events().len(), 2);

    // разбор значений, как их пишет .NET
    assert_eq!(parse_days("Monday, Wednesday"), 0b0000_0101);
    assert_eq!(parse_days("Everyday"), EVERY_DAY);
    assert_eq!(parse_days("127"), 127);
    assert_eq!(parse_days(""), EVERY_DAY);
    assert_eq!(days_label(EVERY_DAY), "каждый день");
    assert_eq!(days_label(0b0000_0101), "пн, ср");
}

use vmix_config::{days_label, parse_days, ScheduledEvent, EVERY_DAY};
