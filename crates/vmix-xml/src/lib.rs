//! Минимальное lossless XML-дерево.
//!
//! Нужно по двум причинам:
//! 1. `.vmc` — это результат .NET `XmlSerializer` с полиморфизмом (`xsi:type`) и
//!    десятками полей на каждый виджет. Порт не должен молча терять поля, которых
//!    он пока не знает, поэтому дерево хранится целиком и пишется обратно.
//! 2. Состояние vMix (`/api`) — большой XML, из которого на первом этапе читается
//!    лишь часть; остальное остаётся доступным через то же дерево.
//!
//! Дерево намеренно простое: узлы с атрибутами, текстом и детьми. Для `.vmc` и
//! состояния vMix этого достаточно (смешанного контента там нет).

use anyhow::{anyhow, Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::io::Write;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

impl Node {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Атрибут по точному имени (`xsi:type`).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Атрибут по локальному имени: `type` найдёт и `type`, и `xsi:type`.
    pub fn attr_local(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == local || k.rsplit(':').next() == Some(local))
            .map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    pub fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.as_str())
    }

    /// Пройти по цепочке имён: `path(&["Controls", "ArrayOfVMixControl"])`.
    pub fn path<'a>(&'a self, path: &[&str]) -> Option<&'a Node> {
        let mut cur = self;
        for name in path {
            cur = cur.child(name)?;
        }
        Some(cur)
    }

    /// То же, но с изменяемым доступом — нужен для правки `.vmc`.
    pub fn path_mut<'a>(&'a mut self, path: &[&str]) -> Option<&'a mut Node> {
        let mut cur = self;
        for name in path {
            cur = cur.child_mut(name)?;
        }
        Some(cur)
    }

    pub fn child_mut(&mut self, name: &str) -> Option<&mut Node> {
        self.children.iter_mut().find(|c| c.name == name)
    }

    /// Установить текст дочернего элемента, создав его при необходимости.
    pub fn set_child_text(&mut self, name: &str, value: &str) {
        match self.child_mut(name) {
            Some(child) => child.text = value.to_string(),
            None => {
                let mut child = Node::new(name);
                child.text = value.to_string();
                self.children.push(child);
            }
        }
    }

    pub fn set_attr(&mut self, name: &str, value: &str) {
        match self.attrs.iter_mut().find(|(key, _)| key == name) {
            Some((_, current)) => *current = value.to_string(),
            None => self.attrs.push((name.to_string(), value.to_string())),
        }
    }

    /// Удалить первого ребёнка с таким именем.
    pub fn remove_child(&mut self, name: &str) -> Option<Node> {
        let position = self.children.iter().position(|c| c.name == name)?;
        Some(self.children.remove(position))
    }

    /// Все узлы поддерева с указанным именем (обход в глубину, сам узел не включается).
    pub fn descendants<'a>(&'a self, name: &'a str) -> Vec<&'a Node> {
        let mut out = Vec::new();
        self.collect(name, &mut out);
        out
    }

    fn collect<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            }
            c.collect(name, out);
        }
    }

    pub fn to_xml_string(&self) -> String {
        let mut out = Vec::new();
        self.write_xml(&mut out).expect("write to Vec cannot fail");
        String::from_utf8(out).expect("XML writer emits UTF-8")
    }

    /// Документ целиком: BOM + декларация + дерево — в том же виде, в каком `.vmc`
    /// пишет оригинальное приложение (оно читает файл через `XmlSerializer`).
    pub fn to_document(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        out.extend_from_slice(br#"<?xml version="1.0" encoding="utf-8"?>"#);
        self.write_xml(&mut out).expect("write to Vec cannot fail");
        out
    }

    pub fn write_xml<W: Write>(&self, w: &mut W) -> std::io::Result<()> {
        write!(w, "<{}", self.name)?;
        for (k, v) in &self.attrs {
            write!(w, " {}=\"{}\"", k, escape_attr(v))?;
        }
        if self.children.is_empty() && self.text.is_empty() {
            return write!(w, "/>");
        }
        write!(w, ">")?;
        if !self.text.is_empty() {
            write!(w, "{}", escape_text(&self.text))?;
        }
        for c in &self.children {
            c.write_xml(w)?;
        }
        write!(w, "</{}>", self.name)
    }
}

/// Разобрать XML (BOM и декларация допускаются).
pub fn parse(bytes: &[u8]) -> Result<Node> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    let mut reader = Reader::from_reader(bytes);
    {
        let cfg = reader.config_mut();
        cfg.trim_text_start = false;
        cfg.trim_text_end = false;
    }

    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => stack.push(start_node(&e)?),
            Ok(Event::Empty(e)) => {
                let node = start_node(&e)?;
                attach(&mut stack, &mut root, node);
            }
            Ok(Event::Text(t)) => {
                if let Some(top) = stack.last_mut() {
                    let decoded = t.unescape().context("не удалось раскодировать текст")?;
                    top.text.push_str(&decoded);
                }
            }
            Ok(Event::CData(c)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&String::from_utf8_lossy(c.as_ref()));
                }
            }
            Ok(Event::End(_)) => {
                if let Some(node) = stack.pop() {
                    attach(&mut stack, &mut root, node);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                return Err(anyhow!(
                    "ошибка XML на позиции {}: {e}",
                    reader.buffer_position()
                ))
            }
        }
        buf.clear();
    }

    if !stack.is_empty() {
        return Err(anyhow!("незакрытых элементов: {}", stack.len()));
    }
    root.ok_or_else(|| anyhow!("документ пуст"))
}

fn attach(stack: &mut Vec<Node>, root: &mut Option<Node>, node: Node) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => *root = Some(node),
    }
}

fn start_node(e: &BytesStart<'_>) -> Result<Node> {
    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let mut attrs = Vec::new();
    for attr in e.attributes() {
        let attr = attr.context("некорректный атрибут")?;
        let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
        let value = attr
            .unescape_value()
            .context("не удалось раскодировать значение атрибута")?
            .into_owned();
        attrs.push((key, value));
    }
    Ok(Node {
        name,
        attrs,
        children: Vec::new(),
        text: String::new(),
    })
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tree_and_keeps_namespaced_attrs() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?><Root><vMixControl xsi:type="vMixControlRegion"><Name>A &amp; B</Name></vMixControl></Root>"#;
        let root = parse(xml).unwrap();
        assert_eq!(root.name, "Root");
        let widget = root.child("vMixControl").unwrap();
        assert_eq!(widget.attr("xsi:type"), Some("vMixControlRegion"));
        assert_eq!(widget.attr_local("type"), Some("vMixControlRegion"));
        assert_eq!(widget.child_text("Name"), Some("A & B"));
    }

    #[test]
    fn round_trip_is_lossless_for_the_tree() {
        let xml = b"\xEF\xBB\xBF<?xml version=\"1.0\" encoding=\"utf-8\"?><a x=\"1\"><b/><c>text</c></a>";
        let first = parse(xml).unwrap();
        let reparsed = parse(&first.to_document()).unwrap();
        assert_eq!(first, reparsed);
    }

    #[test]
    fn finds_nested_nodes_by_path_and_descendants() {
        let root = parse(b"<r><a><b><c>1</c></b><b><c>2</c></b></a></r>").unwrap();
        assert_eq!(root.path(&["a", "b", "c"]).unwrap().text, "1");
        assert_eq!(root.descendants("c").len(), 2);
        assert_eq!(root.descendants("b").len(), 2);
    }
}
