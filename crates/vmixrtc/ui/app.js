// Рабочая поверхность vMixUTC.
//
// Данные виджетов живут в Rust (`vmix-config`), здесь — только отрисовка и правки:
// контейнер виджета повторяет оригинал (фон #1E2328, рамка BorderColor, шапка Color
// с авто-чёрным/белым текстом, выделение заливкой #7F00FFFF).

const { invoke } = window.__TAURI__.core;

const PAGES_FALLBACK = ["MAIN", "DATA", "PAGE1", "PAGE2", "PAGE3", "PAGE4", "PAGE5"];
const ZOOM_MIN = 0.25;
const ZOOM_MAX = 2;

const state = {
  doc: { path: null, pages: PAGES_FALLBACK.slice(), widgets: [] },
  view: { page: 0, zoom: 1 },
  selected: null,
  connection: { host: "127.0.0.1", port: 8088, login: null, password: null },
  vmix: null,
  pollTimer: null,
  palette: [],
  functionMeta: {},
  externals: {},
  lang: "ru",
  locale: {},
  midi: { device: "" },
  midiPorts: [],
  deck: { device: "" },
  deckDevices: [],
  // Заблокирован = режим пульта: виджеты исполняются, а не редактируются
  // (в оригинале это WindowSettings.Locked).
  locked: false,
  functions: [],
};

const $ = (id) => document.getElementById(id);
const canvas = $("canvas");
const surface = $("surface");

// ------------------------------------------------------------------ утилиты

let toastTimer = null;
// ------------------------------------------------------------------ локализация

/// Загрузить словарь и применить переводы к разметке.
async function loadLocale(lang) {
  try {
    const response = await fetch(`locales/${lang}.json`);
    state.locale = await response.json();
  } catch (error) {
    state.locale = {};
    console.warn("словарь не загрузился", error);
  }
  state.lang = lang;
  document.documentElement.lang = lang;
  applyI18n();
}

/// Перевод по ключу с подстановкой `{0}`, `{1}`, … — как `string.Format` в оригинале.
function t(key, ...params) {
  let text = state.locale?.[key] ?? key;
  params.forEach((value, index) => {
    text = text.split(`{${index}}`).join(String(value));
  });
  return text;
}

/// Применить переводы к статичной разметке.
function applyI18n() {
  for (const element of document.querySelectorAll("[data-i18n]")) {
    element.textContent = t(element.dataset.i18n);
  }
  for (const element of document.querySelectorAll("[data-i18n-title]")) {
    element.title = t(element.dataset.i18nTitle);
  }
  for (const element of document.querySelectorAll("[data-i18n-placeholder]")) {
    element.placeholder = t(element.dataset.i18nPlaceholder);
  }
  const lock = $("lock");
  if (lock) lock.textContent = `${state.locked ? "🔒" : "🔓"} ${t(state.locked ? "toolbar.panel" : "toolbar.editor")}`;
}

function toast(message, kind = "") {
  const node = $("toast");
  node.textContent = message;
  node.className = `toast${kind ? " " + kind : ""}`;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => node.classList.add("hidden"), 2600);
}

/// { a, r, g, b } → "#RRGGBB"
function hex(color) {
  const part = (value) => Math.max(0, Math.min(255, value | 0)).toString(16).padStart(2, "0");
  return `#${part(color.r)}${part(color.g)}${part(color.b)}`;
}

/// "#RRGGBB" → { a: 255, r, g, b }
function rgba(value) {
  const text = String(value).replace("#", "");
  return {
    a: 255,
    r: parseInt(text.slice(0, 2), 16) || 0,
    g: parseInt(text.slice(2, 4), 16) || 0,
    b: parseInt(text.slice(4, 6), 16) || 0,
  };
}

/// Чёрный или белый текст по яркости фона — как ColorToBlackOrWhiteConverter в оригинале.
function autoText(color) {
  const luminance = (0.299 * color.r + 0.587 * color.g + 0.114 * color.b) / 255;
  return luminance > 0.55 ? "#000000" : "#FFFFFF";
}

const byIndex = (index) => state.doc.widgets.find((w) => w.index === index);

// ------------------------------------------------------------------ документ

async function applyDoc(promise, message) {
  try {
    const doc = await promise;
    state.doc = doc;
    state.locked = Boolean(doc.locked);
    document.body.classList.toggle("locked", state.locked);
    if (state.selected !== null && !byIndex(state.selected)) state.selected = null;
    if (doc.path) $("path").value = doc.path;
    render();
    if (message) toast(message);
  } catch (error) {
    toast(String(error), "error");
  }
}

const newDocument = () => applyDoc(invoke("vmc_new"), "новый контроллер");
const openDocument = () => {
  const path = $("path").value.trim();
  if (!path) return toast("укажите путь к .vmc", "error");
  applyDoc(invoke("vmc_open", { path }), "открыт");
};
const saveDocument = () =>
  applyDoc(invoke("vmc_save", { path: $("path").value.trim() || null }), "сохранено");
const addWidget = (typeName, left, top) => applyDoc(invoke("vmc_add", { typeName, left, top }));
const updateWidget = (index, patch) => applyDoc(invoke("vmc_update", { index, patch }));
const removeWidget = (index) => applyDoc(invoke("vmc_remove", { index }), "виджет удалён");
const duplicateWidget = (index) => applyDoc(invoke("vmc_duplicate", { index }), "виджет скопирован");

// ------------------------------------------------------------------ отрисовка

function render() {
  renderPages();
  renderCanvas();
  renderZoom();
  renderLock();
  renderVmix();
}

function renderLock() {
  const button = $("lock");
  button.textContent = `${state.locked ? "🔒" : "🔓"} ${t(state.locked ? "toolbar.panel" : "toolbar.editor")}`;
  button.classList.toggle("primary", state.locked);
  button.title = state.locked
    ? "Режим пульта: виджеты исполняются. Нажмите, чтобы редактировать"
    : "Режим редактора: виджеты двигаются. Нажмите, чтобы включить пульт";
}

function renderPages() {
  const container = $("pages");
  container.replaceChildren();
  state.doc.pages.forEach((name, index) => {
    const button = document.createElement("button");
    button.className = `page${index === state.view.page ? " active" : ""}`;
    button.textContent = name;
    button.addEventListener("click", () => {
      state.view.page = index;
      state.selected = null;
      render();
    });
    container.appendChild(button);
  });
}

function renderZoom() {
  $("zoom").textContent = `${Math.round(state.view.zoom * 100)}%`;
}

function renderCanvas() {
  canvas.style.transform = `scale(${state.view.zoom})`;
  for (const element of canvas.querySelectorAll(".widget")) element.remove();

  const widgets = state.doc.widgets
    .filter((w) => w.page === state.view.page)
    .sort((a, b) => a.zIndex - b.zIndex || a.index - b.index);

  for (const widget of widgets) canvas.appendChild(widgetElement(widget));
  renderProperties();
}

function widgetElement(widget, options = {}) {
  const nested = options.nested === true;
  const container = options.container ?? null;
  const element = document.createElement("div");
  element.className = nested ? "widget nested" : "widget";
  element.dataset.index = widget.index;
  element.style.left = `${widget.left}px`;
  element.style.top = `${widget.top}px`;
  element.style.width = `${widget.width}px`;
  element.style.height = `${widget.height}px`;
  element.style.zIndex = String(widget.zIndex + 100);
  element.style.setProperty("--border-color", hex(widget.borderColor));
  if (!nested && widget.index === state.selected && !state.locked) element.classList.add("selected");

  if (widget.captionOn && widget.captionVisible) element.appendChild(captionElement(widget, state.locked));
  element.appendChild(contentElement(widget));

  if (!nested && !state.locked && widget.index === state.selected && !widget.locked) {
    for (const direction of ["e", "s", "se"]) {
      const handle = document.createElement("div");
      handle.className = `handle ${direction}`;
      handle.addEventListener("pointerdown", (event) => startDrag(event, widget, direction));
      element.appendChild(handle);
    }
  }

  element.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    if (state.locked) {
      // режим пульта: виджет исполняет свои команды
      event.preventDefault();
      event.stopPropagation();
      pressWidget(widget.index, container);
      return;
    }
    if (nested) {
      event.stopPropagation();
      return;
    }
    select(widget.index);
    if (!widget.locked) startDrag(event, widget, "move");
  });

  if (!state.locked && !nested) {
    element.addEventListener("dblclick", () => {
      select(widget.index);
      openProperties();
    });
    element.addEventListener("dblclick", (event) => {
    if (state.document?.locked) return;
    if (state.externals[widget.index]) {
      event.stopPropagation();
      showRows(widget.index);
    }
  });

  element.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      event.stopPropagation();
      select(widget.index);
      widgetMenu(event, widget);
    });
  }

  return element;
}

function captionElement(widget, locked = false) {
  const caption = document.createElement("div");
  caption.className = "caption";
  caption.style.background = hex(widget.color);
  caption.style.color = autoText(widget.color);

  const title = document.createElement("span");
  title.className = "title";
  title.textContent = widget.name || widget.label;
  caption.appendChild(title);

  const tools = document.createElement("span");
  tools.className = "tools";
  const tool = (glyph, hint, action) => {
    const button = document.createElement("button");
    button.textContent = glyph;
    button.title = hint;
    button.addEventListener("pointerdown", (event) => event.stopPropagation());
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      action();
    });
    return button;
  };
  tools.appendChild(tool("⚙", "Свойства", () => openProperties()));
  tools.appendChild(tool("⧉", t("props.duplicate"), () => duplicateWidget(widget.index)));
  tools.appendChild(tool("▣", "Скрыть шапку", () =>
    updateWidget(widget.index, { captionOn: false })));
  tools.appendChild(tool("✕", t("props.delete"), () => removeWidget(widget.index)));
  if (!locked) caption.appendChild(tools);

  if (!locked) {
    caption.addEventListener("contextmenu", (event) => {
      event.preventDefault();
      event.stopPropagation();
      select(widget.index);
      widgetMenu(event, widget);
    });
  }

  return caption;
}

function contentElement(widget) {
  const content = document.createElement("div");
  const kind = String(widget.typeName).replace("vMixControl", "").toLowerCase();
  content.className = `content kind-${kind}`;

  switch (widget.typeName) {
    case "vMixControlRegion":
      content.style.background = hex(widget.borderColor);
      content.style.color = autoText(widget.borderColor);
      content.textContent = widget.text || "";
      break;

    case "vMixControlButton":
    case "vMixControlNewButton": {
      const face = document.createElement("div");
      face.className = "button-face";
      face.style.background = hex(widget.color);
      face.style.border = `1px solid ${hex(widget.borderColor)}`;
      face.style.color = autoText(widget.color);
      face.textContent = widget.text || widget.name;
      if (widget.active) face.classList.add("pressed");
      content.appendChild(face);
      break;
    }

    case "vMixControlScore":
      content.textContent = widget.text || "0";
      break;

    case "vMixControlTimer":
      content.textContent = widget.text || "00:00";
      break;

    case "vMixControlClock": {
      // часы идут локально, без обращений к vMix
      const clock = document.createElement("div");
      clock.className = "clock";
      clock.dataset.clock = "1";
      clock.dataset.useUtc = String(widget.extras?.UseUTC ?? "") === "true" ? "1" : "";
      clock.dataset.seconds = String(widget.extras?.ShowSeconds ?? "true") !== "false" ? "1" : "";
      clock.dataset.date = String(widget.extras?.ShowDate ?? "") === "true" ? "1" : "";
      updateClocksIn(clock);
      content.appendChild(clock);
      break;
    }

    case "vMixControlVolume":
    case "vMixControlSlider": {
      const target = widget.extras?.Target || "Input";
      const key = widget.extras?.InputKey || widget.inputKey || "";
      const input = (state.vmix?.inputs || []).find((item) => item.key === key);
      // состояние vMix отдаёт громкость в своей шкале — обратная кривая к 100*(v/100)^0.25
      const current = input?.volume != null ? 100 * (input.volume / 100) ** 4 : 0;
      const muted = Boolean(input?.muted);

      const box = document.createElement("div");
      box.className = "volume";

      const bar = document.createElement("div");
      bar.className = `volume-bar${muted ? " muted" : ""}`;
      const fill = document.createElement("div");
      fill.className = "volume-fill";
      fill.style.height = `${Math.max(0, Math.min(100, current))}%`;
      bar.appendChild(fill);

      const drag = (event) => {
        const rect = bar.getBoundingClientRect();
        const level = Math.max(0, Math.min(1, 1 - (event.clientY - rect.top) / rect.height));
        const value = Math.round(100 * level ** 0.25);
        const fn = target === "Input" ? "SetVolume" : volumeFunction(target);
        const params = [["Value", String(value)]];
        if (target === "Input") params.push(["Input", key]);
        invoke("vmix_call", { connection: state.connection, function: fn, params }).catch((error) =>
          toast(String(error), "error")
        );
        fill.style.height = `${level * 100}%`;
      };
      bar.addEventListener("pointerdown", (event) => {
        if (state.document?.locked) return;
        bar.setPointerCapture(event.pointerId);
        drag(event);
      });
      bar.addEventListener("pointermove", (event) => {
        if (bar.hasPointerCapture?.(event.pointerId)) drag(event);
      });

      const mute = document.createElement("button");
      mute.className = `volume-mute${muted ? " on" : ""}`;
      mute.textContent = muted ? t("volume.muted") : t("volume.mute");
      mute.addEventListener("pointerdown", (event) => {
        event.stopPropagation();
        if (state.document?.locked) return;
        const params = target === "Input" ? [["Input", key]] : [];
        invoke("vmix_call", {
          connection: state.connection,
          function: muted ? "AudioOn" : "AudioOff",
          params,
        }).catch((error) => toast(String(error), "error"));
      });

      const value = document.createElement("span");
      value.className = "volume-value";
      value.textContent = `${Math.round(current)}%`;

      box.append(bar, mute, value);
      content.appendChild(box);
      break;
    }

    case "vMixControlTBar": {
      const box = document.createElement("div");
      box.className = "tbar";
      const rail = document.createElement("div");
      rail.className = "tbar-rail";
      const knob = document.createElement("div");
      knob.className = "tbar-knob";
      knob.style.left = `${((widget.tbarValue ?? widget.text ?? 0) / 255) * 100}%`;
      rail.appendChild(knob);

      const send = (event) => {
        const rect = rail.getBoundingClientRect();
        const level = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
        const value = Math.round(level * 255);
        knob.style.left = `${(value / 255) * 100}%`;
        // оригинал отправляет готовую строку `Function=SetFader&Value=<0..255>`
        invoke("vmix_call", {
          connection: state.connection,
          function: "SetFader",
          params: [["Value", String(value)]],
        }).catch((error) => toast(String(error), "error"));
      };
      rail.addEventListener("pointerdown", (event) => {
        if (state.document?.locked) return;
        rail.setPointerCapture(event.pointerId);
        send(event);
      });
      rail.addEventListener("pointermove", (event) => {
        if (rail.hasPointerCapture?.(event.pointerId)) send(event);
      });

      box.appendChild(rail);
      content.appendChild(box);
      break;
    }

    case "vMixControlVariableViewer": {
      const name = widget.extras?.Variable || "";
      const entry = (state.doc?.globals || []).find(([key]) => key === name);
      const value = entry ? entry[1] : widget.text || "";
      const showName = String(widget.extras?.ShowVariableName ?? "") === "true";

      const box = document.createElement("div");
      box.className = "variable";
      if (showName) {
        const caption = document.createElement("span");
        caption.className = "variable-name";
        caption.textContent = name || "переменная";
        box.appendChild(caption);
      }
      const text = document.createElement("span");
      text.className = "variable-value";
      text.textContent = value || "—";
      box.appendChild(text);
      content.appendChild(box);
      break;
    }

    case "vMixControlMidiInterface": {
      const box = document.createElement("div");
      box.className = "midi-face";
      const device = document.createElement("span");
      device.className = "midi-device";
      device.textContent =
        state.midi?.device || state.midiError || widget.extras?.MidiDeviceName || t("state.midiOff");
      const count = document.createElement("span");
      count.className = "midi-count";
      count.textContent = t("midi.mappings", (widget.midiMap || []).length);
      box.append(device, count);
      content.appendChild(box);
      break;
    }

    case "vMixControlStreamDeck": {
      const box = document.createElement("div");
      box.className = "midi-face";
      const caption = document.createElement("span");
      caption.className = "midi-device";
      caption.textContent = state.deck?.device || t("state.deckOff");
      const hint = document.createElement("span");
      hint.className = "midi-count";
      hint.textContent = t("deck.keys", (widget.deckKeys || []).length);
      box.append(caption, hint);
      content.appendChild(box);
      break;
    }

    case "vMixControlContainer": {
      // контейнер держит виджеты, импортированные из другого .vmc
      const box = document.createElement("div");
      box.className = "container-body";
      try {
        for (const child of widget.children || []) {
          box.appendChild(widgetElement(child, { nested: true, container: widget.index }));
        }
      } catch (error) {
        const failure = document.createElement("span");
        failure.className = "placeholder";
        failure.textContent = `ошибка вложенного виджета: ${error}`;
        box.appendChild(failure);
      }
      if (!(widget.children || []).length) {
        const hint = document.createElement("span");
        hint.className = "placeholder";
        hint.textContent = t("container.empty");
        box.appendChild(hint);
      }
      content.appendChild(box);
      break;
    }

    case "vMixControlExternalData":
    case "vMixControlList":
    case "vMixControlPlaylist": {
      const external = state.externals[widget.index];
      if (external && external.stream) {
        content.classList.add("external");
        content.appendChild(ndiView(widget.index, external));
        break;
      }
      if (external) {
        content.classList.add("external");
        if (external.error) {
          const error = document.createElement("div");
          error.className = "row error";
          error.textContent = `${providerLabel(external.provider)}: ${external.error}`;
          content.appendChild(error);
          break;
        }
        if (!external.values.length) {
          const placeholder = document.createElement("span");
          placeholder.className = "placeholder";
          placeholder.textContent = `${providerLabel(external.provider)}: —`;
          content.appendChild(placeholder);
          break;
        }
        external.values.slice(0, 60).forEach((value, position) => {
          const row = document.createElement("div");
          row.className = `row${value === widget.text ? " active" : ""}`;
          row.textContent = value;
          row.addEventListener("pointerdown", (event) => {
            event.stopPropagation();
            externalApply(widget.index, position);
          });
          content.appendChild(row);
        });
        break;
      }
      const listExternal = widget.external;
      if (listExternal && listExternal.sourceName) {
        const placeholder = document.createElement("span");
        placeholder.className = "placeholder";
        placeholder.textContent = t("list.source", listExternal.sourceName);
        content.appendChild(placeholder);
        break;
      }
      // строки списка из <Items> (как в оригинале), иначе — текущее значение
      const source = (widget.items && widget.items.length)
        ? widget.items
        : String(widget.text || "").split("\n").filter(Boolean);
      const lines = source.slice(0, 12);
      if (!lines.length) {
        const placeholder = document.createElement("span");
        placeholder.className = "placeholder";
        placeholder.textContent = t("list.empty");
        content.appendChild(placeholder);
      }
      for (const [position, line] of lines.entries()) {
        const row = document.createElement("div");
        const current = String(widget.text || "").trim();
        row.className = `row${(current && line.trim() === current) || (!current && position === 0) ? " active" : ""}`;
        row.textContent = line;
        content.appendChild(row);
      }
      break;
    }

    default:
      content.textContent = widget.text || widget.label;
      break;
  }

  return content;
}

function select(index) {
  state.selected = index;
  for (const element of canvas.querySelectorAll(".widget")) {
    element.classList.toggle("selected", Number(element.dataset.index) === index);
  }
  renderProperties();
}

// ------------------------------------------------------------------ перетаскивание

function startDrag(event, widget, mode) {
  if (event.button !== 0) return;
  event.preventDefault();
  event.stopPropagation();

  const element = canvas.querySelector(`.widget[data-index="${widget.index}"]`);
  const start = {
    x: event.clientX,
    y: event.clientY,
    left: widget.left,
    top: widget.top,
    width: widget.width,
    height: widget.height,
  };

  const move = (moveEvent) => {
    const dx = (moveEvent.clientX - start.x) / state.view.zoom;
    const dy = (moveEvent.clientY - start.y) / state.view.zoom;

    if (mode === "move") {
      widget.left = Math.round(start.left + dx);
      widget.top = Math.round(start.top + dy);
      if (element) {
        element.style.left = `${widget.left}px`;
        element.style.top = `${widget.top}px`;
      }
    } else {
      if (mode.includes("e")) widget.width = Math.max(24, Math.round(start.width + dx));
      if (mode.includes("s")) widget.height = Math.max(24, Math.round(start.height + dy));
      if (element) {
        element.style.width = `${widget.width}px`;
        element.style.height = `${widget.height}px`;
      }
    }
  };

  const finish = () => {
    document.removeEventListener("pointermove", move);
    document.removeEventListener("pointerup", finish);
    updateWidget(
      widget.index,
      mode === "move"
        ? { left: widget.left, top: widget.top }
        : { width: widget.width, height: widget.height }
    );
  };

  document.addEventListener("pointermove", move);
  document.addEventListener("pointerup", finish);
}

// ------------------------------------------------------------------ меню

function hideMenu() {
  $("menu").classList.add("hidden");
}


/// Поиск функции по каталогу: свой выпадающий список вместо `<datalist>`.
///
/// Почему свой: в WKWebView (macOS) подсказки `<datalist>` практически не работают —
/// по 800 функциям искать в них невозможно. Здесь список фильтруется по имени и
/// описанию, поддерживает ↑/↓/Enter и не перерисовывает строки скрипта на каждый
/// символ (иначе терялись бы каретка и выделение).
function functionPicker(initial, onPick) {
  const box = document.createElement("div");
  box.className = "picker";

  const input = document.createElement("input");
  input.type = "text";
  input.className = "picker-input";
  input.value = initial ?? "";
  input.placeholder = t("script.pickFunction");
  input.autocomplete = "off";
  input.spellcheck = false;

  const list = document.createElement("div");
  list.className = "picker-list hidden";
  box.append(input, list);

  let matches = [];
  let active = 0;

  /// Поставить список под полем ввода (fixed — чтобы не обрезался панелью).
  const place = () => {
    const rect = input.getBoundingClientRect();
    const height = Math.min(list.scrollHeight || 280, 280);
    const below = window.innerHeight - rect.bottom;
    const top = below < height + 8 && rect.top > below
      ? Math.max(8, rect.top - height - 4)
      : rect.bottom + 4;
    list.style.left = `${Math.max(8, rect.left)}px`;
    list.style.top = `${top}px`;
    list.style.width = `${Math.max(200, rect.width)}px`;
  };

  const paint = () => {
    list.replaceChildren();
    if (!matches.length) {
      const empty = document.createElement("div");
      empty.className = "picker-empty";
      empty.textContent = t("picker.noMatches");
      list.appendChild(empty);
      list.classList.remove("hidden");
      return;
    }
    place();
    matches.forEach((info, position) => {
      const option = document.createElement("button");
      option.type = "button";
      option.className = `picker-item${position === active ? " active" : ""}`;
      const name = document.createElement("span");
      name.className = "picker-name";
      name.textContent = info.function;
      const hint = document.createElement("span");
      hint.className = "picker-hint";
      hint.textContent = info.description || info.category || "";
      option.append(name, hint);
      // pointerdown + preventDefault: фокус остаётся в поле ввода
      option.addEventListener("pointerdown", (event) => {
        event.preventDefault();
        pick(info.function);
      });
      list.appendChild(option);
    });
    list.classList.remove("hidden");
  };

  /// Что искать: если поле содержит имя выбранной ранее функции и пользователь
  /// дописал символы («Condition» + «Cu»), ищем по дописанной части, а не по всей строке.
  const needleFor = (value) => {
    const text = String(value ?? "").trim().toLowerCase();
    if (!text) return "";
    const known = (state.functions || []).find(
      (info) => text.startsWith(info.function.toLowerCase()) && text.length > info.function.length
    );
    return known ? text.slice(known.function.length).trim() : text;
  };

  const search = (query) => {
    const raw = String(query ?? "").trim().toLowerCase();
    const needle = needleFor(query);
    const all = state.functions || [];
    if (!needle) {
      matches = all.slice(0, 80);
      active = 0;
      paint();
      return;
    }
    // совпадения по имени важнее совпадений по описанию, точные — выше остальных
    const rank = (info) => {
      const name = info.function.toLowerCase();
      if (name === needle) return 0;
      if (name.startsWith(needle)) return 1;
      if (name.includes(needle)) return 2;
      if ((info.description || "").toLowerCase().includes(needle)) return 3;
      return 9;
    };
    matches = all
      .map((info) => ({ info, rank: rank(info) }))
      .filter((entry) => entry.rank < 9)
      .sort((left, right) => left.rank - right.rank || left.info.function.localeCompare(right.info.function))
      .map((entry) => entry.info)
      .slice(0, 80);
    active = 0;
    paint();
    void raw;
  };

  const pick = (name) => {
    input.value = name;
    matches = [];
    list.classList.add("hidden");
    onPick(name);
  };

  input.addEventListener("focus", () => {
    // выделяем содержимое: первый же символ заменяет прежнее имя функции
    input.select();
    search(input.value === (initial ?? "") ? "" : input.value);
  });
  input.addEventListener("input", () => search(input.value));
  input.addEventListener("blur", () => {
    // даём сработать pointerdown по элементу списка
    setTimeout(() => list.classList.add("hidden"), 150);
  });
  // панель и окно могут двигаться/скроллиться — держим список под полем
  window.addEventListener("resize", () => {
    if (!list.classList.contains("hidden")) place();
  });
  window.addEventListener("scroll", (event) => {
    if (list.classList.contains("hidden")) return;
    if (event.target instanceof Node && list.contains(event.target)) return;
    place();
  }, true);
  input.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      active = Math.min(active + 1, Math.max(matches.length - 1, 0));
      paint();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      active = Math.max(active - 1, 0);
      paint();
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (matches[active]) pick(matches[active].function);
      else if (input.value.trim()) pick(input.value.trim());
    } else if (event.key === "Escape") {
      list.classList.add("hidden");
    }
  });

  box.focus = () => input.focus();
  box.value = () => input.value.trim();
  return box;
}

function showMenu(event, items) {
  const menu = $("menu");
  menu.replaceChildren();
  for (const item of items) {
    if (item.separator) {
      const line = document.createElement("div");
      line.className = "sep";
      menu.appendChild(line);
      continue;
    }
    const button = document.createElement("button");
    if (item.color) {
      const swatch = document.createElement("span");
      swatch.className = "swatch";
      swatch.style.background = item.color;
      button.appendChild(swatch);
    }
    const label = document.createElement("span");
    // ключ локализации (список виджетов) либо готовый текст (команды меню)
    label.textContent = item.kindKey ? t(item.kindKey) : item.label ?? "";
    button.appendChild(label);
    button.addEventListener("click", () => {
      hideMenu();
      item.action();
    });
    menu.appendChild(button);
  }
  menu.style.left = `${event.clientX}px`;
  menu.style.top = `${event.clientY}px`;
  menu.classList.remove("hidden");
}

function widgetMenu(event, widget) {
  showMenu(event, [
    { label: t("menu.properties"), action: () => openProperties() },
    { label: t("props.duplicate"), action: () => duplicateWidget(widget.index) },
    { label: widget.captionOn ? t("menu.hideCaption") : t("menu.showCaption"),
      action: () => updateWidget(widget.index, { captionOn: !widget.captionOn }) },
    { label: widget.locked ? t("menu.unlock") : t("menu.lock"),
      action: () => updateWidget(widget.index, { locked: !widget.locked }) },
    { separator: true },
    { label: "Выше", action: () => updateWidget(widget.index, { zIndex: widget.zIndex + 1 }) },
    { label: "Ниже", action: () => updateWidget(widget.index, { zIndex: widget.zIndex - 1 }) },
    { label: `На страницу… ${state.doc.pages[(widget.page + 1) % state.doc.pages.length]}`,
      action: () => updateWidget(widget.index, { page: (widget.page + 1) % state.doc.pages.length }) },
    { separator: true },
    { label: t("props.delete"), action: () => removeWidget(widget.index) },
  ]);
}

function surfaceMenu(event) {
  const items = [
    { label: t("menu.documentNew"), action: newDocument },
    { label: t("menu.documentOpen"), action: openDocument },
    { label: t("menu.documentSave"), action: saveDocument },
    { separator: true },
  ];
  for (const item of state.palette) {
    items.push({
      label: `${t("menu.addWidget")}: ${t(item.kindKey || "kind.other")}`,
      color: item.color,
      action: () => {
        const point = canvasPoint(event);
        addWidget(item.typeName, Math.round(point.x), Math.round(point.y));
      },
    });
  }
  showMenu(event, items);
}

function canvasPoint(event) {
  const rect = canvas.getBoundingClientRect();
  return {
    x: (event.clientX - rect.left) / state.view.zoom,
    y: (event.clientY - rect.top) / state.view.zoom,
  };
}

// ------------------------------------------------------------------ свойства

function openProperties() {
  const widget = byIndex(state.selected);
  if (!widget) return toast("сначала выберите виджет", "error");
  const panel = $("props");
  panel.classList.remove("hidden");
  panel.replaceChildren();

  const title = document.createElement("h3");
  title.textContent = `${widget.label} · #${widget.index}`;
  panel.appendChild(title);

  const field = (label, input) => {
    const wrapper = document.createElement("label");
    const text = document.createElement("span");
    text.textContent = label;
    wrapper.append(text, input);
    panel.appendChild(wrapper);
    return input;
  };

  const name = field(t("props.name"), Object.assign(document.createElement("input"), { type: "text", value: widget.name }));
  const text = field(t("props.text"), Object.assign(document.createElement("input"), { type: "text", value: widget.text }));
  const color = field(t("props.color"), Object.assign(document.createElement("input"), { type: "color", value: hex(widget.color) }));
  const border = field(t("props.border"), Object.assign(document.createElement("input"), { type: "color", value: hex(widget.borderColor) }));
  const zIndex = field(t("props.zIndex"), Object.assign(document.createElement("input"), { type: "number", value: widget.zIndex }));

  const page = document.createElement("select");
  state.doc.pages.forEach((pageName, index) => {
    const option = document.createElement("option");
    option.value = String(index);
    option.textContent = pageName;
    option.selected = index === widget.page;
    page.appendChild(option);
  });
  field(t("props.page"), page);

  const locked = field(t("props.locked"), Object.assign(document.createElement("input"), { type: "checkbox", checked: widget.locked }));
  const caption = field(t("props.captionVisible"), Object.assign(document.createElement("input"), { type: "checkbox", checked: widget.captionVisible }));

  const apply = () => {
    updateWidget(widget.index, {
      name: name.value,
      text: text.value,
      color: rgba(color.value),
      borderColor: rgba(border.value),
      zIndex: Number(zIndex.value) || 0,
      page: Number(page.value) || 0,
      locked: locked.checked,
      captionVisible: caption.checked,
    });
  };
  for (const input of [name, text, zIndex]) input.addEventListener("change", apply);
  for (const input of [color, border]) input.addEventListener("change", apply);
  for (const input of [page, locked, caption]) input.addEventListener("change", apply);

  const actions = document.createElement("div");
  actions.className = "row";
  const duplicate = document.createElement("button");
  duplicate.textContent = t("props.duplicate");
  duplicate.addEventListener("click", () => duplicateWidget(widget.index));
  const remove = document.createElement("button");
  remove.textContent = t("props.delete");
  remove.addEventListener("click", () => {
    closeProperties();
    removeWidget(widget.index);
  });
  const close = document.createElement("button");
  close.textContent = t("props.close");
  close.addEventListener("click", closeProperties);
  actions.append(duplicate, remove, close);
  panel.appendChild(actions);

  appendTypedEditor(panel, widget);
  appendListItemsEditor(panel, widget);
  appendScheduleEditor(panel, widget);
  appendHotkeyEditor(panel, widget);
  appendDeckEditor(panel, widget);
  appendMidiEditor(panel, widget);
  appendContainerEditor(panel, widget);
  appendExternalEditor(panel, widget);
  appendCommandsEditor(panel, widget);
}

/// Ссылки горячих клавиш виджета: их слушают MIDI, Stream Deck и скрипты (`ProcessHotkey`).
function appendHotkeyEditor(panel, widget) {
  const hotkeys = widget.hotkeys || [];
  if (!hotkeys.length) return;

  const box = document.createElement("div");
  box.className = "commands";
  const title = document.createElement("h3");
  title.textContent = t("hotkeys.title", hotkeys.length);
  box.appendChild(title);

  hotkeys.forEach((hotkey, position) => {
    const row = document.createElement("div");
    row.className = "command";

    const name = document.createElement("span");
    name.className = "fn";
    name.textContent = hotkey.name || `#${position}`;

    const link = document.createElement("input");
    link.type = "text";
    link.placeholder = t("hotkeys.link");
    link.value = hotkey.link || "";
    link.addEventListener("change", () => {
      invoke("vmc_set_hotkey", {
        index: widget.index,
        position,
        link: link.value.trim(),
        active: Boolean(link.value.trim()),
      })
        .then((doc) => {
          state.doc = doc;
          render();
        })
        .catch((error) => toast(String(error), "error"));
    });

    const press = document.createElement("button");
    press.textContent = "▶";
    press.title = t("hotkeys.run");
    press.addEventListener("click", async () => {
      if (!link.value.trim()) return toast("ссылка пуста", "error");
      try {
        const result = await invoke("vmix_dispatch_link", {
          link: link.value.trim(),
          value: null,
          connection: state.connection,
        });
        toast(result.log.join(" · ") || "выполнено");
      } catch (error) {
        toast(String(error), "error");
      }
    });

    row.append(name, link, press);
    box.appendChild(row);
  });

  panel.appendChild(box);
}

/// Описание полей по типам виджетов — вместо отдельных XAML-редакторов оригинала.
const KIND_FIELDS = {
  vMixControlButton: [
    { tag: "Style", label: "field.buttonStyle", options: ["Momentary", "Toggle"] },
    { tag: "AutoStart", label: "field.autoStart", bool: true },
    { tag: "IsStateDependent", label: "field.stateDependent", bool: true },
    { tag: "IsColorized", label: "field.colorized", bool: true },
  ],
  vMixControlNewButton: [
    { tag: "Style", label: "field.buttonStyle", options: ["Momentary", "Toggle"] },
    { tag: "AutoStart", label: "field.autoStart", bool: true },
  ],
  vMixControlTextField: [
    { tag: "IsMappedToGUID", label: "field.mappedToGuid", bool: true },
  ],
  vMixControlVolume: [
    { tag: "Target", label: "field.target", options: ["Input", "Master", "Headphones", "Bus A", "Bus B", "Bus C", "Bus D", "Bus E", "Bus F", "Bus G"] },
    { tag: "InputKey", label: "field.input", input: true },
    { tag: "Style", label: "field.orientation", options: ["Vertical", "Horizontal"] },
    { tag: "ShowSlider", label: "field.showSlider", bool: true },
    { tag: "ShowMeters", label: "field.showMeters", bool: true },
  ],
  vMixControlSlider: [
    { tag: "Target", label: "field.target", options: ["Input", "Master", "Headphones", "Bus A", "Bus B", "Bus C", "Bus D", "Bus E", "Bus F", "Bus G"] },
    { tag: "InputKey", label: "field.input", input: true },
    { tag: "ShowSlider", label: "field.showSlider", bool: true },
  ],
  vMixControlTBar: [
    { tag: "Mode", label: "field.mode", options: ["Fader", "Transition", "Auto"] },
    { tag: "Style", label: "field.orientation", options: ["Horizontal", "Vertical"] },
  ],
  vMixControlClock: [
    { tag: "ShowSeconds", label: "field.showSeconds", bool: true },
    { tag: "ShowDate", label: "field.showDate", bool: true },
    { tag: "UseUTC", label: "field.useUtc", bool: true },
  ],
  vMixControlVariableViewer: [
    { tag: "Variable", label: "field.variable", variables: true },
    { tag: "ShowVariableName", label: "field.showVariableName", bool: true },
  ],
  vMixControlList: [
    { tag: "IsTable", label: "field.isTable", bool: true },
    { tag: "IsMappedToGUID", label: "field.mappedToGuid", bool: true },
    { tag: "RestartData", label: "field.restartData", bool: true },
    { tag: "Enabled", label: "field.enabled", bool: true },
  ],
  vMixControlExternalData: [
    { tag: "IsTable", label: "field.isTable", bool: true },
    { tag: "IsMappedToGUID", label: "field.mappedToGuid", bool: true },
    { tag: "RestartData", label: "field.restartData", bool: true },
    { tag: "Enabled", label: "field.enabled", bool: true },
  ],
};

/// Редакторы по типам виджетов (аналог `PropertyEditorTemplates.xaml`).
function appendTypedEditor(panel, widget) {
  const fields = KIND_FIELDS[widget.typeName] || [];
  if (!fields.length) return;

  const box = document.createElement("div");
  box.className = "commands";
  const title = document.createElement("h3");
  title.textContent = t("props.typed");
  box.appendChild(title);

  const save = (tag, value) =>
    invoke("vmc_set_extra", { index: widget.index, tag, value })
      .then((doc) => {
        state.doc = doc;
      })
      .catch((error) => toast(String(error), "error"));

  for (const field of fields) {
    const row = document.createElement("div");
    row.className = "command";
    const label = document.createElement("span");
    label.className = "fn";
    label.textContent = t(field.label);
    row.appendChild(label);

    const current = widget.extras?.[field.tag] ?? "";

    if (field.bool) {
      const input = document.createElement("input");
      input.type = "checkbox";
      input.checked = String(current) === "true" || String(current) === "1";
      input.addEventListener("change", () => save(field.tag, input.checked ? "true" : "false"));
      row.appendChild(input);
    } else if (field.variables) {
      const select = document.createElement("select");
      const empty = document.createElement("option");
      empty.value = "";
      empty.textContent = "—";
      select.appendChild(empty);
      for (const [name] of state.doc?.globals || []) {
        const option = document.createElement("option");
        option.value = name;
        option.textContent = name;
        option.selected = name === current;
        select.appendChild(option);
      }
      select.addEventListener("change", () => save(field.tag, select.value));
      row.appendChild(select);
    } else if (field.input) {
      const select = document.createElement("select");
      const empty = document.createElement("option");
      empty.value = "";
      empty.textContent = "—";
      select.appendChild(empty);
      for (const input of state.vmix?.inputs || []) {
        const option = document.createElement("option");
        option.value = input.key || String(input.number);
        option.textContent = `#${input.number ?? "?"} ${input.title ?? ""}`.trim();
        option.selected = option.value === current;
        select.appendChild(option);
      }
      select.addEventListener("change", () => save(field.tag, select.value));
      row.appendChild(select);
    } else if (field.options) {
      const select = document.createElement("select");
      for (const value of field.options) {
        const option = document.createElement("option");
        option.value = value;
        option.textContent = value;
        option.selected = value === current;
        select.appendChild(option);
      }
      select.addEventListener("change", () => save(field.tag, select.value));
      row.appendChild(select);
    } else {
      const input = document.createElement("input");
      input.type = "text";
      input.value = current;
      input.addEventListener("change", () => save(field.tag, input.value));
      row.appendChild(input);
    }

    box.appendChild(row);
  }

  panel.appendChild(box);
}

/// Расписание часов: время, дни недели, ссылка (порт `SchedulerControl`).
function appendScheduleEditor(panel, widget) {
  if (String(widget.typeName) !== "vMixControlClock") return;

  const box = document.createElement("div");
  box.className = "commands";
  const title = document.createElement("h3");
  title.textContent = t("schedule.title", (widget.events || []).length);
  box.appendChild(title);

  const draft = (widget.events || []).map((event) => ({ ...event }));

  const save = () =>
    invoke("vmc_set_events", { index: widget.index, events: draft })
      .then((doc) => {
        state.doc = doc;
        render();
      })
      .catch((error) => toast(String(error), "error"));

  const DAYS = [
    ["пн", 1],
    ["вт", 2],
    ["ср", 4],
    ["чт", 8],
    ["пт", 16],
    ["сб", 32],
    ["вс", 64],
  ];

  draft.forEach((event, position) => {
    const row = document.createElement("div");
    row.className = "command";

    const time = document.createElement("input");
    time.type = "time";
    time.value = event.time || "09:00";
    time.addEventListener("change", () => {
      event.time = time.value;
      save();
    });

    const command = document.createElement("input");
    command.type = "text";
    command.placeholder = t("schedule.linkPlaceholder");
    command.value = event.command || "";
    command.addEventListener("change", () => {
      event.command = command.value.trim();
      save();
    });

    const remove = document.createElement("button");
    remove.textContent = "✕";
    remove.addEventListener("click", () => {
      draft.splice(position, 1);
      save();
    });

    const fire = document.createElement("button");
    fire.textContent = "▶";
    fire.title = t("schedule.run");
    fire.addEventListener("click", async () => {
      if (!event.command) return toast("ссылка пуста", "error");
      const result = await invoke("vmix_dispatch_link", {
        link: event.command,
        value: null,
        connection: state.connection,
      });
      toast(result.log.join(" · ") || "выполнено");
    });

    row.append(time, command, fire, remove);
    box.appendChild(row);

    const days = document.createElement("div");
    days.className = "command";
    for (const [name, bit] of DAYS) {
      const toggle = document.createElement("label");
      toggle.className = "day";
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = Boolean((event.days || 0) & bit);
      checkbox.addEventListener("change", () => {
        event.days = checkbox.checked ? (event.days || 0) | bit : (event.days || 0) & ~bit;
        save();
      });
      toggle.append(checkbox, document.createTextNode(name));
      days.appendChild(toggle);
    }
    box.appendChild(days);
  });

  const add = document.createElement("button");
  add.textContent = t("schedule.add");
  add.addEventListener("click", () => {
    draft.push({ time: "09:00", command: "", days: 127 });
    save();
  });
  box.appendChild(add);

  panel.appendChild(box);
}

/// Stream Deck: устройство, запуск, яркость, привязки кнопок и обучение.
function appendDeckEditor(panel, widget) {
  if (String(widget.typeName) !== "vMixControlStreamDeck") return;

  const box = document.createElement("div");
  box.className = "commands";
  const title = document.createElement("h3");
  title.textContent = `${t("deck.title")} · ${(widget.deckKeys || []).length}`;
  box.appendChild(title);

  const status = document.createElement("div");
  status.className = "hint";
  status.textContent = state.deck?.device || "устройство не подключено";
  box.appendChild(status);

  const devices = document.createElement("select");
  const empty = document.createElement("option");
  empty.value = "";
  empty.textContent = t("deck.device");
  devices.appendChild(empty);
  for (const device of state.deckDevices || []) {
    const option = document.createElement("option");
    option.value = device;
    option.textContent = device;
    devices.appendChild(option);
  }

  const start = document.createElement("button");
  start.textContent = t("deck.connect");
  start.addEventListener("click", async () => {
    try {
      const device = await invoke("streamdeck_start", { device: devices.value || null });
      state.deck = { device };
      renderCanvas();
      toast(device);
    } catch (error) {
      toast(String(error), "error");
    }
  });

  const stop = document.createElement("button");
  stop.textContent = "Стоп";
  stop.addEventListener("click", async () => {
    await invoke("streamdeck_stop");
    state.deck = { device: "" };
    renderCanvas();
    toast("Stream Deck отключён");
  });

  const find = document.createElement("button");
  find.textContent = t("deck.find");
  find.addEventListener("click", async () => {
    state.deckDevices = await invoke("streamdeck_devices");
    toast(`устройств: ${state.deckDevices.length}`);
  });

  box.append(devices, start, stop, find);

  const brightness = document.createElement("input");
  brightness.type = "range";
  brightness.min = "0";
  brightness.max = "100";
  brightness.value = String(state.deck?.brightness ?? 100);
  brightness.addEventListener("change", async () => {
    await invoke("streamdeck_brightness", { percent: Number(brightness.value) });
    state.deck = { ...(state.deck || {}), brightness: Number(brightness.value) };
  });
  box.append(brightness);

  const link = document.createElement("input");
  link.type = "text";
  link.placeholder = t("midi.learnPlaceholder");
  const learn = document.createElement("button");
  learn.textContent = t("midi.learn");
  learn.addEventListener("click", async () => {
    if (!link.value.trim()) return toast("укажите ссылку", "error");
    try {
      state.doc = await invoke("streamdeck_learn", {
        index: widget.index,
        link: link.value.trim(),
      });
      render();
      toast(`кнопка привязана к «${link.value.trim()}»`);
    } catch (error) {
      toast(String(error), "error");
    }
  });
  box.append(link, learn);

  for (const key of widget.deckKeys || []) {
    const row = document.createElement("div");
    row.className = "command";
    const text = document.createElement("span");
    text.className = "fn";
    text.textContent = t("deck.button", key.index, key.link);
    row.appendChild(text);
    box.appendChild(row);
  }

  panel.appendChild(box);
}

/// MIDI: устройство, запуск/остановка, отображения и обучение.
function appendMidiEditor(panel, widget) {
  if (String(widget.typeName) !== "vMixControlMidiInterface") return;

  const box = document.createElement("div");
  box.className = "commands";
  const title = document.createElement("h3");
  title.textContent = `${t("midi.title")} · ${(widget.midiMap || []).length}`;
  box.appendChild(title);

  const status = document.createElement("div");
  status.className = "hint";
  status.textContent = state.midi?.device || "вход не открыт";
  box.appendChild(status);

  const ports = document.createElement("select");
  const empty = document.createElement("option");
  empty.value = "";
  empty.textContent = t("midi.device");
  ports.appendChild(empty);
  for (const port of state.midiPorts || []) {
    const option = document.createElement("option");
    option.value = port;
    option.textContent = port;
    ports.appendChild(option);
  }

  const start = document.createElement("button");
  start.textContent = t("midi.listen");
  start.addEventListener("click", async () => {
    try {
      const device = await invoke("midi_start", { device: ports.value || null });
      state.midi = { device };
      renderCanvas();
      toast(device);
      refreshVmix();
    } catch (error) {
      toast(String(error), "error");
    }
  });

  const stop = document.createElement("button");
  stop.textContent = "Стоп";
  stop.addEventListener("click", async () => {
    await invoke("midi_stop");
    state.midi = { device: "" };
    renderCanvas();
    toast("MIDI остановлен");
  });

  const refresh = document.createElement("button");
  refresh.textContent = t("midi.refresh");
  refresh.addEventListener("click", async () => {
    state.midiPorts = await invoke("midi_ports");
    toast(`входов: ${state.midiPorts.length}`);
  });

  box.append(ports, start, stop, refresh);

  const link = document.createElement("input");
  link.type = "text";
  link.placeholder = "ссылка для обучения (например Play.Execute)";
  const learn = document.createElement("button");
  learn.textContent = t("deck.learn");
  learn.addEventListener("click", async () => {
    if (!link.value.trim()) return toast("укажите ссылку", "error");
    try {
      state.doc = await invoke("midi_learn", { index: widget.index, link: link.value.trim() });
      render();
      toast(`сообщение привязано к «${link.value.trim()}»`);
    } catch (error) {
      toast(String(error), "error");
    }
  });
  box.append(link, learn);

  for (const mapping of widget.midiMap || []) {
    const row = document.createElement("div");
    row.className = "command";
    const text = document.createElement("span");
    text.className = "fn";
    text.textContent = `${mapping.kind} ch${mapping.channel} #${mapping.number} → ${mapping.link}`;
    row.appendChild(text);
    box.appendChild(row);
  }

  panel.appendChild(box);
}

/// Контейнер: импорт другого `.vmc` внутрь (порт `AfterPropertiesChanged`).
function appendContainerEditor(panel, widget) {
  if (widget.kind !== "Container" && widget.typeName !== "vMixControlContainer") return;

  const box = document.createElement("div");
  box.className = "commands";
  const title = document.createElement("h3");
  title.textContent = t("container.title", (widget.children || []).length);
  box.appendChild(title);

  const path = document.createElement("input");
  path.type = "text";
  path.placeholder = t("container.importPlaceholder");
  const importButton = document.createElement("button");
  importButton.textContent = t("container.import");
  importButton.addEventListener("click", async () => {
    if (!path.value.trim()) return toast("укажите путь к .vmc", "error");
    try {
      state.doc = await invoke("vmc_container_import", {
        index: widget.index,
        path: path.value.trim(),
      });
      render();
      toast("импортировано");
    } catch (error) {
      toast(String(error), "error");
    }
  });

  box.append(path, importButton);
  panel.appendChild(box);
}

/// Настройки внешних данных: провайдер, источник, XPath, период и карта путей.
function appendExternalEditor(panel, widget) {
  const external = widget.external;

  const box = document.createElement("div");
  box.className = "commands";

  // источника ещё нет — предлагаем его создать (XML по умолчанию)
  if (!external) {
    const title = document.createElement("h3");
    title.textContent = t("external.title");
    box.appendChild(title);
    const hint = document.createElement("div");
    hint.className = "hint";
    hint.textContent = t("external.none");
    box.appendChild(hint);
    const create = document.createElement("button");
    create.textContent = t("external.add");
    create.className = "wide";
    create.addEventListener("click", () => {
      const fresh = {
        isLive: false,
        isTable: false,
        isMappedToGUID: false,
        text: "",
        enabled: true,
        restartData: false,
        periodMs: 1000,
        providerPath: "DataProviders\\XmlDataProvider.dll",
        providerProperties: ["", "", "", "1"],
        paths: [],
        sourceName: "",
        sourceData: "",
      };
      invoke("vmc_set_external", { index: widget.index, external: fresh })
        .then((doc) => { state.doc = doc; render(); openProperties(); })
        .catch((error) => toast(String(error), "error"));
    });
    box.appendChild(create);
    panel.appendChild(box);
    return;
  }
  const title = document.createElement("h3");
  title.textContent = external.providerPath
    ? `${t("external.title")} · ${external.providerPath.split(/[\\/]/).pop()}`
    : `${t("external.title")} · ${t("provider.source")} «${external.sourceName || "—"}»`;
  box.appendChild(title);

  const field = (label, input) => {
    const wrapper = document.createElement("label");
    const text = document.createElement("span");
    text.textContent = label;
    wrapper.append(text, input);
    box.appendChild(wrapper);
    return input;
  };

  const draft = { ...external, providerProperties: [...(external.providerProperties || [])] };
  const save = () => invoke("vmc_set_external", { index: widget.index, external: draft })
    .then((doc) => { state.doc = doc; render(); })
    .catch((error) => toast(String(error), "error"));

  // выбор провайдера: пишем тот же путь, что и оригинал (`DataProviders\\*.dll`)
  const providers = [
    ["DataProviders\\XmlDataProvider.dll", "provider.xml"],
    ["DataProviders\\JsonDataProvider.dll", "provider.json"],
    ["DataProviders\\ExcelDataProvider.dll", "provider.excel"],
    ["DataProviders\\GoogleSheetsProvider.dll", "provider.sheets"],
    ["DataProviders\\NDIMonitorDataProvider.dll", "provider.ndi"],
    ["DataProviders\\FileSystemDataProvider.dll", "provider.file"],
  ];
  const providerSelect = document.createElement("select");
  providerSelect.append(new Option(t("external.providerNone"), ""));
  for (const [path, key] of providers) providerSelect.append(new Option(t(key), path));
  providerSelect.value = external.providerPath || "";
  providerSelect.addEventListener("change", () => {
    draft.providerPath = providerSelect.value;
    save();
  });
  field(t("external.provider"), providerSelect);

  if (draft.providerPath) {
    const kind = String(draft.providerPath).toLowerCase();
    const property = (index, value) => {
      draft.providerProperties[index] = value;
      save();
    };
    const textInput = (index, placeholder, label) =>
      field(label, Object.assign(document.createElement("input"), {
        type: "text",
        value: draft.providerProperties[index] ?? "",
        placeholder,
        onchange: (event) => property(index, event.target.value),
      }));
    const numberInput = (index, label, fallback) =>
      field(label, Object.assign(document.createElement("input"), {
        type: "number",
        value: draft.providerProperties[index] ?? fallback,
        onchange: (event) => property(index, event.target.value),
      }));

    if (kind.includes("json")) {
      textInput(0, "http://host:port/api/scores или путь к файлу", "Источник");
      textInput(1, "$.scores[*].team", t("external.jsonPath"));
      numberInput(2, t("external.groupBy"), "1");
      field(t("external.headers"), Object.assign(document.createElement("input"), {
        type: "text",
        value: draft.providerProperties[3] ?? "",
        placeholder: "X-Api-Key: secret",
        onchange: (event) => property(3, event.target.value),
      }));
    } else if (kind.includes("sheets") || kind.includes("google")) {
      textInput(7, "https://docs.google.com/spreadsheets/d/<id>/edit", t("external.sheetKey"));
      textInput(0, "Google API key", t("external.apiKey"));
      numberInput(5, t("external.sheet"), "0");
      numberInput(1, t("external.startRow"), "0");
      numberInput(2, t("external.endRow"), "-1");
      numberInput(3, t("external.startCol"), "0");
      numberInput(4, t("external.endCol"), "-1");
      field(t("external.table"), Object.assign(document.createElement("input"), {
        type: "checkbox",
        checked: String(draft.providerProperties[6] ?? "true") !== "false",
        onchange: (event) => property(6, event.target.checked ? "true" : "false"),
      }));
    } else if (kind.includes("ndi")) {
      textInput(0, "NDI source name", t("external.ndiSource"));
      numberInput(2, t("external.ndiLayout"), "0");
      textInput(3, "16:9", t("external.ndiAspect"));
      field(t("external.ndiAudio"), Object.assign(document.createElement("input"), {
        type: "checkbox",
        checked: String(draft.providerProperties[4] ?? "true") !== "false",
        onchange: (event) => property(4, event.target.checked ? "true" : "false"),
      }));
      field(t("external.ndiLowBandwidth"), Object.assign(document.createElement("input"), {
        type: "checkbox",
        checked: String(draft.providerProperties[5] ?? "") === "true",
        onchange: (event) => property(5, event.target.checked ? "true" : "false"),
      }));
      const note = document.createElement("span");
      note.className = "hint";
      note.textContent = t("external.ndiNote");
      box.appendChild(note);
    } else if (kind.includes("excel")) {
      textInput(0, "/путь/к/книге.xlsx", t("external.file"));
      textInput(5, "Scores / 0", t("external.sheet"));
      numberInput(1, "Начальная строка", "0");
      numberInput(2, "Конечная строка (-1 = все)", "-1");
      textInput(3, "A", t("external.startCol"));
      textInput(4, "C", t("external.endCol"));
      field("Таблица (строки через |)", Object.assign(document.createElement("input"), {
        type: "checkbox",
        checked: String(draft.providerProperties[6] ?? "") === "true",
        onchange: (event) => property(6, event.target.checked ? "true" : "false"),
      }));
    } else {
      textInput(0, "http://127.0.0.1:8088/api или путь к файлу", "Источник");
      textInput(1, "./vmix/inputs/input/@number", "XPath");
      numberInput(3, t("external.groupBy"), "1");
    }

    const period = field(t("external.period"), Object.assign(document.createElement("input"), {
      type: "number",
      value: draft.periodMs ?? 1000,
    }));
    period.addEventListener("change", () => {
      draft.periodMs = Number(period.value) || 1000;
      save();
    });

    // проверка источника не дожидаясь периода опроса
    const check = document.createElement("button");
    check.textContent = t("external.refresh");
    check.className = "wide";
    const checkResult = document.createElement("div");
    checkResult.className = "hint";
    check.addEventListener("click", async () => {
      check.disabled = true;
      try {
        const rows = await invoke("vmc_refresh_external", { index: widget.index });
        state.externals[widget.index] = rows;
        const count = (rows.values || []).length;
        checkResult.textContent = `${providerLabel(rows.provider)}: ${count}${
          rows.error ? ` · ${rows.error}` : ""
        }`;
        toast(t("external.refreshed", count), rows.error ? "error" : "ok");
      } catch (error) {
        checkResult.textContent = String(error);
        toast(String(error), "error");
      }
      check.disabled = false;
    });
    box.append(check, checkResult);
  }

  if (external.sourceName) {
    const source = field("Виджет-источник", Object.assign(document.createElement("input"), {
      type: "text",
      value: draft.sourceName,
    }));
    source.addEventListener("change", () => {
      draft.sourceName = source.value;
      save();
    });
  }

  const rows = state.externals[widget.index];
  if (rows) {
    // выбор строки: применяется к vMix и запоминается в виджете (Text)
    if ((rows.values || []).length) {
      const pick = document.createElement("select");
      const none = new Option(t("external.rowNone"), "");
      pick.appendChild(none);
      rows.values.forEach((value, position) => {
        const option = new Option(value, String(position));
        if (String(value).trim() === String(widget.text || "").trim()) option.selected = true;
        pick.appendChild(option);
      });
      pick.addEventListener("change", () => {
        if (pick.value === "") return;
        externalApply(widget.index, Number(pick.value));
      });
      field(t("external.row"), pick);
    }
    const show = document.createElement("button");
    show.textContent = t("rows.show");
    show.addEventListener("click", () => showRows(widget.index));
    box.appendChild(show);
    const status = document.createElement("div");
    status.className = "hint";
    status.textContent = `${providerLabel(rows.provider)}: ${(rows.values || []).length}${
      rows.error ? ` · ${rows.error}` : ""
    }`;
    box.appendChild(status);
  }

  if ((external.paths || []).length) {
    const mapTitle = document.createElement("h3");
    mapTitle.textContent = `Карта путей (${external.paths.length})`;
    box.appendChild(mapTitle);
    for (const [key, name] of external.paths) {
      const line = document.createElement("div");
      line.className = "command";
      const text = document.createElement("span");
      text.className = "fn";
      text.textContent = `${inputTitle(key)} → ${name}`;
      line.appendChild(text);
      box.appendChild(line);
    }
  }

  panel.appendChild(box);
}

/// Строки списка (`<Items>`): добавление, правка, порядок, удаление.
function appendListItemsEditor(panel, widget) {
  if (!widget.items || !widget.items.length) return;

  const box = document.createElement("div");
  box.className = "commands";

  const title = document.createElement("h3");
  title.textContent = t("list.itemsCount", widget.items.length);
  box.appendChild(title);

  const hint = document.createElement("div");
  hint.className = "hint";
  hint.textContent = t("list.itemsHint");
  box.appendChild(hint);

  const items = [...widget.items];
  const save = () => invoke("vmc_set_list_items", { index: widget.index, items })
    .then((doc) => { state.doc = doc; render(); })
    .catch((error) => toast(String(error), "error"));

  items.forEach((value, position) => {
    const row = document.createElement("div");
    row.className = "command";
    const field = document.createElement("input");
    field.type = "text";
    field.value = value;
    field.className = "grow";
    field.addEventListener("change", () => {
      items[position] = field.value;
      save();
    });
    row.appendChild(field);
    row.append(
      smallButton("↑", () => {
        if (position === 0) return;
        [items[position - 1], items[position]] = [items[position], items[position - 1]];
        save();
      }),
      smallButton("↓", () => {
        if (position === items.length - 1) return;
        [items[position + 1], items[position]] = [items[position], items[position + 1]];
        save();
      }),
      smallButton("✕", () => {
        items.splice(position, 1);
        save();
      })
    );
    box.appendChild(row);
  });

  const add = document.createElement("button");
  add.textContent = t("list.addItem");
  add.className = "wide";
  add.addEventListener("click", () => {
    const next = String(items.length + 1);
    items.push(`${next}|`);
    save();
  });
  box.appendChild(add);
  panel.appendChild(box);
}

/// Команды кнопки: список + добавление (имя функции из каталога, вход, параметры).
function appendCommandsEditor(panel, widget) {
  if (!String(widget.typeName).includes("Button")) return;

  const box = document.createElement("div");
  box.className = "commands";

  const title = document.createElement("h3");
  title.textContent = t("props.commandsCount", widget.commands.length);
  box.appendChild(title);

  const openEditor = document.createElement("button");
  openEditor.textContent = t("props.openScript");
  openEditor.addEventListener("click", () => openScriptEditor(widget.index));
  box.appendChild(openEditor);

  widget.commands.forEach((command, position) => {
    const row = document.createElement("div");
    row.className = "command";
    const text = document.createElement("span");
    text.className = "fn";
    text.textContent = describeCommand(command);
    text.title = command.formatString || command.description || "";
    row.appendChild(text);
    row.append(
      smallButton("↑", () => moveCommand(widget, position, -1)),
      smallButton("↓", () => moveCommand(widget, position, 1)),
      smallButton("▶", () => pressWidget(widget.index)),
      smallButton("✕", () =>
        setCommands(widget, widget.commands.filter((_, index) => index !== position)))
    );
    box.appendChild(row);
  });

  const form = document.createElement("div");
  form.className = "command-add";
  const fn = functionPicker("", () => {});
  const input = document.createElement("input");
  input.type = "number";
  input.min = "0";
  input.placeholder = t("props.input");
  const parameter = document.createElement("input");
  parameter.placeholder = t("props.parameter");
  const value = document.createElement("input");
  value.placeholder = t("props.text");
  const add = document.createElement("button");
  add.textContent = t("props.addCommand");
  add.className = "wide";
  add.addEventListener("click", () => {
    const name = fn.value();
    if (!name) return toast(t("script.pickFunction"), "error");
    const command = { function: name, executable: true, useInActiveState: true };
    if (input.value) command.input = Number(input.value);
    if (parameter.value) command.parameter = parameter.value;
    if (value.value) command.stringParameter = value.value;
    setCommands(widget, [...widget.commands, command]);
  });
  form.append(fn, input, parameter, value, add);
  box.appendChild(form);

  panel.appendChild(box);
}

function describeCommand(command) {
  const parameters = [];
  if (command.inputKey) parameters.push(command.inputKey);
  else if (command.input) parameters.push(command.input);
  if (command.parameter) parameters.push(command.parameter);
  if (command.stringParameter) parameters.push(JSON.stringify(command.stringParameter));
  return `${command.function}(${parameters.join(",")})`;
}

function smallButton(label, action) {
  const button = document.createElement("button");
  button.textContent = label;
  button.addEventListener("click", action);
  return button;
}

async function setCommands(widget, commands) {
  await applyDoc(invoke("vmix_set_commands", { index: widget.index, commands }));
}

function moveCommand(widget, position, delta) {
  const target = position + delta;
  if (target < 0 || target >= widget.commands.length) return;
  const commands = widget.commands.slice();
  [commands[position], commands[target]] = [commands[target], commands[position]];
  setCommands(widget, commands);
}

/// Коды провайдеров — строки берём из словаря (`provider.xml` и т. п.).
const PROVIDER_LABELS = {
  XML: "provider.xml",
  JSON: "provider.json",
  Excel: "provider.excel",
  "Google Sheets": "provider.sheets",
  NDI: "provider.ndi",
  "Файлы": "provider.files",
};

function providerLabel(provider) {
  if (!provider) return "—";
  const key = PROVIDER_LABELS[provider];
  if (key) return t(key);
  if (provider.includes("не поддержан")) {
    return t("provider.unsupported", provider.replace(" (не поддержан)", ""));
  }
  return provider;
}

function rowsFor(index) {
  return state.externals[index];
}

/// Просмотр строк источника — порт `RowsViewer` (каждая строка разбивается по «|»).
function showRows(index) {
  const widget = byIndex(index);
  const external = state.externals[index];
  if (!widget || !external) return toast("у виджета нет внешних данных", "error");

  $("rows-title").textContent = `${t("rows.title")} · ${widget.name || widget.label} · #${index}`;
  $("rows-info").textContent = `${t("rows.info", external.provider, (external.values || []).length)}${
    external.error ? ` · ${external.error}` : ""
  }`;

  const list = $("rows-list");
  list.replaceChildren();
  const values = external.values || [];
  if (!values.length) {
    const empty = document.createElement("div");
    empty.className = "rows-empty";
    empty.textContent = t("rows.empty");
    list.appendChild(empty);
  } else {
    const columns = Math.max(...values.map((value) => value.split("|").length));
    values.forEach((value, position) => {
      const row = document.createElement("div");
      row.className = `rows-row${position === 0 ? " head" : ""}`;
      const cells = value.split("|");
      for (let column = 0; column < columns; column += 1) {
        const cell = document.createElement("span");
        cell.className = "cell";
        cell.textContent = cells[column] ?? "";
        if (position === 0) cell.textContent = cell.textContent || `колонка ${column + 1}`;
        row.appendChild(cell);
      }
      row.title = value;
      list.appendChild(row);
    });
  }
  $("rows").classList.remove("hidden");
}

function closeRows() {
  $("rows").classList.add("hidden");
}

function wireRowsViewer() {
  $("rows-close").addEventListener("click", closeRows);
  $("rows").addEventListener("pointerdown", (event) => {
    if (event.target === $("rows")) closeRows();
  });
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !$("rows").classList.contains("hidden")) closeRows();
  });
}

async function externalApply(index, row) {
  try {
    const result = await invoke("external_apply", {
      index,
      row,
      connection: state.connection,
    });
    toast(result.log.length ? result.log.join(" · ") : "нечего применять");
    await refreshVmix();
  } catch (error) {
    toast(String(error), "error");
  }
}

async function pressWidget(index, container = null) {
  try {
    const result = await invoke("vmix_press", { index, connection: state.connection });
    if (result.log.length) toast(result.log.join(" · "));
    else toast("у виджета нет команд");
    applyPageCommand(result);
    // подсветка придёт из состояния vMix, как в оригинале
    await refreshVmix();
  } catch (error) {
    toast(String(error), "error");
  }
}

/// Команды страниц (`NextPage`/`PrevPage`/`SetPage`) — внутренние для UTC,
/// поэтому применяет интерфейс, а не vMix.
function applyPageCommand(result) {
  const count = state.doc.pages.length;
  if (result.page !== null && result.page !== undefined) {
    state.view.page = Math.max(0, Math.min(count - 1, result.page));
  } else if (result.pageDelta) {
    state.view.page = (((state.view.page + result.pageDelta) % count) + count) % count;
  } else {
    return;
  }
  state.selected = null;
  render();
}

function closeProperties() {
  $("props").classList.add("hidden");
}

function renderProperties() {
  if ($("props").classList.contains("hidden")) return;
  if (state.selected === null || !byIndex(state.selected)) return closeProperties();
  openProperties();
}

// ------------------------------------------------------------------ vMix

function renderVmix() {
  const node = $("vmix-status");
  if (!state.vmix) {
    node.textContent = "нет связи";
    node.className = "pill";
  } else {
    node.textContent = `vMix ${state.vmix.version ?? ""}`.trim();
    node.className = "pill ok";
  }
  $("btn-stream").classList.toggle("on", Boolean(state.vmix?.streaming));
  $("btn-record").classList.toggle("on", Boolean(state.vmix?.recording));
  $("btn-ftb").classList.toggle("on", Boolean(state.vmix?.fadeToBlack));
}

async function refreshVmix() {
  try {
    // один запрос: состояние vMix + пересчитанная «нажатость» виджетов
    const result = await invoke("vmix_poll", { connection: state.connection });
    state.vmix = result.state;
    // MIDI-сообщения разбираются тем же циклом
    try {
      const midi = await invoke("midi_poll", { connection: state.connection });
      if (midi.device) state.midi = { device: midi.device };
      if (midi.log?.length) toast(midi.log.join(" · "));
    } catch (error) {
      state.midiError = String(error);
    }
    try {
      const schedule = await invoke("schedule_poll", { connection: state.connection });
      if (schedule.log?.length) toast(schedule.log.join(" · "));
    } catch (error) {
      /* расписание пустое — это нормально */
    }
    try {
      const deck = await invoke("streamdeck_poll", { connection: state.connection });
      if (deck.device) state.deck = { ...(state.deck || {}), device: deck.device };
      if (deck.log?.length) toast(deck.log.join(" · "));
    } catch (error) {
      state.deckError = String(error);
    }
    applyWidgetStates(result.widgets);
    applyExternalRows(result.externals || []);
  } catch (error) {
    state.vmix = null;
    $("vmix-status").textContent = String(error).slice(0, 60);
    $("vmix-status").className = "pill error";
    return;
  }
  renderVmix();
}

/// Имя функции vMix для цели громкости: Input → SetVolume, Master → SetMasterVolume, Bus A → SetBusAVolume.
function volumeFunction(target) {
  if (target.startsWith("Bus ")) return `SetBus${target.slice(4).replace(" ", "")}Volume`;
  return `Set${target}Volume`;
}

function updateClocksIn(root = document) {
  const now = new Date();
  for (const clock of root.querySelectorAll("[data-clock]")) {
    const useUtc = clock.dataset.useUtc === "1";
    const withSeconds = clock.dataset.seconds === "1";
    const parts = [];
    const time = useUtc
      ? now.toISOString().slice(11, withSeconds ? 19 : 16)
      : now.toLocaleTimeString("ru-RU", { hour12: false, hour: "2-digit", minute: "2-digit", ...(withSeconds ? { second: "2-digit" } : {}) });
    parts.push(time);
    if (clock.dataset.date === "1") {
      parts.push(useUtc ? now.toISOString().slice(0, 10) : now.toLocaleDateString("ru-RU"));
    }
    clock.textContent = parts.join(" · ");
  }
}

/// Картинка потока живёт между перерисовками: иначе браузер переподключался бы к MJPEG
/// на каждое обновление состояния, и вместо видео мелькала бы подпись.
function ndiView(index, external) {
  state.ndiViews ||= {};
  const current = state.ndiViews[index];
  if (current && current.dataset.src === external.stream) return current;

  const view = document.createElement("img");
  view.className = "ndi-view";
  view.alt = external.provider;
  view.title = external.provider;
  view.dataset.src = external.stream;
  view.src = external.stream;
  state.ndiViews[index] = view;
  return view;
}

/// Строки внешних данных приходят из Rust: складываем их и перерисовываем виджеты,
/// если данные действительно изменились.
function applyExternalRows(rows) {
  const before = externalSignature();
  for (const row of rows) state.externals[row.index] = row;
  for (const index of Object.keys(state.externals)) {
    if (!rows.some((row) => String(row.index) === String(index))) delete state.externals[index];
  }
  if (externalSignature() !== before) renderCanvas();
  if (state.pendingRows != null && rowsFor(state.pendingRows)) {
    const index = state.pendingRows;
    state.pendingRows = null;
    showRows(index);
  }
}

function externalSignature() {
  return Object.values(state.externals)
    .map((row) => `${row.index}:${(row.values || []).join("~")}`)
    .sort()
    .join("|");
}

/// «Нажатость» считает Rust по состоянию vMix (`ActiveStatePath`/`ActiveStateXPath`),
/// здесь только обновляем классы — без перерисовки поверхности.
function applyWidgetStates(widgets) {
  if (Array.isArray(widgets) && widgets.length) state.doc.widgets = widgets;
  for (const element of canvas.querySelectorAll(".widget")) {
    const widget = byIndex(Number(element.dataset.index));
    const face = element.querySelector(".button-face");
    if (face) face.classList.toggle("pressed", Boolean(widget && widget.active));
  }
}

async function connectVmix() {
  state.connection.host = $("host").value.trim() || "127.0.0.1";
  state.connection.port = Number($("port").value) || 8088;
  if (state.pollTimer) clearInterval(state.pollTimer);
  await refreshVmix();
  state.pollTimer = setInterval(refreshVmix, 1000);
}

async function sendFunction(fn, params = []) {
  try {
    await invoke("vmix_call", { connection: state.connection, function: fn, params });
    await refreshVmix();
  } catch (error) {
    toast(String(error), "error");
  }
}

// ------------------------------------------------------------------ запуск

window.addEventListener("DOMContentLoaded", async () => {
  try {
    state.palette = await invoke("vmc_palette");
  } catch (error) {
    toast(`меню виджетов недоступно: ${error}`, "error");
  }

  $("connect").addEventListener("click", connectVmix);
  $("new").addEventListener("click", newDocument);
  $("open").addEventListener("click", openDocument);
  $("save").addEventListener("click", saveDocument);
  $("lock").addEventListener("click", () =>
    applyDoc(invoke("vmc_set_locked", { locked: !state.locked })));

  // каталог функций vMix — для редактора скрипта
  try {
    const info = await invoke("catalogue_info");
    state.functions = info.functions;
    state.functionMeta = Object.fromEntries(info.functions.map((item) => [item.function, item]));
    console.log(`каталог функций: ${info.count}`);
  } catch (error) {
    toast(`каталог функций недоступен: ${error}`, "error");
  }

  // язык: из ответа бэкенда (настройки платформы), иначе из локали системы
  let initialLanguage = "ru";
  try {
    initialLanguage = await invoke("language");
  } catch (error) {
    initialLanguage = (navigator.language || "ru").slice(0, 2);
  }
  await loadLocale(initialLanguage);

  $("language").addEventListener("change", async () => {
    const chosen = await invoke("set_language", { lang: $("language").value });
    await loadLocale(chosen);
    render();
    toast(chosen === "ru" ? "Язык: русский" : "Language: English");
  });

  wireScriptEditor();
  wireRowsViewer();
  try {
    state.midiPorts = await invoke("midi_ports");
  } catch (error) {
    state.midiPorts = [];
  }
  try {
    state.deckDevices = await invoke("streamdeck_devices");
  } catch (error) {
    state.deckDevices = [];
  }
  // часы идут без участия vMix — обновляем раз в секунду
  window.setInterval(() => updateClocksIn(), 1000);
  $("zoom-in").addEventListener("click", () => zoomBy(1.25));
  $("zoom-out").addEventListener("click", () => zoomBy(0.8));
  $("zoom-reset").addEventListener("click", () => {
    state.view.zoom = 1;
    renderZoom();
  });

  for (const button of document.querySelectorAll("button[data-fn]")) {
    button.addEventListener("click", () => sendFunction(button.dataset.fn));
  }
  $("btn-stream").addEventListener("click", () =>
    sendFunction(state.vmix?.streaming ? "StopStreaming" : "StartStreaming"));
  $("btn-record").addEventListener("click", () =>
    sendFunction(state.vmix?.recording ? "StopRecording" : "StartRecording"));
  $("btn-ftb").addEventListener("click", () =>
    sendFunction(state.vmix?.fadeToBlack ? "FadeToBlackOff" : "FadeToBlackOn"));

  surface.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    surfaceMenu(event);
  });
  surface.addEventListener("pointerdown", (event) => {
    hideMenu();
    if (event.target === surface || event.target === canvas || event.target.classList.contains("grid-layer")) {
      select(null);
      closeProperties();
    }
  });

  // панорамирование средней кнопкой
  surface.addEventListener("pointerdown", (event) => {
    if (event.button !== 1) return;
    event.preventDefault();
    const start = { x: event.clientX, y: event.clientY, left: surface.scrollLeft, top: surface.scrollTop };
    const move = (moveEvent) => {
      surface.scrollLeft = start.left - (moveEvent.clientX - start.x);
      surface.scrollTop = start.top - (moveEvent.clientY - start.y);
    };
    const up = () => {
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerup", up);
    };
    document.addEventListener("pointermove", move);
    document.addEventListener("pointerup", up);
  });

  // Ctrl+колесо — масштаб, обычное колесо — прокрутка
  surface.addEventListener(
    "wheel",
    (event) => {
      if (!event.ctrlKey && !event.metaKey) return;
      event.preventDefault();
      zoomBy(event.deltaY < 0 ? 1.1 : 0.9);
    },
    { passive: false }
  );

  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      hideMenu();
      select(null);
      closeProperties();
      return;
    }
    const widget = byIndex(state.selected);
    if (!widget) return;
    if ((event.key === "Delete" || event.key === "Backspace") && !widget.locked) {
      event.preventDefault();
      removeWidget(widget.index);
      return;
    }
    const step = event.shiftKey ? 10 : 1;
    const moves = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] };
    const delta = moves[event.key];
    if (delta) {
      event.preventDefault();
      updateWidget(widget.index, { left: widget.left + delta[0], top: widget.top + delta[1] });
    }
  });

  window.addEventListener("blur", hideMenu);

  // документ из аргумента командной строки либо пустой (как в оригинале)
  await applyDoc(invoke("vmc_startup"));
  if (state.doc.selected !== null && state.doc.selected !== undefined && byIndex(state.doc.selected)) {
    select(state.doc.selected);
    openProperties();
    if (state.doc.openScript) {
      openScriptEditor(state.doc.selected);
      // сразу открываем поиск функции: в редакторе скрипта это первое действие
      setTimeout(() => document.querySelector("#script .picker-input")?.focus(), 50);
    }
    if (state.doc.showPickerQuery) {
      // dev-удобство: проверить фильтр поиска функций без клавиатуры
      setTimeout(() => {
        const field = document.querySelector("#script .picker-input")
          || document.querySelector("#props .picker-input");
        if (field) {
          field.value = state.doc.showPickerQuery;
          field.dispatchEvent(new Event("input"));
          field.focus();
          // панель свойств может быть прокручена — показываем поле с подсказками
          field.closest(".picker")?.scrollIntoView({ block: "center" });
        }
      }, 120);
    }
    if (state.doc.showProps) {
      setTimeout(() => openProperties(), 150);
    }
    if (state.doc.showPalette) {
      // dev-удобство: показать меню поверхности (проверка подписей списка виджетов)
      setTimeout(() => surfaceMenu({ clientX: 320, clientY: 220 }), 200);
    }
    // строки источника появятся после первого опроса — откроем окно тогда
    if (state.doc.openRows) state.pendingRows = state.doc.selected;
    if (state.doc.showSchedule) {
      // dev-удобство: сразу показать редактор расписания
      const schedule = document.querySelector(".commands h3");
      for (const heading of document.querySelectorAll(".commands h3")) {
        if (heading.textContent.startsWith("Расписание")) {
          heading.scrollIntoView({ block: "center" });
          heading.parentElement.style.outline = "2px solid #00ffff";
          break;
        }
      }
      void schedule;
    }
    if (state.doc.startDeck) {
      try {
        state.deck = { device: await invoke("streamdeck_start", { device: null }) };
        renderCanvas();
        toast(state.deck.device);
      } catch (error) {
        toast(`Stream Deck: ${error}`, "error");
      }
    }
    if (state.doc.startMidi) {
      try {
        state.midi = { device: await invoke("midi_start", { device: null }) };
        renderCanvas();
        toast(state.midi.device);
      } catch (error) {
        toast(`MIDI: ${error}`, "error");
      }
    }
  }
  renderVmix();
  await connectVmix();
});

function zoomBy(factor) {
  const next = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, state.view.zoom * factor));
  if (next === state.view.zoom) return;
  state.view.zoom = Number(next.toFixed(3));
  renderZoom();
  canvas.style.transform = `scale(${state.view.zoom})`;
}

// ------------------------------------------------------------------ редактор скрипта

let scriptDraft = null;
let scriptIndex = null;
let scriptExpanded = new Set();

/// Метаданные функции из каталога: что показывать в полях.
function functionMeta(name) {
  return state.functionMeta?.[name] ?? null;
}

function openScriptEditor(index) {
  const widget = byIndex(index);
  if (!widget) return toast("сначала выберите виджет", "error");
  if (!String(widget.typeName).includes("Button")) {
    return toast("скрипт есть только у кнопок", "error");
  }
  scriptIndex = index;
  scriptDraft = widget.commands.map((command) => ({ ...command }));
  // сразу раскрываем команду с условием — у неё самые интересные параметры
  const condition = scriptDraft.findIndex(
    (command) => (command.additionalParameters || []).length > 0
  );
  scriptExpanded = new Set(condition >= 0 ? [condition] : []);
  $("script-title").textContent = `Скрипт · ${widget.name || widget.label} · #${index}`;
  $("script-text").classList.add("hidden");
  $("script-list").classList.remove("hidden");
  $("script").classList.remove("hidden");
  renderScript();
}

function closeScriptEditor() {
  $("script").classList.add("hidden");
  scriptDraft = null;
  scriptIndex = null;
}

function renderScript() {
  const list = $("script-list");
  list.replaceChildren();
  if (!scriptDraft || !scriptDraft.length) {
    const empty = document.createElement("div");
    empty.className = "hint";
    empty.textContent = t("script.empty");
    list.appendChild(empty);
    return;
  }
  scriptDraft.forEach((command, position) => list.appendChild(scriptRow(command, position)));
}

function scriptRow(command, position) {
  const row = document.createElement("div");
  row.className = "script-row";

  const line = document.createElement("div");
  line.className = "script-line";

  const number = document.createElement("span");
  number.className = "no";
  number.textContent = String(position + 1);

  const enabled = document.createElement("input");
  enabled.type = "checkbox";
  enabled.checked = command.executable !== false;
  enabled.title = "Исполнять команду (IsExecutable)";
  enabled.addEventListener("change", () => {
    command.executable = enabled.checked;
  });

  const name = document.createElement("button");
  name.className = "fn";
  name.textContent = command.function || t("script.pickFunction");
  name.title = command.description || command.formatString || "";
  name.addEventListener("click", () => {
    if (scriptExpanded.has(position)) scriptExpanded.delete(position);
    else scriptExpanded.add(position);
    renderScript();
  });

  const summary = document.createElement("span");
  summary.className = "summary";
  summary.textContent = commandSummary(command);

  const tools = document.createElement("span");
  tools.className = "tools";
  tools.append(
    smallButton("↑", () => moveScriptCommand(position, -1)),
    smallButton("↓", () => moveScriptCommand(position, 1)),
    smallButton("⧉", () => {
      scriptDraft.splice(position + 1, 0, { ...command });
      renderScript();
    }),
    smallButton("✕", () => {
      scriptDraft.splice(position, 1);
      renderScript();
    })
  );

  line.append(number, enabled, name, summary, tools);
  row.appendChild(line);
  if (scriptExpanded.has(position)) row.appendChild(scriptFields(command, position));
  return row;
}

/// Строка-превью команды, как `ToString()` в оригинале.
function commandSummary(command) {
  const parts = [];
  if (command.inputKey) parts.push(`<${inputTitle(command.inputKey)}>`);
  else if (command.input) parts.push(String(command.input));
  if (command.parameter) parts.push(command.parameter);
  if (command.stringParameter) parts.push(JSON.stringify(command.stringParameter));
  if (command.floatParameter) parts.push(command.floatParameter);
  for (const extra of command.additionalParameters || []) if (extra) parts.push(extra);
  return parts.join(", ");
}

function inputTitle(key) {
  const input = (state.vmix?.inputs || []).find((item) => item.key === key);
  if (!input) return key;
  return `#${input.number ?? "?"} ${input.title ?? ""}`.trim();
}

function scriptFields(command, position) {
  const meta = functionMeta(command.function);
  const fields = document.createElement("div");
  fields.className = "script-fields";

  const add = (label, input, hint) => {
    const text = document.createElement("label");
    text.textContent = label;
    fields.append(text, input);
    if (hint) {
      const note = document.createElement("span");
      note.className = "hint";
      note.textContent = hint;
      fields.appendChild(note);
    }
    return input;
  };

  const textField = (value, onChange, list) => {
    const input = document.createElement("input");
    input.type = "text";
    input.value = value ?? "";
    if (list) input.setAttribute("list", list);
    input.addEventListener("input", () => onChange(input.value));
    return input;
  };

  // функция: свой поиск (в WKWebView datalist не работает)
  const functionInput = functionPicker(command.function, (value) => {
    command.function = value;
    renderScript();
  });
  add(t("props.function"), functionInput, meta?.description || null);

  // вход
  if (!meta || meta.hasInput) {
    const select = document.createElement("select");
    const empty = document.createElement("option");
    empty.value = "";
    empty.textContent = "—";
    select.appendChild(empty);
    for (const input of state.vmix?.inputs || []) {
      const option = document.createElement("option");
      option.value = input.key || String(input.number);
      option.textContent = `#${input.number ?? "?"} ${input.title ?? ""}`.trim();
      option.selected = option.value === command.inputKey;
      select.appendChild(option);
    }
    select.addEventListener("change", () => {
      command.inputKey = select.value || null;
      if (!select.value && command.input == null) command.input = null;
      renderScript();
    });
    add(t("props.input"), select);
  }

  // числовой параметр
  if (!meta || meta.hasInt) {
    add(
      meta?.intDescription || "Параметр",
      Object.assign(document.createElement("input"), {
        type: "number",
        value: command.parameter ?? "",
        oninput: (event) => {
          command.parameter = event.target.value;
        },
      })
    );
  }

  // текстовый параметр
  if (!meta || meta.hasString) {
    add(
      meta?.stringDescription || "Значение",
      textField(command.stringParameter ?? "", (value) => {
        command.stringParameter = value;
      }, meta?.stringValues?.length ? `values-${command.function}` : null)
    );
    if (meta?.stringValues?.length) ensureValueList(`values-${command.function}`, meta.stringValues);
  }

  // float
  if (meta?.hasFloat) {
    add(
      meta.floatDescription || "Дробное",
      textField(command.floatParameter ?? "", (value) => {
        command.floatParameter = value;
      })
    );
  }

  // дополнительные параметры: у Condition это сравниваемые выражения
  const additionalCount = Math.max(meta?.additionalCount ?? 0, command.function === "Condition" ? 4 : 0);
  if (additionalCount > 0) {
    const labels = [t("script.field.value1"), t("script.field.expression1"), t("script.field.operator"), t("script.field.expression2")];
    while ((command.additionalParameters || []).length < additionalCount) {
      (command.additionalParameters ||= []).push("");
    }
    for (let index = 0; index < additionalCount; index += 1) {
      add(
        labels[index] || `Доп. ${index + 1}`,
        textField(command.additionalParameters[index] ?? "", (value) => {
          command.additionalParameters[index] = value;
        }, index === 2 ? "operators" : null)
      );
    }
    const note = document.createElement("span");
    note.className = "hint";
    note.textContent = t("script.expressionHint");
    fields.appendChild(note);
  }

  return fields;
}

function ensureValueList(id, values) {
  if (document.getElementById(id)) return;
  const list = document.createElement("datalist");
  list.id = id;
  for (const value of values) {
    const option = document.createElement("option");
    option.value = value;
    list.appendChild(option);
  }
  document.body.appendChild(list);
}

function moveScriptCommand(position, delta) {
  const target = position + delta;
  if (target < 0 || target >= scriptDraft.length) return;
  [scriptDraft[position], scriptDraft[target]] = [scriptDraft[target], scriptDraft[position]];
  renderScript();
}

async function addScriptCommand() {
  const command = {
    function: "Cut",
    executable: true,
    useInActiveState: true,
    formatString: "",
    additionalParameters: [],
  };
  scriptDraft.push(command);
  scriptExpanded = new Set([scriptDraft.length - 1]);
  renderScript();
}

async function saveScript() {
  if (scriptIndex === null || !scriptDraft) return;
  const index = scriptIndex;
  closeScriptEditor();
  await applyDoc(invoke("vmix_set_commands", { index, commands: scriptDraft }), "скрипт сохранён");
}

async function exportScript() {
  if (scriptIndex === null) return;
  try {
    const text = await invoke("vmc_export_script", { index: scriptIndex });
    const area = $("script-text");
    area.value = text;
    area.classList.remove("hidden");
    $("script-list").classList.add("hidden");
    $("script-export").textContent = "Применить текст";
    $("script-export").onclick = importScript;
  } catch (error) {
    toast(String(error), "error");
  }
}

async function importScript() {
  if (scriptIndex === null) return;
  const index = scriptIndex;
  const text = $("script-text").value;
  try {
    const doc = await invoke("vmc_import_script", { index, text });
    state.doc = doc;
    render();
    const widget = byIndex(index);
    scriptDraft = widget ? widget.commands.map((command) => ({ ...command })) : [];
    scriptExpanded = new Set();
    $("script-text").classList.add("hidden");
    $("script-list").classList.remove("hidden");
    $("script-export").textContent = "Экспорт";
    $("script-export").onclick = exportScript;
    renderScript();
    toast("скрипт импортирован");
  } catch (error) {
    toast(String(error), "error");
  }
}

function wireScriptEditor() {
  const list = document.createElement("datalist");
  list.id = "operators";
  for (const operator of ["=", "!=", ">", "<", ">=", "<=", "~", "`", "&&", "||"]) {
    const option = document.createElement("option");
    option.value = operator;
    list.appendChild(option);
  }
  document.body.appendChild(list);

  $("script-add").addEventListener("click", addScriptCommand);
  $("script-clear").addEventListener("click", () => {
    scriptDraft = [];
    renderScript();
  });
  $("script-export").addEventListener("click", exportScript);
  $("script-import").addEventListener("click", () => {
    $("script-text").classList.remove("hidden");
    $("script-list").classList.add("hidden");
    $("script-export").textContent = "Применить текст";
    $("script-export").onclick = importScript;
    $("script-text").focus();
  });
  $("script-save").addEventListener("click", saveScript);
  $("script-close").addEventListener("click", closeScriptEditor);
  $("script").addEventListener("pointerdown", (event) => {
    if (event.target === $("script")) closeScriptEditor();
  });
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !$("script").classList.contains("hidden")) closeScriptEditor();
  });
}
