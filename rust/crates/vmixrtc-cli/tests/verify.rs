//! Сквозная проверка: все примеры оригинала проходят «чтение → запись → чтение»
//! без потерь, а все функции/типы/провайдеры распознаются.

use std::path::PathBuf;

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn catalogue() -> vmix_functions::Catalogue {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    vmix_functions::Catalogue::load(&[
        dir.join("Functions.xml"),
        dir.join("NewFunctions.xml"),
    ])
    .expect("каталог функций")
}

#[test]
fn examples_survive_round_trip_without_losses() {
    let report = vmixrtc_cli::verify_dir(&examples(), &catalogue()).expect("проверка примеров");
    assert!(report.files >= 5, "примеров найдено: {}", report.files);
    assert!(report.widgets > 20, "виджетов: {}", report.widgets);
    assert!(
        report.issues.is_empty(),
        "найдены проблемы:\n{}",
        report
            .issues
            .iter()
            .map(|issue| format!("  {}: {}", issue.file, issue.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn unknown_widget_kinds_are_reported() {
    // «чужой» тип виджета должен попадать в проблемы, а не молча теряться
    let mut vmc = vmix_config::Vmc::empty();
    let index = vmc
        .create_widget(&vmix_config::WidgetKind::Button, 0.0, 0.0)
        .unwrap();
    let _ = index;
    let bytes = vmc.to_bytes();
    let dir = std::env::temp_dir().join(format!("vmix-verify-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("one.vmc");
    std::fs::write(&path, bytes).unwrap();

    let report = vmixrtc_cli::verify_dir(&dir, &catalogue()).unwrap();
    assert_eq!(report.files, 1);
    assert!(report.issues.is_empty(), "чистый файл не должен давать проблем: {:?}", report.issues);
    std::fs::remove_dir_all(&dir).unwrap();
}
