const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);

let config = { bindings: [], enabled: true, startWithWindows: false, startMinimised: false };
let editing = null; // the binding being edited, or null when adding
let captured = null; // chord captured in the modal
let capturing = false;

// ------------------------------------------------------------------ boot

async function boot() {
  config = await invoke("get_config");
  render();
  loadSuggestions();

  // A chord arrives here only while the capture button is armed. Nothing else
  // the user types ever reaches the frontend.
  listen("chord-captured", async (event) => {
    if (!capturing) return;
    captured = event.payload;
    await stopCapture();
    const label = await invoke("describe_chord", { chord: captured });
    const btn = $("fCapture");
    btn.classList.add("set");
    $("fChordLabel").textContent = label;
    $("captureHint").textContent = captured.vk === 0x86
      ? "That's the Copilot key."
      : "Click again to pick a different key.";
  });

  listen("action-error", (event) => toast(event.payload, true));
  listen("file-picked", (event) => {
    $("fTarget").value = event.payload;
    validate();
  });
}

// ---------------------------------------------------------------- render

function render() {
  const has = config.bindings.length > 0;
  $("empty").classList.toggle("hidden", has);
  $("listWrap").classList.toggle("hidden", !has);

  $("master").checked = config.enabled;
  $("masterLabel").textContent = config.enabled ? "Enabled" : "Paused";
  $("startWithWindows").checked = config.startWithWindows;
  $("startMinimised").checked = config.startMinimised;

  const list = $("list");
  list.innerHTML = "";
  config.bindings.forEach((b, i) => {
    const li = document.createElement("li");
    li.className = "binding" + (b.enabled ? "" : " off");

    const chord = document.createElement("span");
    chord.className = "chord";
    chord.textContent = b.chordLabel || "…";
    li.appendChild(chord);

    const meta = document.createElement("div");
    meta.className = "meta";
    const name = document.createElement("div");
    name.className = "name";
    name.textContent = b.name || "Untitled";
    const target = document.createElement("div");
    target.className = "target";
    target.textContent = actionTarget(b.action);
    meta.append(name, target);
    li.appendChild(meta);

    li.appendChild(iconButton(b.enabled ? "Pause" : "Resume", "", async () => {
      config.bindings[i].enabled = !config.bindings[i].enabled;
      await persist();
    }));
    li.appendChild(iconButton("Edit", "", () => openModal(i)));
    li.appendChild(iconButton("Delete", "danger", async () => {
      config.bindings.splice(i, 1);
      await persist();
    }));

    list.appendChild(li);
  });

  // Chord labels come from Rust so the UI never has to know key codes.
  config.bindings.forEach(async (b, i) => {
    const label = await invoke("describe_chord", { chord: b.chord });
    b.chordLabel = label;
    const el = list.children[i];
    if (el) el.querySelector(".chord").textContent = label;
  });
}

function iconButton(text, extra, onClick) {
  const b = document.createElement("button");
  b.className = "icon " + extra;
  b.textContent = text;
  b.addEventListener("click", onClick);
  return b;
}

function actionTarget(action) {
  return action.path || action.url || action.command || "";
}

async function persist() {
  const payload = {
    bindings: config.bindings.map(({ chordLabel, ...rest }) => rest),
    enabled: config.enabled,
    startWithWindows: config.startWithWindows,
    startMinimised: config.startMinimised,
  };
  try {
    await invoke("save_config", { new: payload });
    render();
  } catch (err) {
    toast(String(err), true);
  }
}

// ----------------------------------------------------------------- modal

async function openModal(index, presetChord) {
  editing = typeof index === "number" ? index : null;
  const b = editing !== null ? config.bindings[editing] : null;

  $("modalTitle").textContent = b ? "Edit key" : "Add a key";
  $("fName").value = b ? b.name : "";
  $("fTarget").value = b ? actionTarget(b.action) : "";
  $("fArgs").value = b && b.action.args ? b.action.args : "";
  $("fKind").value = b ? b.action.kind : "launchApp";
  $("fError").classList.add("hidden");
  $("captureHint").textContent = "";

  captured = b ? b.chord : presetChord || null;
  const btn = $("fCapture");
  btn.classList.toggle("set", !!captured);
  btn.classList.remove("armed");
  $("fChordLabel").textContent = captured
    ? await invoke("describe_chord", { chord: captured })
    : "Click, then press a key";
  if (presetChord) {
    $("captureHint").textContent = "That's the Copilot key.";
    if (!$("fName").value) $("fName").value = "Copilot key";
  }

  onKindChange();
  $("modal").classList.remove("hidden");
  $("fName").focus();
}

async function closeModal() {
  await stopCapture();
  $("modal").classList.add("hidden");
  editing = null;
  captured = null;
}

async function startCapture() {
  capturing = true;
  await invoke("set_learn_mode", { on: true });
  const btn = $("fCapture");
  btn.classList.add("armed");
  btn.classList.remove("set");
  $("fChordLabel").textContent = "Press any key…";
  $("captureHint").textContent = "Hold modifiers too if you want them included.";
}

async function stopCapture() {
  if (!capturing) return;
  capturing = false;
  await invoke("set_learn_mode", { on: false });
  $("fCapture").classList.remove("armed");
}

function onKindChange() {
  const kind = $("fKind").value;
  const labels = {
    launchApp: "App",
    openUrl: "Website",
    openFolder: "Folder",
    runCommand: "Command",
  };
  const placeholders = {
    launchApp: "Pick or type a path",
    openUrl: "https://example.com",
    openFolder: "C:\\Users\\you\\Documents",
    runCommand: "shutdown /h",
  };
  $("fTargetLabel").textContent = labels[kind];
  $("fTarget").placeholder = placeholders[kind];
  $("fBrowse").classList.toggle("hidden", kind === "openUrl" || kind === "runCommand");
  $("fArgsField").classList.toggle("hidden", kind !== "launchApp");
  $("fTarget").setAttribute("list", kind === "launchApp" ? "appList" : "");
}

function buildAction() {
  const kind = $("fKind").value;
  const value = $("fTarget").value.trim();
  switch (kind) {
    case "launchApp":
      return { kind, path: value, args: $("fArgs").value.trim() };
    case "openUrl":
      return { kind, url: value };
    case "openFolder":
      return { kind, path: value };
    case "runCommand":
      return { kind, command: value };
  }
}

function validate() {
  $("fError").classList.add("hidden");
  return true;
}

function showError(msg) {
  const el = $("fError");
  el.textContent = msg;
  el.classList.remove("hidden");
}

async function save() {
  if (!captured) return showError("Pick a key first.");
  if (!$("fTarget").value.trim()) return showError("Say what the key should open.");

  const clash = config.bindings.findIndex(
    (b, i) => i !== editing && sameChord(b.chord, captured)
  );
  if (clash !== -1) {
    return showError(`That key is already used by "${config.bindings[clash].name}".`);
  }

  const binding = {
    id: editing !== null ? config.bindings[editing].id : crypto.randomUUID(),
    name: $("fName").value.trim() || "Untitled",
    chord: captured,
    action: buildAction(),
    enabled: editing !== null ? config.bindings[editing].enabled : true,
  };

  if (editing !== null) config.bindings[editing] = binding;
  else config.bindings.push(binding);

  await closeModal();
  await persist();
}

function sameChord(a, b) {
  return (
    a.vk === b.vk &&
    !!a.ctrl === !!b.ctrl &&
    !!a.shift === !!b.shift &&
    !!a.alt === !!b.alt &&
    !!a.win === !!b.win
  );
}

// ------------------------------------------------------------ suggestions

async function loadSuggestions() {
  try {
    const apps = await invoke("suggest_apps");
    const list = $("appList");
    list.innerHTML = "";
    for (const app of apps) {
      const opt = document.createElement("option");
      opt.value = app.path;
      opt.label = app.name;
      list.appendChild(opt);
    }
  } catch {
    /* the picker still works by hand */
  }
}

// ------------------------------------------------------------------ toast

let toastTimer;
function toast(message, bad) {
  const el = $("toast");
  el.textContent = message;
  el.classList.toggle("bad", !!bad);
  el.classList.remove("hidden");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.add("hidden"), 3200);
}

// ------------------------------------------------------------------ wire

$("master").addEventListener("change", async (e) => {
  config.enabled = e.target.checked;
  await persist();
});
$("startWithWindows").addEventListener("change", async (e) => {
  config.startWithWindows = e.target.checked;
  await persist();
});
$("startMinimised").addEventListener("change", async (e) => {
  config.startMinimised = e.target.checked;
  await persist();
});
$("openConfig").addEventListener("click", () => invoke("open_config_folder"));

$("add").addEventListener("click", () => openModal());
$("emptyAdd").addEventListener("click", () => openModal());
$("quickCopilot").addEventListener("click", async () => {
  const chord = await invoke("copilot_chord");
  openModal(null, chord);
});

$("fCapture").addEventListener("click", () => (capturing ? stopCapture() : startCapture()));
$("fKind").addEventListener("change", onKindChange);
$("fBrowse").addEventListener("click", () => {
  invoke($("fKind").value === "openFolder" ? "pick_folder" : "pick_app");
});
$("fTest").addEventListener("click", async () => {
  try {
    await invoke("test_action", { action: buildAction() });
    toast("Launched it.");
  } catch (err) {
    toast(String(err), true);
  }
});
$("fCancel").addEventListener("click", closeModal);
$("fSave").addEventListener("click", save);

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !$("modal").classList.contains("hidden")) closeModal();
});

boot();
