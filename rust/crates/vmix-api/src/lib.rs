//! Порт `vMixAPI`: клиент Web API vMix и разбор состояния.
//!
//! Соответствие оригиналу (C#):
//! * URL функции — `http://{ip}:{port}/api?` + `Function=...&Param=...`
//!   (`State.SendFunction`, `vMixAPI/State.cs:552`);
//! * авторизация — HTTP Basic из `login:password` (`StateFabrique.GetCredentials`
//!   и `SendWithAuthAsync` в `ApiRequestManager.cs`);
//! * состояние — XML с корнем `<vmix>`, разбираемый `XmlSerializer(typeof(State))`;
//!   имена атрибутов взяты из `[XmlAttribute]` в `vMixAPI/*.cs`.

use anyhow::{anyhow, Context, Result};
use std::time::Duration;
use vmix_xml::Node;

pub const DEFAULT_PORT: u16 = 8088;

// ---------------------------------------------------------------- клиент

pub struct VmixClient {
    host: String,
    port: u16,
    credentials: Option<(String, String)>,
    agent: ureq::Agent,
}

impl VmixClient {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .build();
        Self {
            host: host.into(),
            port,
            credentials: None,
            agent,
        }
    }

    pub fn with_credentials(
        mut self,
        login: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.credentials = Some((login.into(), password.into()));
        self
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://{host}:{port}/api` — как в оригинале (там к базе добавляется `?`).
    pub fn api_url(&self) -> String {
        format!("http://{}:{}/api", self.host, self.port)
    }

    /// URL вызова функции: `http://host:port/api?Function=Cut&Input=1`.
    pub fn function_url(&self, function: &str, params: &[(&str, &str)]) -> String {
        let mut url = format!("{}?Function={}", self.api_url(), encode(function));
        for (key, value) in params {
            url.push('&');
            url.push_str(&encode(key));
            url.push('=');
            url.push_str(&encode(value));
        }
        url
    }

    pub fn send_function(&self, function: &str, params: &[(&str, &str)]) -> Result<String> {
        let url = self.function_url(function, params);
        self.get(&url)
    }

    /// Отправить готовую строку запроса (её строит каталог функций по `FormatString`),
    /// например `Function=SetVolume&Input=1&Value=50`.
    pub fn send_query(&self, query: &str) -> Result<String> {
        let url = format!("{}?{}", self.api_url(), query.trim_start_matches('?'));
        self.get(&url)
    }

    /// Запрос по произвольному URL — функции `API` и `APIPOST` в скриптах.
    pub fn fetch_url(&self, url: &str, post: bool) -> Result<String> {
        let mut request = if post {
            self.agent.post(url)
        } else {
            self.agent.get(url)
        };
        if let Some((login, password)) = &self.credentials {
            request = request.set("Authorization", &basic_auth(login, password));
        }
        let response = if post {
            request.send_string("")
        } else {
            request.call()
        };
        let response = response.with_context(|| format!("запрос {url}"))?;
        response.into_string().context("чтение ответа")
    }

    pub fn fetch_state_xml(&self) -> Result<String> {
        self.get(&self.api_url())
    }

    pub fn state(&self) -> Result<VmixState> {
        VmixState::parse(&self.fetch_state_xml()?)
    }

    fn get(&self, url: &str) -> Result<String> {
        let mut request = self.agent.get(url);
        if let Some((login, password)) = &self.credentials {
            request = request.set("Authorization", &basic_auth(login, password));
        }
        let response = request
            .call()
            .with_context(|| format!("запрос к vMix: {url}"))?;
        response
            .into_string()
            .context("чтение ответа vMix")
    }
}

fn basic_auth(login: &str, password: &str) -> String {
    format!("Basic {}", base64(format!("{login}:{password}").as_bytes()))
}

// ---------------------------------------------------------------- состояние

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Input {
    pub number: Option<i64>,
    pub key: Option<String>,
    pub title: String,
    pub kind: Option<String>,
    pub state: Option<String>,
    pub position: Option<i64>,
    pub duration: Option<i64>,
    pub muted: Option<bool>,
    pub volume: Option<i64>,
    pub audiobusses: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Output {
    pub kind: Option<String>,
    pub number: Option<i64>,
    pub source: Option<String>,
    pub ndi: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Overlay {
    pub number: Option<i64>,
    pub preview: Option<bool>,
    /// Текст элемента — ключ или номер входа.
    pub input: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Transition {
    pub number: Option<i64>,
    pub effect: Option<String>,
    pub duration: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioBus {
    pub name: String,
    pub volume: Option<i64>,
    pub muted: Option<bool>,
    pub meter_f1: Option<f64>,
    pub meter_f2: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Mix {
    pub number: Option<i64>,
    pub preview: Option<i64>,
    pub active: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VmixState {
    pub version: Option<String>,
    pub edition: Option<String>,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub overlays: Vec<Overlay>,
    pub transitions: Vec<Transition>,
    pub audio: Vec<AudioBus>,
    pub mixes: Vec<Mix>,
    pub active: Option<i64>,
    pub preview: Option<i64>,
    pub recording: bool,
    pub streaming: bool,
    pub external: bool,
    pub playlist: bool,
    pub multicorder: bool,
    pub fade_to_black: bool,
    /// Полное дерево ответа: всё, что ещё не разобрано, остаётся доступным.
    /// В JSON для UI не отдаём — это внутреннее представление.
    #[serde(skip)]
    pub raw: Node,
}

impl VmixState {
    pub fn parse(xml: &str) -> Result<Self> {
        let root = vmix_xml::parse(xml.as_bytes())?;
        if root.name != "vmix" {
            return Err(anyhow!("ожидался <vmix>, найден <{}>", root.name));
        }
        Ok(Self::from_node(root))
    }

    pub fn from_node(root: Node) -> Self {
        let text = |name: &str| root.child_text(name).map(str::to_string);

        let inputs = root
            .path(&["inputs"])
            .map(|inputs| {
                inputs
                    .children_named("input")
                    .map(|node| Input {
                        number: attr_i64(node, "number"),
                        key: node.attr("key").map(str::to_string),
                        title: node
                            .attr("title")
                            .map(str::to_string)
                            .unwrap_or_else(|| node.text.clone()),
                        kind: node.attr("type").map(str::to_string),
                        state: node.attr("state").map(str::to_string),
                        position: attr_i64(node, "position"),
                        duration: attr_i64(node, "duration"),
                        muted: attr_bool(node, "muted"),
                        volume: attr_i64(node, "volume"),
                        audiobusses: node.attr("audiobusses").map(str::to_string),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let outputs = root
            .path(&["outputs"])
            .map(|outputs| {
                outputs
                    .children_named("output")
                    .map(|node| Output {
                        kind: node.attr("type").map(str::to_string),
                        number: attr_i64(node, "number"),
                        source: node.attr("source").map(str::to_string),
                        ndi: node.attr("ndi").map(str::to_string),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let overlays = root
            .path(&["overlays"])
            .map(|overlays| {
                overlays
                    .children_named("overlay")
                    .map(|node| Overlay {
                        number: attr_i64(node, "number"),
                        preview: attr_bool(node, "preview"),
                        input: if node.text.is_empty() {
                            None
                        } else {
                            Some(node.text.clone())
                        },
                    })
                    .collect()
            })
            .unwrap_or_default();

        let transitions = root
            .path(&["transitions"])
            .map(|transitions| {
                transitions
                    .children_named("transition")
                    .map(|node| Transition {
                        number: attr_i64(node, "number"),
                        effect: node.attr("effect").map(str::to_string),
                        duration: attr_i64(node, "duration"),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let audio = root
            .path(&["audio"])
            .map(|audio| {
                audio
                    .children
                    .iter()
                    .map(|node| AudioBus {
                        name: node.name.clone(),
                        volume: attr_i64(node, "volume"),
                        muted: attr_bool(node, "muted"),
                        meter_f1: attr_f64(node, "meterF1"),
                        meter_f2: attr_f64(node, "meterF2"),
                    })
                    .collect()
            })
            .unwrap_or_default();

        // В vMix элементы <mix> лежат прямо в корне (в C#: [XmlElement("mix")]).
        let mixes = root
            .children_named("mix")
            .map(|node| Mix {
                number: attr_i64(node, "number"),
                preview: child_i64(node, "preview"),
                active: child_i64(node, "active"),
            })
            .collect();

        Self {
            version: text("version"),
            edition: text("edition"),
            inputs,
            outputs,
            overlays,
            transitions,
            audio,
            mixes,
            active: text("active").and_then(|v| v.parse().ok()),
            preview: text("preview").and_then(|v| v.parse().ok()),
            recording: flag(&root, "recording"),
            streaming: flag(&root, "streaming"),
            external: flag(&root, "external"),
            playlist: flag(&root, "playList"),
            multicorder: flag(&root, "multiCorder"),
            fade_to_black: flag(&root, "fadeToBlack"),
            raw: root,
        }
    }

    pub fn input_by_number(&self, number: i64) -> Option<&Input> {
        self.inputs.iter().find(|i| i.number == Some(number))
    }

    pub fn input_by_key(&self, key: &str) -> Option<&Input> {
        self.inputs.iter().find(|i| i.key.as_deref() == Some(key))
    }
}

fn attr_i64(node: &Node, name: &str) -> Option<i64> {
    node.attr(name).and_then(|v| v.parse().ok())
}

fn attr_f64(node: &Node, name: &str) -> Option<f64> {
    node.attr(name)
        .and_then(|v| v.replace(',', ".").parse().ok())
}

fn attr_bool(node: &Node, name: &str) -> Option<bool> {
    node.attr(name).map(|v| matches!(v, "True" | "true" | "1"))
}

fn child_i64(node: &Node, name: &str) -> Option<i64> {
    node.child_text(name).and_then(|v| v.parse().ok())
}

fn flag(node: &Node, name: &str) -> bool {
    matches!(node.child_text(name), Some("True") | Some("true") | Some("1"))
}

// ---------------------------------------------------------------- утилиты

/// Процентное кодирование параметров (значения вида `Colour 1`, `a&b`).
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = r#"<vmix>
<version>29.0.0.1</version><edition>4K</edition>
<inputs>
<input key="a1b2" number="1" type="Colour" title="Colour 1" state="Running" position="0" duration="0" muted="False" volume="100" audiobusses="M">Colour 1</input>
<input key="c3d4" number="2" type="Video" title="Clip.mp4" state="Paused" position="1234" duration="5000" muted="True" volume="50" audiobusses="M,A">Clip.mp4</input>
</inputs>
<overlays><overlay number="1" preview="False">a1b2</overlay><overlay number="2" preview="True"/></overlays>
<transitions><transition number="1" effect="Cut" duration="0"/><transition number="2" effect="Fade" duration="500"/></transitions>
<audio><master volume="100" muted="False" meterF1="0.5" meterF2="0.25"/><busA volume="80" muted="True"/></audio>
<mix number="1"><preview>1</preview><active>2</active></mix>
<mix number="2"><preview>3</preview><active>4</active></mix>
<recording>False</recording><streaming>True</streaming><external>False</external>
<playList>False</playList><multiCorder>False</multiCorder><fadeToBlack>False</fadeToBlack>
<active>2</active><preview>1</preview>
</vmix>"#;

    #[test]
    fn function_url_matches_the_original_format() {
        let client = VmixClient::new("127.0.0.1", 8088);
        assert_eq!(
            client.function_url("SetFader", &[("Value", "50")]),
            "http://127.0.0.1:8088/api?Function=SetFader&Value=50"
        );
        assert_eq!(
            client.function_url("Cut", &[("Input", "Colour 1")]),
            "http://127.0.0.1:8088/api?Function=Cut&Input=Colour%201"
        );
    }

    #[test]
    fn basic_auth_header_is_built_from_login_and_password() {
        assert_eq!(basic_auth("admin", "secret"), "Basic YWRtaW46c2VjcmV0");
    }

    #[test]
    fn parses_state_like_the_original() {
        let state = VmixState::parse(STATE).unwrap();
        assert_eq!(state.version.as_deref(), Some("29.0.0.1"));
        assert_eq!(state.edition.as_deref(), Some("4K"));
        assert_eq!(state.inputs.len(), 2);
        assert_eq!(state.inputs[0].title, "Colour 1");
        assert_eq!(state.inputs[1].volume, Some(50));
        assert_eq!(state.inputs[1].muted, Some(true));
        assert_eq!(state.active, Some(2));
        assert_eq!(state.preview, Some(1));
        assert!(!state.recording);
        assert!(state.streaming);

        assert_eq!(state.overlays.len(), 2);
        assert_eq!(state.overlays[0].input.as_deref(), Some("a1b2"));

        assert_eq!(state.transitions[1].effect.as_deref(), Some("Fade"));
        assert_eq!(state.transitions[1].duration, Some(500));

        assert_eq!(state.audio.len(), 2);
        assert_eq!(state.audio[0].name, "master");
        assert_eq!(state.audio[0].meter_f1, Some(0.5));
        assert_eq!(state.audio[1].muted, Some(true));

        assert_eq!(state.mixes.len(), 2);
        assert_eq!(state.mixes[1].active, Some(4));

        assert_eq!(state.input_by_number(2).unwrap().key.as_deref(), Some("c3d4"));
        assert_eq!(state.input_by_key("a1b2").unwrap().number, Some(1));
    }

    #[test]
    fn rejects_foreign_documents() {
        assert!(VmixState::parse("<other/>").is_err());
    }

    /// Живая проверка против реального vMix: `VMIX_HOST=127.0.0.1 cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_state_round_trip() {
        let host = std::env::var("VMIX_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port = std::env::var("VMIX_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        let state = VmixClient::new(host, port).state().unwrap();
        println!(
            "vMix {} / входов: {} / активный: {:?}",
            state.version.as_deref().unwrap_or("?"),
            state.inputs.len(),
            state.active
        );
        assert!(!state.inputs.is_empty());
    }
}
