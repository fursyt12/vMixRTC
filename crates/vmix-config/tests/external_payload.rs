//! Проверка полезной нагрузки, которую отправляет интерфейс при «Добавить источник данных».
use serde_json::json;

#[test]
fn ui_payload_deserializes_into_external_data() {
    let payload = json!({
        "isLive": false,
        "isTable": false,
        "isMappedToGUID": false,
        "text": "",
        "enabled": true,
        "restartData": false,
        "periodMs": 1000,
        "providerPath": "DataProviders\\XmlDataProvider.dll",
        "providerProperties": ["", "", "", "1"],
        "paths": [],
        "sourceName": "",
        "sourceData": ""
    });
    let external: vmix_config::ExternalData = serde_json::from_value(payload.clone())
        .unwrap_or_else(|error| panic!("интерфейс шлёт payload, который не разбирается: {error}"));
    assert!(!external.is_mapped_to_guid);
    assert_eq!(external.provider_path, "DataProviders\\XmlDataProvider.dll");
    assert_eq!(external.period_ms, 1000);

    // обратно в JSON — теми же именами, которые читает интерфейс
    let back = serde_json::to_value(&external).expect("сериализация");
    assert!(back.get("isMappedToGUID").is_some(), "потеряно имя isMappedToGUID: {back}");
    assert!(back.get("providerPath").is_some(), "потеряно имя providerPath: {back}");
}
