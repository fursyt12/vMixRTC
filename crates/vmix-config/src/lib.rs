//! `.vmc` — файл контроллера vMixUTC.
//!
//! Формат: результат .NET `XmlSerializer` для класса `vMixController.Classes.VMixControl`:
//!
//! ```xml
//! <Root>
//!   <Controls><ArrayOfVMixControl>
//!     <vMixControl xsi:type="vMixControlRegion">…свойства…</vMixControl>
//!   </ArrayOfVMixControl></Controls>
//!   <WindowSettings><MainWindowSettings>…</MainWindowSettings></WindowSettings>
//!   <GlobalVariables><ArrayOfPairOfStringString>
//!     <PairOfStringString><A>key</A><B>value</B></PairOfStringString>
//!   </ArrayOfPairOfStringString></GlobalVariables>
//! </Root>
//! ```
//!
//! Первый этап порта читает оболочку и общие свойства виджета, сохраняя тело виджета
//! целиком: неизвестные пока свойства не теряются при чтении/записи.

use anyhow::{anyhow, Result};
use vmix_xml::Node;

mod widget;

pub use widget::{
    days_label, parse_days, DeckKey, ExternalData, MidiMapEntry, RelayTarget, Rgba, ScheduledEvent,
    WidgetCommand, WidgetData, WidgetHotkey, WidgetKind, WidgetPatch, EVERY_DAY,
};

pub const ROOT: &str = "Root";
pub const CONTROLS_PATH: [&str; 2] = ["Controls", "ArrayOfVMixControl"];
pub const WINDOW_PATH: [&str; 2] = ["WindowSettings", "MainWindowSettings"];
pub const GLOBALS_PATH: [&str; 2] = ["GlobalVariables", "ArrayOfPairOfStringString"];

#[derive(Debug, Clone)]
pub struct Vmc {
    pub root: Node,
}

impl Vmc {
    /// Пустой документ: оболочка `Root` с пустым списком виджетов, настройками окна
    /// и пустым словарём глобальных переменных — то же, что создаёт оригинал для
    /// нового контроллера.
    pub fn empty() -> Self {
        let mut root = Node::new(ROOT);

        let mut controls = Node::new("Controls");
        let mut array = Node::new("ArrayOfVMixControl");
        array.set_attr("xmlns:xsi", "http://www.w3.org/2001/XMLSchema-instance");
        array.set_attr("xmlns:xsd", "http://www.w3.org/2001/XMLSchema");
        controls.children.push(array);
        root.children.push(controls);

        let mut settings_wrapper = Node::new("WindowSettings");
        let mut settings = Node::new("MainWindowSettings");
        settings.set_child_text("State", "Normal");
        settings.set_child_text("Left", "100");
        settings.set_child_text("Top", "100");
        settings.set_child_text("Width", "1280");
        settings.set_child_text("Height", "800");
        settings.set_child_text("IP", "127.0.0.1");
        settings.set_child_text("Port", "8088");
        settings.set_child_text("Locked", "false");
        settings.set_child_text("UIScale", "1");
        settings.set_child_text("EnableLog", "false");
        settings.set_child_text("IsTopmost", "false");
        settings.set_child_text("ShowIndividualLock", "false");
        settings_wrapper.children.push(settings);
        root.children.push(settings_wrapper);

        let mut globals = Node::new("GlobalVariables");
        let mut pairs = Node::new("ArrayOfPairOfStringString");
        pairs.set_attr("xmlns:xsi", "http://www.w3.org/2001/XMLSchema-instance");
        pairs.set_attr("xmlns:xsd", "http://www.w3.org/2001/XMLSchema");
        globals.children.push(pairs);
        root.children.push(globals);

        Self { root }
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let root = vmix_xml::parse(bytes)?;
        if root.name != ROOT {
            return Err(anyhow!("ожидался <{ROOT}>, найден <{}>", root.name));
        }
        Ok(Self { root })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.root.to_document()
    }

    pub fn widget_nodes(&self) -> Vec<&Node> {
        self.root
            .path(&CONTROLS_PATH)
            .map(|array| array.children.iter().collect())
            .unwrap_or_default()
    }

    pub fn widgets(&self) -> Vec<Widget<'_>> {
        self.widget_nodes().into_iter().map(Widget::new).collect()
    }

    /// Сколько виджетов каждого типа (`xsi:type`) — удобно для сводки и тестов.
    pub fn widget_types(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for widget in self.widgets() {
            let Some(kind) = widget.type_name() else {
                continue;
            };
            match counts.iter_mut().find(|(name, _)| name == kind) {
                Some((_, count)) => *count += 1,
                None => counts.push((kind.to_string(), 1)),
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        counts
    }

    pub fn window_settings(&self) -> Option<WindowSettings> {
        self.root.path(&WINDOW_PATH).map(WindowSettings::from_node)
    }

    /// Запомнить блокировку окна: в оригинале заблокированный контроллер исполняет
    /// виджеты, разблокированный — редактирует их.
    pub fn set_window_locked(&mut self, locked: bool) -> Result<()> {
        let settings = self
            .root
            .path_mut(&WINDOW_PATH)
            .ok_or_else(|| anyhow!("в файле нет WindowSettings/MainWindowSettings"))?;
        settings.set_child_text("Locked", if locked { "true" } else { "false" });
        Ok(())
    }

    /// Установить глобальную переменную (их меняют скрипты кнопок).
    pub fn set_global_variable(&mut self, name: &str, value: &str) -> Result<()> {
        let array = self
            .root
            .path_mut(&GLOBALS_PATH)
            .ok_or_else(|| anyhow!("в файле нет GlobalVariables"))?;
        if let Some(pair) = array.children.iter_mut().find(|pair| {
            pair.name == "PairOfStringString" && pair.child_text("A") == Some(name)
        }) {
            pair.set_child_text("B", value);
            return Ok(());
        }
        let mut pair = Node::new("PairOfStringString");
        pair.set_child_text("A", name);
        pair.set_child_text("B", value);
        array.children.push(pair);
        Ok(())
    }

    pub fn global_variables(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let Some(array) = self.root.path(&GLOBALS_PATH) else {
            return out;
        };
        for pair in array.children_named("PairOfStringString") {
            if let (Some(key), Some(value)) = (pair.child_text("A"), pair.child_text("B")) {
                out.push((key.to_string(), value.to_string()));
            }
        }
        out
    }
}

/// Общие для всех виджетов поля (остальное — в `node`).
#[derive(Debug, Clone, Copy)]
pub struct Widget<'a> {
    pub node: &'a Node,
}

impl<'a> Widget<'a> {
    pub fn new(node: &'a Node) -> Self {
        Self { node }
    }

    /// `xsi:type` — реальный C#-тип виджета (`vMixControlRegion` и т.п.).
    pub fn type_name(&self) -> Option<&'a str> {
        self.node.attr_local("type")
    }

    /// Имя без префикса `vMixControl`.
    pub fn short_type(&self) -> Option<&'a str> {
        self.type_name()
            .map(|t| t.strip_prefix("vMixControl").unwrap_or(t))
    }

    pub fn name(&self) -> Option<&'a str> {
        self.node.child_text("Name")
    }

    pub fn page(&self) -> Option<&'a str> {
        self.node.child_text("Page")
    }

    pub fn property(&self, property: &str) -> Option<&'a str> {
        self.node.child_text(property)
    }

    pub fn number_property(&self, property: &str) -> Option<f64> {
        self.property(property)?.parse().ok()
    }

    pub fn bool_property(&self, property: &str) -> Option<bool> {
        match self.property(property)? {
            "true" | "True" | "1" => Some(true),
            "false" | "False" | "0" => Some(false),
            _ => None,
        }
    }

    /// Left, Top, Width, Height.
    pub fn geometry(&self) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
        (
            self.number_property("Left"),
            self.number_property("Top"),
            self.number_property("Width"),
            self.number_property("Height"),
        )
    }

    pub fn z_index(&self) -> Option<f64> {
        self.number_property("ZIndex")
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowSettings {
    pub state: Option<String>,
    pub ip: Option<String>,
    pub port: Option<String>,
    pub login: Option<String>,
    pub password: Option<String>,
    pub locked: bool,
    pub use_infinite_canvas: bool,
    pub show_links: bool,
    pub left: Option<f64>,
    pub top: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub ui_scale: Option<f64>,
    pub poll_time: Option<i64>,
    pub is_topmost: bool,
}

impl WindowSettings {
    pub fn from_node(node: &Node) -> Self {
        let text = |name: &str| node.child_text(name).map(str::to_string);
        let num = |name: &str| node.child_text(name).and_then(|v| v.parse().ok());
        let flag = |name: &str| {
            matches!(node.child_text(name), Some("true") | Some("True") | Some("1"))
        };
        Self {
            state: text("State"),
            ip: text("IP"),
            port: text("Port"),
            login: text("HttpLogin"),
            password: text("HttpPassword"),
            locked: flag("Locked"),
            use_infinite_canvas: flag("UseInfiniteCanvas"),
            show_links: flag("ShowLinks"),
            left: num("Left"),
            top: num("Top"),
            width: num("Width"),
            height: num("Height"),
            ui_scale: num("UIScale"),
            poll_time: node.child_text("PollTime").and_then(|v| v.parse().ok()),
            is_topmost: flag("IsTopmost"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"<?xml version="1.0" encoding="utf-8"?><Root>
<Controls><ArrayOfVMixControl xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<vMixControl xsi:type="vMixControlRegion"><Name>Timer</Name><Left>8</Left><Top>216</Top><Width>424</Width><Height>72</Height><ZIndex>-1</ZIndex></vMixControl>
<vMixControl xsi:type="vMixControlNewButton"><Name>Cut</Name><Left>1</Left><Top>2</Top><Width>3</Width><Height>4</Height></vMixControl>
</ArrayOfVMixControl></Controls>
<WindowSettings><MainWindowSettings><IP>10.0.0.5</IP><Port>8088</Port><Locked>true</Locked><UseInfiniteCanvas>false</UseInfiniteCanvas><UIScale>1</UIScale></MainWindowSettings></WindowSettings>
<GlobalVariables><ArrayOfPairOfStringString><PairOfStringString><A>TestVariable</A><B>Hello</B></PairOfStringString></ArrayOfPairOfStringString></GlobalVariables>
</Root>"#;

    #[test]
    fn reads_envelope_widgets_settings_and_globals() {
        let vmc = Vmc::parse(MINIMAL.as_bytes()).unwrap();
        assert_eq!(vmc.widget_nodes().len(), 2);
        assert_eq!(vmc.widget_types(), vec![("vMixControlNewButton".into(), 1), ("vMixControlRegion".into(), 1)]);
        assert_eq!(vmc.global_variables(), vec![("TestVariable".to_string(), "Hello".to_string())]);

        let settings = vmc.window_settings().unwrap();
        assert_eq!(settings.ip.as_deref(), Some("10.0.0.5"));
        assert_eq!(settings.port.as_deref(), Some("8088"));
        assert!(settings.locked);
    }

    #[test]
    fn widget_accessors_work() {
        let vmc = Vmc::parse(MINIMAL.as_bytes()).unwrap();
        let widgets = vmc.widgets();
        assert_eq!(widgets[0].type_name(), Some("vMixControlRegion"));
        assert_eq!(widgets[0].short_type(), Some("Region"));
        assert_eq!(widgets[0].name(), Some("Timer"));
        assert_eq!(widgets[0].geometry(), (Some(8.0), Some(216.0), Some(424.0), Some(72.0)));
        assert_eq!(widgets[1].z_index(), None);
    }

    #[test]
    fn rejects_foreign_documents() {
        assert!(Vmc::parse(b"<NotRoot/>").is_err());
    }
}
