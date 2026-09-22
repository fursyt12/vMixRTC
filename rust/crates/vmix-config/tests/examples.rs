//! Golden-тесты на реальных файлах из репозитория: порт обязан читать те же `.vmc`,
//! что и оригинальное приложение, и не терять данные при чтении/записи.

use std::fs;
use std::path::PathBuf;
use vmix_config::Vmc;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn vmc_files() -> Vec<PathBuf> {
    let dir = examples_dir();
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("нет каталога {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e == "vmc").unwrap_or(false))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "в {} нет .vmc", dir.display());
    files
}

#[test]
fn every_example_parses_and_round_trips() {
    for file in vmc_files() {
        let bytes = fs::read(&file).expect("read .vmc");
        let vmc = Vmc::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", file.display()));

        let widgets = vmc.widget_nodes();
        assert!(!widgets.is_empty(), "{}: виджетов нет", file.display());
        for widget in &widgets {
            assert!(
                widget.attr_local("type").is_some(),
                "{}: у виджета <{}> нет xsi:type",
                file.display(),
                widget.name
            );
        }

        let settings = vmc
            .window_settings()
            .unwrap_or_else(|| panic!("{}: нет WindowSettings", file.display()));
        assert!(settings.port.is_some(), "{}: нет порта", file.display());

        let again = Vmc::parse(&vmc.to_bytes()).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        assert_eq!(vmc.root, again.root, "{}: данные потерялись при записи", file.display());

        println!(
            "{:<28} виджетов: {:>2}  типов: {:>2}  порт: {}",
            file.file_name().unwrap().to_string_lossy(),
            widgets.len(),
            vmc.widget_types().len(),
            settings.port.as_deref().unwrap_or("?")
        );
    }
}

#[test]
fn scoreboard_example_matches_the_original_app() {
    let file = examples_dir().join("Scoreboard.vmc");
    let vmc = Vmc::parse(&fs::read(&file).unwrap()).unwrap();

    let settings = vmc.window_settings().unwrap();
    assert_eq!(settings.ip.as_deref(), Some("127.0.0.1"));
    assert_eq!(settings.port.as_deref(), Some("8088"));

    assert!(vmc.widgets().len() >= 5, "виджетов: {}", vmc.widgets().len());
    assert!(!vmc.widget_types().is_empty());
    assert_eq!(
        vmc.global_variables(),
        vec![("TestVariable".to_string(), "Hello".to_string())]
    );

    // у каждого виджета есть геометрия — значит общие свойства читаются
    for widget in vmc.widgets() {
        let (left, top, width, height) = widget.geometry();
        assert!(left.is_some() && top.is_some() && width.is_some() && height.is_some(),
            "виджет {:?} без геометрии", widget.type_name());
    }
}
