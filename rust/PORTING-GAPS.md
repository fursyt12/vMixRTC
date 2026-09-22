# Что ещё не перенесено на Rust

Документ фиксирует **всё**, что осталось в исходном C#/WPF-коде и ещё не имеет Rust-реализации.
Цель форка — оставить в проекте только Rust, поэтому для каждого пункта указано, чем его
заменять.

Масштаб исходников (для ориентира): **442 `.cs` + 40 `.xaml` ≈ 46 700 + 8 200 строк**.
Перенесено: **~9 900 строк Rust** (9 крейтов).

| проект | .cs | .xaml | строк | статус |
|---|---|---|---|---|
| `vMixController` (приложение) | 163 | 28 | 33 716 | 🔄 частично |
| `vMixAPI` (модель состояния и HTTP) | 27 | — | 2 718 | 🔄 частично |
| `vMixControllerSkin` (шаблоны/оформление) | 11 | 3 | 2 329 | ❌ |
| `vMixStreamDeckLibrary` | 15 | — | 1 614 | ❌ |
| `vMixUTCNDIMonitorDataProvider` | 7 | 1 | 1 561 | 🔄 частично |
| `Popcron.Sheets` | 164 | — | 2 680 | ➖ заменён своим клиентом |
| `UTCGoogleSheetsDataProvider` | 4 | 1 | 771 | ✅ |
| `vMixGenericXmlDataProvider` | 5 | 2 | 727 | ✅ |
| `UTCXLSXDataProvider` | 3 | 1 | 680 | ✅ |
| `JsonDataProvider` | 3 | 1 | 672 | ✅ |
| `NanoXML` | 3 | — | 419 | ➖ заменён `vmix-xml` |
| `FileSystemDataProvider` | 3 | 1 | 402 | ✅ (ядро) |
| `vMixWeatherExternalDataProvider` | 4 | 2 | 303 | ❌ |
| `HighPrecisionTimer` | 2 | — | 247 | ❌ |
| `vMixUTCStreamDeck` | 2 | — | 139 | ❌ |
| `vMixControllerDataProvider` | 3 | — | 91 | ✅ (интерфейс) |
| `ncalc` (submodule) | — | — | — | ➖ заменён своим вычислителем |
| `NDILibDotNet2` | 23 | — | 5 854 | ➖ заменён FFI к NDI runtime |

---

## 1. Ядро приложения

| модуль | строк | что не сделано | замена в Rust |
|---|---|---|---|
| `ViewModel/MainViewModel.cs` и рядом | ~4 380 | полный цикл синхронизации, автосохранение при выходе, восстановление после сбоя, «умные» пересчёты, менеджеры окон | дописать в `vmixrtc` (часть уже есть: поллинг, страницы, состояние) |
| `ViewModel/vMixWidgetSettingsViewModel.cs` | — | модель настроек виджета со всеми типами полей | расширить `WidgetPatch`/`extras` |
| `Classes/UpdateScheduler.cs` + `HighPrecisionTimer` | 247+ | точный планировщик с интервалами и приоритетами (сейчас — опрос 1 Гц) | `tokio`-интервалы или свой планировщик в `vmixrtc` |
| `Classes/XmlDocumentMessenger.cs`, `SharedData.cs`, `Messages.cs` | — | обмен данными между виджетами и внутренняя шина сообщений | канал (`tokio::sync::broadcast`) |
| `Classes/PerfMetrics.cs` | — | метрики производительности | `sysinfo` |
| `Classes/Hotkey.cs` + `KeyLearnWindow` | — | 🔄 **ссылки разобраны и работают** (`hotkey_targets`, `dispatch_link`); не сделано: системные горячие клавиши клавиатуры и окно обучения | `global-hotkey` |
| `Classes/Singleton.cs`, `Constants.cs`, `Utils.cs`, `Pair/Triple/Quadriple`, `One.cs`, `DummyStringProperty.cs` | — | вспомогательные типы (частично перенесены по смыслу) | уже не нужны как таковые |
| `Properties/` | 237 | настройки приложения, `Settings.settings` | `serde` + файл в XDG-каталоге |
| `App.xaml.cs`, `UTCSplashScreen`, `MainWindow.xaml` | ~750 | меню, панель инструментов, сплэш, drag&drop файлов, контекстные меню окна | `vmixrtc` (часть есть) |
| `Classes/BindingHelper.cs`, `BindingProxy.cs`, `Converters/*` (51 файл) | 2 294 | WPF-конвертеры значений (для UI-слоя) | не нужны: логика переносится в JS/DOM |

## 2. Виджеты

**Сделано (16):** область, кнопка, новая кнопка, текст, счёт, таймер, список, плейлист,
внешние данные, часы, громкость, Slider(алиас), TBar, просмотр переменных, контейнер,
MIDI-интерфейс (кроме MIDI-выхода), Stream Deck (без картинок и ручек).

**Не сделано:**

| виджет/поведение | строк | замена в Rust |
|---|---|---|
| `vMixControlMidiInterface` (MIDI In/Out, маппинг) | 1 виджет + `MidiLearnWindow` | 🔄 сделано (`midir`), кроме MIDI-**выхода** (LED/feedback) |
| `vMixControlStreamDeck` + `vMixStreamDeckLibrary` + `StreamDeckLearnWindow` | 1 753 | 🔄 сделано через прямой HID (оригинал ходил в плагин Elgato по WebSocket); остались картинки кнопок и ручки |
| изображения кнопок (`Image`, `ImageMax`, `ImageNumber`, `IsColorized`) | — | DOM/`<img>`, загрузка файлов в Tauri |
| мигание рамки (`BlinkBorderColor`) | 9 файлов | CSS-анимация |
| ссылки между виджетами (`WidgetLinksOverlay`) | 3 файла | SVG-слой поверх поверхности |
| выбор пути свойства (`vMixPathSelector`) | 2 файла | выпадающий список путей состояния |
| `vMixControlContainerDummy` | — | не нужен (служебный для конструктора WPF) |
| `TBarSlider`, `VolumeSlider`, `ReleaseButton` | — | поведение уже перенесено в JS |
| жесты: `vMixControlMoveThumb`, `vMixControlResizeThumb`, `WheelControlledScrollViewer` | — | указатели/DOM-события (есть) |
| `AutoStart`, `IsStateDependent`, `IsColorized`, `Style`-варианты кнопок | — | частично: подсветка по состоянию есть |
| `vMixControlSettingsView` (окно настроек vMix) | — | форма в `vmixrtc` |

## 3. Редакторы свойств

`PropertiesControls/*` — **30 файлов / 3 655 строк** (15 XAML-редакторов):
Bool, ComboBox, DataSource, FilePath, InputSelector, Int, Label, List, MidiMapping,
Scheduler, Script, StreamDeckMapping, String, TitleMapping.

В порте: общая панель свойств, редактор скрипта, настройки внешних данных, импорт контейнера.
**Не сделано:** редакторы по типам полей, выбор входа/пути тайтла, **планировщик**
(`SchedulerControl` + `Classes/ScheduledEvent.cs` — события часов!), маппинг MIDI/StreamDeck,
редактор кода с подсветкой (`BindableAvalonEditor` + AvalonEdit, 5 файлов).

## 4. Скриптовый слой

Реализовано: выражения, `_('путь')`, стек условий, `Else`/`ConditionEnd`, `GoTo`, `Timer`/`Delay`,
`SetVariable`/`SetGlobalVariable`, `ValueChanged`, `IsPressed`, `HasVariable`, `API`/`APIPOST`,
команды страниц, импорт/экспорт, GUI-редактор.

**Не сделано:**

* нативные функции: `If`/`EndIf`, `SetButtonColor`, `LIVEOn/Off/Toggle`, `Win`,
  `SyncState`/`SyncInternalButtonState` (`ExecLink` сделан, все функции из примеров исполняются);
* полный набор выражений NCalc (в примерах используются `getvalue`, `split`, `len`, `_()`,
  арифметика, строки — часть есть, полного паритета нет);
* `Classes/Scripting/ScriptLoopAnalyzer.cs` — статический анализ зацикливаний
  (в порте только счётчик переходов);
* `ScriptExecutionLoopGuard.cs` — защита исполнения;
* новые кнопки: `vMixControlNewButtonCommand`/`Helper`, `vMixFunctionReference`,
  `vMixNewFunctionReference` — подстановки `$Value/$InputKey/$Mix/$Channel/$SelectedIndex/$IntMinus`
  (частично есть в `vmix-functions`);
* `ApiRequestManager` — очередь и батчинг запросов к vMix (в порте отправка синхронная).

## 5. Данные и провайдеры

**Сделано:** XML, JSON, Excel, Google Sheets, NDI-источники, файлы; источник `http(s)` с
заголовками; связь «источник → потребитель»; применение строк к тайтлам; просмотр строк;
`IsMappedToGUID`.

**Не сделано:** провайдер **погоды** (`vMixWeatherExternalDataProvider`, 303 строки);
**OMT**-транспорт (папка `OMT` в NDI-провайдере); аудио NDI; mDNS-обнаружение NDI без runtime;
окна свойств провайдера (`OnWidgetUI.xaml`, `PropertiesWindow`) вместо полей в панели;
запись в Excel/Sheets (порт только читает); `Extensions/` NDI-провайдера.

## 6. Железо и нативные SDK

| что | статус | путь в Rust |
|---|---|---|
| MIDI (in/out, learn, маппинг на команды) | 🔄 модель, отображения, диспетчер и UI сделаны; `midir` подключён. **Не проверено с железом**: в песочнице нет `/dev/snd/seq`, включается тестовый источник | уже `midir` |
| Stream Deck (кнопки, экраны, learn) | 🔄 сделан **прямой HID** (`hidapi`): модели, отчёты, фронты, яркость, ссылки, обучение. Не проверено с устройством; не сделаны картинки кнопок и ручки Stream Deck + | уже `hidapi` |
| NDI (обнаружение + приём видео) | 🔄 FFI написан, в бою не проверен | FFI к NDI runtime — **чистым Rust быть не может**, SDK закрытый |
| OMT (Open Media Transport) | ❌ | FFI, если понадобится |
| .NET-провайдеры из `.vmc` (`DataProviderContent`) | ➖ осознанно не поддерживаются | заменены встроенными провайдерами; поле в файле сохраняется |

## 7. Оформление, локализация, платформа

* **Локализация**: `LocalizationManager` (11 файлов) + `loc:Loc` + `UserLocales/*.json` —
  в порте строки зашиты по-русски; замена — `fluent`/`rust-i18n` + JSON-файлы локалей.
* **Скины/темы**: `Skins/*.xaml` (1 623), `vMixControllerSkin` (2 329 + спутниковые DLL) —
  оформление WPF; в порте — CSS-темы (сейчас одна).
* **Цвета**: `ColorNamePair`, палитра AvalonEdit, `Images/` — палитра именованных цветов и
  картинки интерфейса.
* **Упаковка**: Costura Fody (встраивание DLL), Windows-установщик и автообновление,
  подпись, `packages.config`/NuGet — замена: `tauri-plugin-updater`, `tray-icon`,
  сборки под Windows/Linux/ARM/Android/iOS (пункт 8 плана).
* **WPF-специфика, которую переносить не нужно** (но её поведение проверять):
  `TypeTemplateSelector`, `MenuStyleSelector`, `Converters/*`, `Extensions/*` behaviors
  (AutoGridRow, IgnoreMouseWheel, ScrollIntoView и т. п.).

## 8. Приоритеты (Rust-only дорога)

1. **Системные горячие клавиши** (`global-hotkey`) — ссылки, MIDI, Stream Deck и расписание уже работают, не хватает привязки к клавиатуре и окна обучения.
2. **Мелочи виджетов**: картинки кнопок, мигание рамки, ссылки-оверлей, поворот ручек Stream Deck, MIDI-выход.
3. **Проверка железа** на машине с MIDI-секвенсором, Stream Deck и NDI runtime.
4. **Нативные функции без примеров**: `If`/`EndIf`, `SetButtonColor`, `LIVE*`, `Sync*`.
5. **Локализация** (`fluent`) и вторая тема оформления.
6. **Обновления/упаковка** (`tauri-plugin-updater`), сборки под ARM/Android/iOS.
7. **Новые кнопки и остаток нативных функций** скриптов (`If`, `SetButtonColor`, `LIVE*`, `Sync*`).
8. **Провайдер погоды, OMT, аудио NDI** — по потребности.
9. **Проверка NDI FFI на машине с NDI runtime** — единственный непроверенный участок.

Что **принципиально** останется вне чистого Rust: приём/передача NDI (закрытый SDK — только FFI)
и, при необходимости, OMT. Всё остальное — включая MIDI, Stream Deck, Excel, Sheets, локализацию,
обновления и упаковку — закрывается обычными Rust-крейтами.
