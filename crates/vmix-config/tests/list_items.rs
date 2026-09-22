//! Круговой прогон `.vmc`: содержимое виджета «Список» (`<Items>`) не должно теряться.
//!
//! `verify` сравнивает только те поля, которые понимает модель, поэтому потерю «сырых»
//! элементов вроде `<Items>` он не заметит. Этот тест смотрит на записанный XML.

use std::path::PathBuf;

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

#[test]
fn list_items_survive_round_trip() {
    let bytes = std::fs::read(example("ProxySample.vmc")).expect("чтение ProxySample.vmc");
    let vmc = vmix_config::Vmc::parse(&bytes).expect("разбор");
    let written = vmc.to_bytes();
    let text = String::from_utf8_lossy(&written);

    assert!(text.contains("<Items>"), "потерян элемент <Items>");
    for item in ["1|PlayerOne", "2|PlayerTwo", "3|PlayerThree"] {
        assert!(
            text.contains(item),
            "потеряна строка списка {item}; записано: {}",
            &text[..text.len().min(400)]
        );
    }

    // повторный разбор даёт тот же XML — значит элементы действительно наши, а не случайный текст
    let again = vmix_config::Vmc::parse(&written).expect("повторный разбор");
    assert!(
        String::from_utf8_lossy(&again.to_bytes()).contains("3|PlayerThree"),
        "после повторного разбора строки списка исчезли"
    );
}

#[test]
fn set_widget_items_rewrites_the_list() {
    let bytes = std::fs::read(example("ProxySample.vmc")).expect("чтение");
    let mut vmc = vmix_config::Vmc::parse(&bytes).expect("разбор");

    // виджет «Список» — индекс 1 в этом файле
    let index = vmc
        .widgets_data()
        .iter()
        .position(|widget| widget.kind == vmix_config::WidgetKind::List)
        .expect("в файле нет виджета-списка");
    assert_eq!(vmc.widgets_data()[index].items.len(), 3, "исходные строки не прочитались");

    let items = vec!["1|Аня".to_string(), "2|Борис".to_string(), "3|Вера".to_string()];
    vmc.set_widget_items(index, &items).expect("запись строк");

    let reloaded = vmix_config::Vmc::parse(&vmc.to_bytes()).expect("повторный разбор");
    assert_eq!(reloaded.widgets_data()[index].items, items);
}
