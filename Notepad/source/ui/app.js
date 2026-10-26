"use strict";
// Axentra Notepad's workbench: tabs, files and sessions, commands, menus, the command palette,
// find and replace, the explorer and search views, and the status bar. The text view lives in
// editor.js and the editing logic in core.js; Rust handles files, encodings, search and dialogs.

const $ = (s, root = document) => root.querySelector(s);
const NATIVE = !!(window.ipc && typeof window.ipc.postMessage === "function");
const LG = globalThis.AxnLang;
const APP = "Axentra Notepad";

// ------------------------------------------------------------------ icons
const ICON = {
  newFile: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5M12 11v6M9 14h6",
  open: "M3 7V5h7l2 2h9v12H3zM3 9h18",
  folder: "M3 7V5h7l2 2h9v12H3z",
  save: "M5 3h11l3 3v15H5zM8 3v5h7V3M8 21v-7h8v7",
  saveAll: "M7 6h10l3 3v12H7zM10 6v4h6V6M10 21v-5h7v5M4 17V3h11",
  close: "m6 6 12 12M18 6 6 18",
  cut: "M6 7a2.5 2.5 0 1 0 0-.01M6 17a2.5 2.5 0 1 0 0 .01M8 8l11 9M8 16l11-9",
  copy: "M8 8h12v12H8zM4 16V4h12",
  paste: "M9 4h6v3H9zM8 5H5v16h14V5h-3",
  undo: "M9 14 4 9l5-5M4 9h10a6 6 0 0 1 0 12h-3",
  redo: "m15 14 5-5-5-5M20 9H10a6 6 0 0 0 0 12h3",
  find: "M10.5 4a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM15.5 15.5 20 20",
  replace: "M4 7h9M4 12h9M4 17h6M15 14l3 3 4-5",
  zoomIn: "M10.5 4a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM15.5 15.5 20 20M8 10.5h5M10.5 8v5",
  zoomOut: "M10.5 4a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM15.5 15.5 20 20M8 10.5h5",
  wrap: "M4 6h16M4 12h13a3 3 0 0 1 0 6h-4m2-2-2 2 2 2M4 18h6",
  pilcrow: "M10 4h9M14 4v16M17 4v16M10 4a4 4 0 0 0 0 8h4",
  guides: "M5 4v16M10 8v12M15 12v8M19 4h-4",
  record: "M12 6a6 6 0 1 0 0 12 6 6 0 0 0 0-12z",
  stop: "M7 7h10v10H7z",
  play: "m8 5 11 7-11 7z",
  palette: "M4 6h16M4 12h10M4 18h7M17 15l3 3-3 3",
  bookmark: "M7 3h10v18l-5-4-5 4z",
  sidebar: "M4 5h16v14H4zM9 5v14",
  terminal: "M4 5h16v14H4zM7 10l3 2-3 2M12 15h5",
  print: "M7 9V3h10v6M7 17H4v-7h16v7h-3M7 14h10v7H7z",
};
const svg = (name, cls = "") => `<svg viewBox="0 0 24 24" class="${cls}" aria-hidden="true"><path d="${ICON[name] || name}"/></svg>`;
function el(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "html") {
      if (v != null) e.innerHTML = v;
    } else if (k === "on") {
      if (v) for (const [ev, fn] of Object.entries(v)) e.addEventListener(ev, fn);
    }
    else if (k === "class") e.className = v;
    else if (k === "style") e.style.cssText = v;
    else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v === true ? "" : v);
  }
  for (const k of kids) if (k !== null && k !== undefined && k !== false) e.append(k);
  return e;
}
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const basename = (p) => (p ? p.replace(/\/+$/, "").split("/").pop() : "");
const dirname = (p) => (p ? p.replace(/\/[^/]*$/, "") || "/" : "");
const plural = (n, w) => `${n.toLocaleString()} ${w}${n === 1 ? "" : "s"}`;

// ------------------------------------------------------------------ backend
async function post(path, body) {
  const r = await fetch(path, { method: "POST", body: typeof body === "string" ? body : JSON.stringify(body ?? {}) });
  return r.json();
}
async function api(op, args = {}) {
  try {
    return await post("/api", { op, ...args });
  } catch (e) {
    return { ok: false, error: `${APP} couldn't reach its file service (${e}).` };
  }
}
async function clipSet(text) {
  try { await fetch("/clip/set", { method: "POST", body: text }); } catch {}
}
async function clipGet() {
  try { return (await post("/clip/get", {})).text ?? ""; } catch { return ""; }
}

// ------------------------------------------------------------------ preferences
const LAYOUTS = {
  notepad: { activityBar: false, sidebar: false, toolbar: false, minimap: false, lineNumbers: "off", folding: false, glyphMargin: false, breadcrumbs: false, indentGuides: false, currentLine: false, wrap: true, scrollBeyond: false },
  notepadpp: { activityBar: false, sidebar: false, toolbar: true, minimap: false, lineNumbers: "on", folding: true, glyphMargin: true, breadcrumbs: false, indentGuides: true, currentLine: true, wrap: false, scrollBeyond: false },
  code: { activityBar: true, sidebar: true, toolbar: false, minimap: true, lineNumbers: "on", folding: true, glyphMargin: true, breadcrumbs: true, indentGuides: true, currentLine: true, wrap: false, scrollBeyond: true },
};
const LAYOUT_NAMES = { notepad: "Notepad", notepadpp: "Notepad++", code: "Code" };
const DEFAULT_FONT = '"Cascadia Mono", "Cascadia Code", "JetBrains Mono", "DejaVu Sans Mono", "Ubuntu Mono", "Liberation Mono", monospace';
const DEFAULTS = {
  theme: "dark", layout: "code", fontFamily: "", fontSize: 14, lineHeight: 1.5, zoom: 100,
  ...LAYOUTS.code, tabs: true, statusBar: true, whitespace: "none", eol: false, rulers: "",
  cursorStyle: "line", cursorBlink: true, matchBrackets: true, occurrences: true, highlight: true,
  tabSize: 4, insertSpaces: true, detectIndent: true, autoClose: true, autoIndent: true, smartHome: true,
  restoreSession: true, unsavedOnClose: "ask", trimTrailing: false, finalNewline: false, defaultEol: "lf", defaultEncoding: "utf-8",
  sideView: "explorer", sideWidth: 260, openEditors: true, wrapColumn: 0,
  findCase: false, findWord: false, findRegex: false,
};
const prefs = (() => {
  try { return Object.assign({ ...DEFAULTS }, JSON.parse(localStorage.getItem("axn-prefs") || "{}")); } catch { return { ...DEFAULTS }; }
})();
function savePrefs() {
  try { localStorage.setItem("axn-prefs", JSON.stringify(prefs)); } catch {}
}
function setPref(k, v) {
  prefs[k] = v;
  savePrefs();
  applyPrefs();
}
function togglePref(k) { setPref(k, !prefs[k]); }

// ------------------------------------------------------------------ state
const S = {
  tabs: [], active: null, folder: null, recent: [], recentFolders: [], primary: true, home: "", encodings: [], fonts: [],
  closed: [], overtype: false, columnMode: false, box: null, version: "", status: "", statusTimer: 0,
  macro: { recording: false, steps: [], last: [] }, folderFiles: null, folderFilesAt: 0,
};
let view;
const active = () => S.active;
const langOf = (t) => LG.byId(t ? t.langId : "plaintext");
const newId = () => Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
const firstLine = (text) => (text || "").slice(0, 300).split("\n")[0];

function editOpts(tab = S.active) {
  return { lang: langOf(tab), tabSize: tab.tabSize, insertSpaces: tab.insertSpaces, autoClose: prefs.autoClose, autoIndent: prefs.autoIndent };
}
function wordRe(tab = S.active) {
  return langOf(tab).wordChars || AxnCore.DEFAULT_WORD;
}

// ------------------------------------------------------------------ tabs
function makeTab(d) {
  const text = d.text ?? "";
  const untitled = d.path ? 0 : d.untitled || nextUntitled();
  const name = d.name || (d.path ? basename(d.path) : `Untitled-${untitled}`);
  const langId = d.lang && LG.byId(d.lang).id === d.lang ? d.lang : LG.detect(d.path ? name : "", firstLine(text));
  const buf = new AxnCore.Buffer(text, { lang: prefs.highlight ? langId : "plaintext", tokenize: LG.tokenize });
  const tab = {
    id: d.id || newId(), buf, path: d.path || null, name, untitled,
    encoding: d.encoding || prefs.defaultEncoding, eol: d.eol && d.eolDetected !== false ? d.eol : prefs.defaultEol,
    mtime: d.mtime || 0, size: d.size || 0, readonlyFile: !!d.readonly, missing: !!d.missing, langId, langLocked: !!d.langLocked,
    bookmarks: new Set(), scroll: { top: 0, left: 0 }, conflict: false, sessionVersion: -1, notices: [], lastDirty: false,
  };
  tab.layout = new AxnCore.Layout(buf);
  const lang = LG.byId(langId);
  tab.tabSize = lang.tabSize || prefs.tabSize;
  tab.insertSpaces = lang.useTabs ? false : prefs.insertSpaces;
  if (prefs.detectIndent) {
    const det = AxnCore.detectIndent(buf.doc);
    if (det) {
      tab.insertSpaces = det.insertSpaces;
      if (det.tabSize) tab.tabSize = det.tabSize;
    }
  }
  if (d.binary) tab.notices.push({ kind: "warn", text: "This file looks like binary data. Saving it from a text editor can damage it." });
  if (d.lossy) tab.notices.push({ kind: "warn", text: `Some bytes aren't valid ${encName(tab.encoding)}. Reopen it with the right encoding before saving.`, action: ["Reopen with Encoding…", () => pickEncoding(tab, "reopen")] });
  if (d.mixedEol) tab.notices.push({ kind: "info", text: `This file mixed line endings. Saving uses ${eolName(tab.eol)} throughout.` });
  buf.on((c) => onBufChange(tab, c));
  return tab;
}

function onBufChange(tab, c) {
  if (tab.bookmarks.size) {
    const next = new Set();
    for (const l of tab.bookmarks) {
      const m = AxnCore.mapLine(l, c);
      next.add(m < 0 ? c.from.line : m);
    }
    tab.bookmarks = next;
  }
  if (tab === S.active && F.open) scheduleFind();
  if (tab.buf.dirty !== tab.lastDirty) {
    tab.lastDirty = tab.buf.dirty;
    renderTabs();
    renderOpenEditors();
  }
  sessionDirty();
  scheduleChrome();
}

function untitledPristine(t) {
  return t && !t.path && !t.buf.dirty && t.buf.doc.size === 1 && !t.buf.doc.lines[0];
}

function addTab(tab, { activate: act = true, at = null } = {}) {
  const i = at ?? (S.active ? S.tabs.indexOf(S.active) + 1 : S.tabs.length);
  S.tabs.splice(i, 0, tab);
  if (act) activate(tab);
  else renderTabs();
  sessionDirty();
  return tab;
}

/** Untitled tabs take the lowest number no open tab uses, like VS Code and Notepad++. */
function nextUntitled() {
  const used = new Set(S.tabs.filter((t) => !t.path).map((t) => t.untitled));
  let n = 1;
  while (used.has(n)) n++;
  return n;
}

function newTab(text = "") {
  const t = makeTab({ text });
  addTab(t);
  return t;
}

async function newCodeFile() {
  const items = LG.languages.filter((l) => l.id !== "plaintext").map((l) => ({ label: l.name, desc: (l.exts || []).slice(0, 3).join(" "), value: l.id, icon: fileIconHtml(l.id) }));
  const id = await pick(items, { placeholder: "Language for the new file (type to filter)" });
  if (!id) return;
  setLang(newTab(), id, true);
}

function activate(tab, { focus = true } = {}) {
  if (!tab) return;
  if (S.active && S.active !== tab) S.active.scroll = { top: view.scroller.scrollTop, left: view.scroller.scrollLeft };
  S.active = tab;
  configureView();
  view.setTab(tab);
  if (F.open) {
    F.version = -1;
    refreshFind();
  }
  renderTabs();
  renderChrome();
  renderOpenEditors();
  renderTree();
  if (focus) view.focus();
  sessionDirty(true);
}

async function closeTab(tab, { force = false } = {}) {
  if (!tab) return false;
  if (tab.buf.dirty && !force) {
    const choice = await ask({ title: APP, body: `Do you want to save the changes you made to “${tab.name}”?`, note: "Your changes will be lost if you don't save them.", buttons: [{ label: "Cancel", value: "cancel", cancel: true }, { label: "Don't Save", value: "discard" }, { label: "Save", value: "save", primary: true }] });
    if (choice === "cancel" || choice === null) return false;
    if (choice === "save" && !(await saveTab(tab))) return false;
  }
  const i = S.tabs.indexOf(tab);
  if (i < 0) return true;
  if (tab.path || !untitledPristine(tab)) S.closed.push(closedRecord(tab));
  if (S.closed.length > 20) S.closed.shift();
  S.tabs.splice(i, 1);
  tab.layout.dispose();
  if (S.active === tab) {
    S.active = null;
    const next = S.tabs[Math.min(i, S.tabs.length - 1)];
    if (next) activate(next);
    else view.setTab(null);
  }
  renderTabs();
  renderChrome();
  renderOpenEditors();
  sessionDirty();
  return true;
}
function closedRecord(t) {
  return { path: t.path, text: t.path && !t.buf.dirty ? null : t.buf.doc.text(), name: t.name, untitled: t.untitled, encoding: t.encoding, eol: t.eol, sel: t.buf.main.head, dirty: t.buf.dirty };
}
async function reopenClosed() {
  const r = S.closed.pop();
  if (!r) return status("No closed tabs to reopen");
  if (r.text === null && r.path) {
    await openPath(r.path, { at: r.sel });
  } else {
    const free = r.path || !S.tabs.some((o) => !o.path && o.untitled === r.untitled);
    const t = makeTab({ text: r.text, path: r.path, name: free ? r.name : undefined, untitled: free ? r.untitled : undefined, encoding: r.encoding, eol: r.eol });
    if (r.dirty) t.buf.markDirty();
    addTab(t);
    t.buf.setSels([AxnCore.S(r.sel)]);
    view.afterMove(true);
  }
}
async function closeTabs(list) {
  for (const t of [...list]) if (!(await closeTab(t))) return false;
  return true;
}

// ------------------------------------------------------------------ files
const encName = (id) => (S.encodings.find((e) => e.id === id) || { name: id.toUpperCase() }).name;
const eolName = (e) => ({ crlf: "Windows (CRLF)", lf: "Unix (LF)", cr: "Macintosh (CR)" })[e] || e;
const eolShort = (e) => ({ crlf: "CRLF", lf: "LF", cr: "CR" })[e] || e;

function findTab(path) {
  return S.tabs.find((t) => t.path === path) || null;
}

/** Adds a tab for a document the backend read (or focuses it if it's open). */
function openDoc(doc, { at = null, activate: act = true } = {}) {
  let tab = findTab(doc.path);
  if (!tab) {
    const replace = untitledPristine(S.active) ? S.active : null;
    tab = makeTab(doc);
    addTab(tab, { activate: false, at: replace ? S.tabs.indexOf(replace) : null });
    if (replace) {
      S.tabs.splice(S.tabs.indexOf(replace), 1);
      replace.layout.dispose();
      if (S.active === replace) S.active = null;
    }
    rememberRecent(doc.path);
  }
  if (act) activate(tab);
  if (at) {
    const p = tab.buf.doc.clip(at.line !== undefined ? at : AxnCore.P(at[0], at[1]));
    tab.buf.setSels([at.to ? AxnCore.S(p, tab.buf.doc.clip(at.to)) : AxnCore.S(p)]);
    if (act) view.afterMove(false), view.reveal(tab.buf.main.head, 0, "center");
  }
  return tab;
}

async function openPath(path, opts = {}) {
  const open = findTab(path);
  if (open) return openDoc({ path }, opts);
  const r = await api("open", { path });
  if (!r.ok) {
    if (r.isDir) return setFolder(path);
    if (r.notFound && opts.create) return openDoc({ path, name: basename(path), text: "", missing: true }, opts);
    await alertBox("Couldn't open the file", r.error);
    if (r.notFound) {
      forgetRecent(path);
    }
    return null;
  }
  return openDoc(r.doc, opts);
}

async function openPaths(paths) {
  const r = await api("open_paths", { paths });
  if (!r.ok) return toast(r.error, true);
  for (const f of r.folders || []) await setFolder(f);
  for (const d of r.docs || []) openDoc(d);
  for (const e of r.errors || []) toast(e, true);
}

async function openFileDialog() {
  if (!NATIVE) {
    const p = await promptText({ title: "Open File", label: "Path to the file (this page isn't running in the app window, so there's no file dialog)", value: S.folder ? S.folder + "/" : S.home + "/" });
    if (p) await openPath(p, { create: true });
    return;
  }
  const r = await post("/dialog/open", { dir: dirForDialogs() });
  for (const p of r.paths || []) await openPath(p);
}
async function openFolderDialog() {
  let path;
  if (!NATIVE) path = await promptText({ title: "Open Folder", label: "Path to the folder", value: S.folder || S.home });
  else path = (await post("/dialog/folder", { dir: S.folder || S.home })).path;
  if (path) await setFolder(path);
}
function dirForDialogs() {
  return (S.active && S.active.path && dirname(S.active.path)) || S.folder || null;
}

function rememberRecent(path) {
  if (!path) return;
  S.recent = [path, ...S.recent.filter((p) => p !== path)].slice(0, 20);
}
function forgetRecent(path) {
  S.recent = S.recent.filter((p) => p !== path);
  api("recent_remove", { path });
}

/** Saves a tab. Returns true when it's on disk. */
async function saveTab(tab = S.active, { as = false } = {}) {
  if (!tab) return false;
  let path = tab.path;
  if (!path || as) {
    const name = tab.path ? basename(tab.path) : suggestName(tab);
    if (!NATIVE) path = await promptText({ title: "Save As", label: "Save to this path", value: (dirForDialogs() || S.home) + "/" + name });
    else path = (await post("/dialog/save", { name, dir: tab.path ? dirname(tab.path) : dirForDialogs() })).path;
    if (!path) return false;
  }
  if (prefs.trimTrailing || prefs.finalNewline) {
    const b = tab.buf;
    if (prefs.trimTrailing) {
      const items = [];
      b.doc.lines.forEach((l, i) => {
        const m = /[ \t]+$/.exec(l);
        if (m && !(b.sels.some((s) => s.head.line === i && s.head.ch >= m.index) && tab === S.active)) items.push({ from: AxnCore.P(i, m.index), to: AxnCore.P(i, l.length), text: "" });
      });
      if (items.length) b.apply(items, "trim", { mapSels: true, noMerge: true });
    }
    if (prefs.finalNewline && b.doc.lines[b.doc.size - 1] !== "") b.apply([{ from: b.doc.end(), to: b.doc.end(), text: "\n" }], "finalnewline", { mapSels: true, noMerge: true });
  }
  const id = tab.buf.topId();
  tab.saving = true;
  let r = await api("save", { path, text: tab.buf.doc.text(), encoding: tab.encoding, eol: tab.eol });
  if (!r.ok && r.unencodable) {
    const choice = await ask({ title: "Save with UTF-8?", body: r.error, buttons: [{ label: "Cancel", value: null, cancel: true }, { label: "Save as UTF-8", value: "utf8", primary: true }] });
    if (choice === "utf8") {
      tab.encoding = "utf-8";
      r = await api("save", { path, text: tab.buf.doc.text(), encoding: tab.encoding, eol: tab.eol });
    } else {
      tab.saving = false;
      return false;
    }
  }
  tab.saving = false;
  if (!r.ok) {
    if (r.readonly) {
      const c = await ask({ title: "Couldn't save", body: r.error, buttons: [{ label: "Cancel", value: null, cancel: true }, { label: "Save As…", value: "as", primary: true }] });
      return c === "as" ? saveTab(tab, { as: true }) : false;
    }
    await alertBox("Couldn't save", r.error);
    return false;
  }
  const renamed = tab.path !== r.path;
  tab.path = r.path;
  tab.name = r.name || basename(r.path);
  tab.mtime = r.mtime;
  tab.size = r.size;
  tab.missing = false;
  tab.conflict = false;
  tab.untitled = 0;
  tab.readonlyFile = false;
  tab.buf.cleanId = id;
  tab.lastDirty = tab.buf.dirty;
  if (renamed && !tab.langLocked) {
    const id2 = LG.detect(tab.name, firstLine(tab.buf.doc.lines[0]));
    if (id2 !== tab.langId) setLang(tab, id2, false);
  }
  rememberRecent(tab.path);
  status(`Saved ${tab.name}`);
  if (S.folder && tab.path.startsWith(S.folder + "/")) refreshTree();
  renderTabs();
  renderChrome();
  renderOpenEditors();
  sessionDirty();
  return true;
}
function suggestName(tab) {
  const lang = langOf(tab);
  const ext = lang.id === "plaintext" ? ".txt" : (lang.exts && lang.exts[0]) || ".txt";
  const first = tab.buf.doc.lines.find((l) => l.trim()) || "";
  let stem = first.trim().replace(/[\\/:*?"<>|#]+/g, " ").replace(/\s+/g, " ").slice(0, 40).trim();
  if (!stem || lang.id !== "plaintext") stem = tab.name.replace(/\.[^.]+$/, "");
  return stem + ext;
}
async function saveAll() {
  for (const t of S.tabs) if (t.buf.dirty && t.path) await saveTab(t);
  const untitled = S.tabs.filter((t) => t.buf.dirty && !t.path);
  for (const t of untitled) {
    activate(t);
    if (!(await saveTab(t))) break;
  }
}

async function reloadTab(tab = S.active, { quiet = false, encoding = null } = {}) {
  if (!tab || !tab.path) return;
  if (tab.buf.dirty && !quiet && !encoding) {
    const c = await ask({ title: "Reload from disk?", body: `“${tab.name}” has unsaved changes. Reloading replaces them with the file on disk. You can still undo it.`, buttons: [{ label: "Cancel", value: null, cancel: true }, { label: "Reload", value: "ok", primary: true }] });
    if (!c) return;
  }
  const r = await api("open", encoding ? { path: tab.path, encoding } : { path: tab.path });
  if (!r.ok) return toast(r.error, true);
  const d = r.doc;
  const head = tab.buf.main.head;
  if (d.text !== tab.buf.doc.text()) tab.buf.setText(d.text, "reload");
  tab.buf.setSels([AxnCore.S(tab.buf.doc.clip(head))]);
  tab.buf.markClean();
  tab.lastDirty = false;
  tab.encoding = d.encoding;
  if (d.eolDetected) tab.eol = d.eol;
  tab.mtime = d.mtime;
  tab.size = d.size;
  tab.conflict = false;
  tab.missing = false;
  tab.notices = tab.notices.filter((n) => !n.lossy);
  if (d.lossy) tab.notices.push({ kind: "warn", lossy: true, text: `Some bytes aren't valid ${encName(d.encoding)}.`, action: ["Reopen with Encoding…", () => pickEncoding(tab, "reopen")] });
  if (tab === S.active) view.afterMove(false);
  if (quiet) status(`Reloaded ${tab.name} (it changed on disk)`);
  renderTabs();
  renderChrome();
}

// Files changed or deleted by other programs.
let checking = false;
async function checkDisk() {
  if (checking || document.hidden) return;
  const withPath = S.tabs.filter((t) => t.path && !t.saving);
  if (!withPath.length) return;
  checking = true;
  try {
    const r = await api("stat", { paths: withPath.map((t) => t.path) });
    if (!r.ok) return;
    let changed = false;
    for (const t of withPath) {
      const f = r.files[t.path];
      if (!f || t.saving) continue;
      if (!f.exists) {
        if (!t.missing && t.mtime) {
          t.missing = true;
          changed = true;
        }
        continue;
      }
      if (t.missing) {
        t.missing = false;
        changed = true;
      }
      if (t.mtime && Math.abs(f.mtime - t.mtime) > 0.5) {
        if (!t.buf.dirty) await reloadTab(t, { quiet: true });
        else if (!t.conflict) {
          t.conflict = true;
          changed = true;
        }
      }
    }
    if (changed) {
      renderTabs();
      renderChrome();
    }
  } finally {
    checking = false;
  }
}

async function setFolder(path) {
  if (!path) {
    S.folder = null;
    TREE.children.clear();
    TREE.expanded.clear();
    renderTree();
    renderChrome();
    sessionDirty();
    return;
  }
  const r = await api("list_dir", { path });
  if (!r.ok) return toast(r.error, true);
  S.folder = path.replace(/\/+$/, "") || "/";
  S.recentFolders = [S.folder, ...S.recentFolders.filter((p) => p !== S.folder)].slice(0, 10);
  api("recent_folder_add", { path: S.folder });
  TREE.children.clear();
  TREE.children.set(S.folder, r.entries);
  TREE.expanded = new Set([S.folder]);
  S.folderFiles = null;
  if (!prefs.sidebar && prefs.layout === "code") setPref("sidebar", true);
  showView("explorer", false);
  renderTree();
  renderChrome();
  sessionDirty();
}

// ------------------------------------------------------------------ session (tabs survive restarts)
let sessionTimer = 0, sessionPending = false, sessionBuffers = new Set(), sessionSaving = null;
function sessionDirty(soft = false) {
  if (!S.primary || !booted) return;
  if (!soft) sessionPending = true;
  clearTimeout(sessionTimer);
  sessionTimer = setTimeout(saveSession, soft ? 1500 : 700);
  syncState();
}
async function saveSession() {
  if (!S.primary) return;
  if (sessionSaving) await sessionSaving;
  const run = async () => {
    clearTimeout(sessionTimer);
    const tabs = [], buffers = {}, keep = new Set();
    for (const t of S.tabs) {
      const b = t.buf;
      const needText = !t.discarded && (b.dirty || (!t.path && !untitledPristine(t)) || t.missing);
      const e = {
        id: t.id, path: t.path, name: t.name, untitled: t.untitled, dirty: b.dirty && !t.discarded, encoding: t.encoding, eol: t.eol, mtime: t.mtime,
        lang: t.langLocked ? t.langId : null, tabSize: t.tabSize, insertSpaces: t.insertSpaces, readonly: b.readonly,
        sels: b.sels.slice(0, 50).map((s) => [s.anchor.line, s.anchor.ch, s.head.line, s.head.ch]), primary: b.primary,
        scroll: t === S.active ? { top: view.scroller.scrollTop, left: view.scroller.scrollLeft } : t.scroll,
        bookmarks: [...t.bookmarks], folds: t.layout.folds,
      };
      if (needText && prefs.restoreSession) {
        e.buffer = t.id;
        keep.add(t.id);
        if (t.sessionVersion !== b.version || !sessionBuffers.has(t.id)) {
          buffers[t.id] = b.doc.text();
          t.sessionVersion = b.version;
        }
      }
      tabs.push(e);
    }
    const drop = [...sessionBuffers].filter((id) => !keep.has(id));
    const session = {
      version: 1, tabs: prefs.restoreSession ? tabs : [], active: S.tabs.indexOf(S.active), folder: S.folder, expanded: [...TREE.expanded].slice(0, 200),
      search: { query: SR.query, include: SR.include, exclude: SR.exclude },
    };
    sessionPending = false;
    // A reply that never comes mustn't leave the session (and closing) waiting forever.
    const r = await withTimeout(api("session_save", { session, buffers, drop }), 10000, { ok: false });
    if (r.ok) sessionBuffers = keep;
    else sessionPending = true;
    syncState();
  };
  sessionSaving = run();
  await sessionSaving;
  sessionSaving = null;
}

async function restoreSession(sess, buffers) {
  const want = [];
  for (const e of sess.tabs || []) {
    let tab = null;
    const text = e.buffer ? buffers[e.buffer] : undefined;
    if (text !== undefined) {
      tab = makeTab({ id: e.id, text, path: e.path, name: e.name, untitled: e.untitled, encoding: e.encoding, eol: e.eol, mtime: e.mtime, lang: e.lang, langLocked: !!e.lang });
      if (e.dirty) tab.buf.markDirty();
      tab.lastDirty = tab.buf.dirty;
      sessionBuffers.add(e.buffer);
      tab.sessionVersion = tab.buf.version;
    } else if (e.path) {
      const r = await api("open", { path: e.path });
      if (!r.ok) continue;
      tab = makeTab({ ...r.doc, id: e.id, lang: e.lang, langLocked: !!e.lang });
    } else continue;
    if (e.lang) tab.langLocked = true;
    if (e.tabSize) tab.tabSize = e.tabSize;
    if (e.insertSpaces !== undefined) tab.insertSpaces = e.insertSpaces;
    if (e.readonly) tab.buf.readonly = true;
    if (Array.isArray(e.sels) && e.sels.length) tab.buf.setSels(e.sels.map((s) => AxnCore.S(AxnCore.P(s[0], s[1]), AxnCore.P(s[2], s[3]))), e.primary || 0);
    if (e.scroll) tab.scroll = e.scroll;
    for (const l of e.bookmarks || []) if (l < tab.buf.doc.size) tab.bookmarks.add(l);
    if (e.folds && e.folds.length) tab.layout.setFolds(e.folds.filter((f) => f.end < tab.buf.doc.size));
    S.tabs.push(tab);
    want.push(e === (sess.tabs || [])[sess.active] ? tab : null);
  }
  const act = want.find(Boolean) || S.tabs[0];
  if (sess.folder) {
    const r = await api("list_dir", { path: sess.folder });
    if (r.ok) {
      S.folder = sess.folder;
      TREE.children.set(S.folder, r.entries);
      TREE.expanded = new Set([S.folder, ...(sess.expanded || [])]);
      for (const d of TREE.expanded) if (d !== S.folder) loadDir(d, false);
    }
  }
  if (sess.search) Object.assign(SR, { query: sess.search.query || "", include: sess.search.include || "", exclude: sess.search.exclude || "" });
  if (act) activate(act, { focus: true });
}

// ------------------------------------------------------------------ editing glue
function configureView() {
  const t = S.active;
  const z = prefs.zoom / 100;
  view.configure({
    fontFamily: prefs.fontFamily ? `"${prefs.fontFamily}", ${DEFAULT_FONT}` : DEFAULT_FONT,
    fontSize: Math.max(5, Math.round(prefs.fontSize * z * 10) / 10), lineHeight: prefs.lineHeight, wrap: prefs.wrap, wrapColumn: prefs.wrapColumn,
    lineNumbers: prefs.lineNumbers, folding: prefs.folding, glyphMargin: prefs.glyphMargin, minimap: prefs.minimap,
    whitespace: prefs.whitespace, eol: prefs.eol, indentGuides: prefs.indentGuides, currentLine: prefs.currentLine,
    rulers: String(prefs.rulers || "").split(/[ ,]+/).map(Number).filter((n) => n > 0), cursorStyle: prefs.cursorStyle,
    cursorBlink: prefs.cursorBlink, scrollBeyond: prefs.scrollBeyond,
  });
  void t;
}

/** Runs an edit on the active tab, then reveals the cursor and refreshes. */
function edit(fn, { reveal = true } = {}) {
  const tab = S.active;
  if (!tab) return false;
  if (tab.buf.readonly) {
    status("This tab is read-only (Edit › Read-Only)");
    return false;
  }
  const r = fn(tab.buf, tab);
  view.afterMove(reveal);
  return r;
}

function typed(text) {
  const tab = S.active;
  if (!tab) return;
  text = text.replace(/\r\n?/g, "\n");
  if (S.macro.recording) S.macro.steps.push({ t: "text", v: text });
  edit((b) => {
    const single = [...text].length === 1;
    if (S.overtype && single && text !== "\n") {
      return b.apply(b.sels.map((s) => {
        const len = b.doc.line(s.head.line).length;
        if (AxnCore.sEmpty(s) && s.head.ch < len) return { from: s.head, to: AxnCore.moveChar(b.doc, s.head, 1), text };
        return { from: AxnCore.sFrom(s), to: AxnCore.sTo(s), text };
      }), "type");
    }
    if (text === "\n") return AxnCore.newline(b, editOpts(tab));
    if (single) return AxnCore.typeChar(b, text, editOpts(tab));
    return AxnCore.insertText(b, text, "type");
  });
}

let clipMeta = null;
function selectionText(cut) {
  const b = S.active.buf;
  const doc = b.doc;
  if (b.sels.every(AxnCore.sEmpty)) {
    const lines = [...new Set(b.sels.map((s) => s.head.line))].sort((x, y) => x - y);
    const parts = lines.map((l) => doc.line(l) + "\n");
    clipMeta = { text: parts.join(""), parts, line: true };
    if (cut) edit((bb) => AxnCore.deleteLines(bb));
    return clipMeta.text;
  }
  const parts = b.sels.filter((s) => !AxnCore.sEmpty(s)).map((s) => doc.get(AxnCore.sFrom(s), AxnCore.sTo(s)));
  clipMeta = { text: parts.join("\n"), parts, line: false };
  if (cut) edit((bb) => AxnCore.insertText(bb, "", "cut"));
  return clipMeta.text;
}
async function copy(cut = false) {
  if (!S.active) return;
  const t = selectionText(cut);
  await clipSet(t);
  if (S.macro.recording) S.macro.steps.push({ t: "cmd", id: cut ? "cut" : "copy" });
}
async function paste(text) {
  if (!S.active) return;
  if (text === undefined) text = await clipGet();
  if (!text) return;
  pasteText(text);
}
function pasteText(raw) {
  const tab = S.active;
  if (!tab) return;
  const text = raw.replace(/\r\n?/g, "\n");
  if (S.macro.recording) S.macro.steps.push({ t: "paste", v: text });
  edit((b) => {
    const meta = clipMeta && clipMeta.text === text ? clipMeta : null;
    if (meta && meta.line && b.sels.every(AxnCore.sEmpty)) {
      const items = b.sels.map((s, i) => ({ from: AxnCore.P(s.head.line, 0), to: AxnCore.P(s.head.line, 0), text: meta.parts.length === b.sels.length ? meta.parts[i] : text }));
      return b.apply(items, "paste", { mapSels: true, noMerge: true });
    }
    if (b.sels.length > 1) {
      const parts = meta && meta.parts.length === b.sels.length ? meta.parts : text.split("\n");
      if (parts.length === b.sels.length) return AxnCore.insertText(b, parts, "paste");
    }
    return AxnCore.insertText(b, text, "paste");
  });
}

/** Moves every cursor with fn(sel) → pos or {pos, goal, bias}; extend keeps anchors. */
function move(fn, extend = false) {
  const tab = S.active;
  if (!tab) return;
  const b = tab.buf;
  b.setSels(b.sels.map((s) => {
    const r = fn(s);
    const pos = r.pos || r;
    return extend ? AxnCore.S(s.anchor, pos, r.goal ?? null, r.bias || 0) : AxnCore.S(pos, pos, r.goal ?? null, r.bias || 0);
  }), b.primary);
  view.afterMove(true);
}
const M = AxnCore;
function moveH(dir, extend, word) {
  const tab = S.active;
  if (!tab) return;
  const doc = tab.buf.doc;
  move((s) => {
    if (!extend && !M.sEmpty(s) && !word) return dir < 0 ? M.sFrom(s) : M.sTo(s);
    return word ? M.moveWord(doc, s.head, dir, wordRe(tab)) : M.moveChar(doc, s.head, dir);
  }, extend);
}
function moveV(rows, extend) {
  const tab = S.active;
  if (!tab) return;
  move((s) => {
    if (!extend && !M.sEmpty(s) && Math.abs(rows) === 1) {
      const edge = rows < 0 ? M.sFrom(s) : M.sTo(s);
      return M.moveVertical(tab.layout, M.S(edge), rows);
    }
    return M.moveVertical(tab.layout, s, rows);
  }, extend);
}
function boxExtend(dl, dc) {
  const tab = S.active;
  const b = tab.buf;
  let box = S.box;
  if (!box || box.tab !== tab || box.sels !== b.sels) {
    const h = b.main.head;
    const col = M.vwidth(b.doc.line(h.line), 0, h.ch, tab.tabSize);
    box = S.box = { tab, al: h.line, ac: col, hl: h.line, hc: col };
  }
  box.hl = M.clamp(box.hl + dl, 0, b.doc.size - 1);
  box.hc = Math.max(0, box.hc + dc);
  b.setSels(M.boxSels(b.doc, tab.tabSize, box.al, box.ac, box.hl, box.hc), box.hl >= box.al ? 1e9 : 0);
  box.sels = b.sels;
  view.afterMove(true);
}

function setLang(tab, id, locked = true) {
  tab.langId = id;
  tab.langLocked = locked;
  tab.buf.hl.setLang(prefs.highlight ? id : "plaintext");
  const lang = LG.byId(id);
  if (lang.useTabs) tab.insertSpaces = false;
  if (tab === S.active) {
    view.htmlCache.clear();
    view.render();
  }
  renderTabs();
  renderChrome();
  sessionDirty();
}

// ------------------------------------------------------------------ selection side effects
let selRaf = 0, occTimer = 0;
function onSelectionChanged() {
  if (selRaf) return;
  selRaf = requestAnimationFrame(() => {
    selRaf = 0;
    const tab = S.active;
    if (!tab) return;
    const b = tab.buf;
    view.deco.bracket = prefs.matchBrackets && b.sels.length === 1 && M.sEmpty(b.main) ? matchedBracket(b) : null;
    if (F.open) updateFindCurrent();
    renderStatus();
    renderCrumbs();
    clearTimeout(occTimer);
    occTimer = setTimeout(updateOccurrences, 120);
    view.render();
    sessionDirty(true);
  });
}
function matchedBracket(b) {
  const m = M.matchBracket(b, b.head);
  return m && m.b ? m : null;
}
function updateOccurrences() {
  const tab = S.active;
  if (!tab) return;
  const b = tab.buf;
  view.deco.occ = [];
  if (prefs.occurrences && b.sels.length === 1) {
    const s = b.main;
    let q = null, word = false;
    if (!M.sEmpty(s)) {
      const f = M.sFrom(s), t = M.sTo(s);
      if (f.line === t.line && t.ch - f.ch <= 200) {
        q = b.doc.get(f, t);
        if (!q.trim()) q = null;
      }
    } else {
      const w = M.wordAt(b.doc.line(s.head.line), s.head.ch, wordRe(tab));
      if (w) {
        q = b.doc.line(s.head.line).slice(w[0], w[1]);
        word = true;
      }
    }
    if (q) {
      const cq = M.compileQuery(q, { caseSensitive: true, wholeWord: word });
      const from = Math.max(0, s.head.line - 400), to = Math.min(b.doc.size - 1, s.head.line + 400);
      if (cq && !cq.error) {
        const all = M.findAll(b.doc, cq, { limit: 3000, range: { from: M.P(from, 0), to: M.P(to, b.doc.line(to).length) } });
        view.deco.occ = all.length > 1 ? all.filter((m) => !(M.peq(m.from, M.sFrom(s)) && M.peq(m.to, M.sTo(s))) || word) : [];
      }
    }
  }
  view.render();
}

let idleTimer = 0;
function scheduleIdle() {
  if (idleTimer) return;
  idleTimer = setTimeout(function work() {
    idleTimer = 0;
    const tab = S.active;
    if (!tab || tab.buf.hl.done) return;
    tab.buf.hl.ensure(tab.buf.doc.size, Date.now() + 12);
    view.htmlCache.clear();
    view.render();
    if (!tab.buf.hl.done) idleTimer = setTimeout(work, 30);
  }, 60);
}

// ------------------------------------------------------------------ bookmarks and folding
function toggleBookmark(line = S.active.buf.head.line) {
  const t = S.active;
  if (t.bookmarks.has(line)) t.bookmarks.delete(line);
  else t.bookmarks.add(line);
  view.render();
  renderBookmarks();
  sessionDirty();
}
function gotoBookmark(dir) {
  const t = S.active;
  if (!t || !t.bookmarks.size) return status("No bookmarks in this tab (Ctrl+F2 adds one)");
  const lines = [...t.bookmarks].sort((a, b) => a - b);
  const cur = t.buf.head.line;
  const next = dir > 0 ? lines.find((l) => l > cur) ?? lines[0] : [...lines].reverse().find((l) => l < cur) ?? lines[lines.length - 1];
  t.buf.setSels([M.S(M.P(next, 0))]);
  view.afterMove(false);
  view.reveal(t.buf.main.head, 0, "center");
}
function toggleFoldAt(line) {
  const t = S.active;
  if (!t) return;
  if (t.layout.foldAt(line)) t.layout.unfold(line);
  else {
    const r = M.foldRange(t.buf, line, langOf(t).fold || "indent", t.tabSize);
    if (!r) return;
    t.layout.fold(r.start, r.end);
    moveCursorsOutOfFolds(t);
  }
  view.afterMove(false);
  sessionDirty();
}
function moveCursorsOutOfFolds(t) {
  const b = t.buf;
  if (!b.sels.some((s) => t.layout.hiddenBy(s.head.line))) return;
  b.setSels(b.sels.map((s) => {
    const f = t.layout.hiddenBy(s.head.line);
    return f ? M.S(M.P(f.start, b.doc.line(f.start).length)) : s;
  }));
}
function foldCursor(fold) {
  const t = S.active;
  const mode = langOf(t).fold || "indent";
  const line = t.buf.head.line;
  if (!fold) {
    for (const f of [...t.layout.folds]) if (f.start <= line && line <= f.end) t.layout.unfold(f.start);
  } else {
    // The innermost block containing the cursor.
    for (let l = line; l >= 0 && l > line - 5000; l--) {
      if (t.layout.foldAt(l)) continue;
      if (!M.canFold(t.buf, l, mode, t.tabSize)) continue;
      const r = M.foldRange(t.buf, l, mode, t.tabSize);
      if (r && r.end >= line) {
        t.layout.fold(r.start, r.end);
        break;
      }
    }
    moveCursorsOutOfFolds(t);
  }
  view.afterMove(false);
  sessionDirty();
}
function foldAll(level = 0) {
  const t = S.active;
  const all = M.allFolds(t.buf, langOf(t).fold || "indent", t.tabSize, level);
  t.layout.setFolds(level ? [...t.layout.folds, ...all.filter((f) => !t.layout.foldAt(f.start))] : all);
  moveCursorsOutOfFolds(t);
  view.afterMove(false);
  sessionDirty();
}
function unfoldAll() {
  S.active.layout.setFolds([]);
  view.afterMove(false);
  sessionDirty();
}

// ------------------------------------------------------------------ find and replace (in the editor)
const F = { open: false, replace: false, cq: null, matches: [], version: -1, timer: 0, inSel: null };
const findInput = $("#findInput"), replaceInput = $("#replaceInput");
function openFind(replace = false) {
  const tab = S.active;
  if (!tab) return;
  const b = tab.buf;
  const s = b.main;
  let seed = null;
  if (!M.sEmpty(s) && M.sFrom(s).line === M.sTo(s).line) seed = b.doc.get(M.sFrom(s), M.sTo(s));
  else if (M.sEmpty(s)) {
    const w = M.wordAt(b.doc.line(s.head.line), s.head.ch, wordRe(tab));
    if (w && !findInput.value) seed = b.doc.line(s.head.line).slice(w[0], w[1]);
  }
  if (seed && (!F.inSel || replace)) findInput.value = prefs.findRegex ? M.escapeRe(seed) : seed;
  F.open = true;
  F.replace = replace || F.replace && replace !== false;
  $("#find").hidden = false;
  $("#replaceRow").hidden = !F.replace;
  $("#findToggleIcon").setAttribute("d", F.replace ? "m4 6 4 4 4-4" : "m6 4 4 4-4 4");
  syncFindOpts();
  F.version = -1;
  refreshFind();
  const target = replace && findInput.value ? replaceInput : findInput;
  target.focus();
  target.select();
}
function closeFind() {
  F.open = false;
  F.inSel = null;
  $("#findInSel").classList.remove("on");
  $("#find").hidden = true;
  view.deco.find = [];
  view.deco.current = -1;
  view.render();
  view.focus();
}
function syncFindOpts() {
  $("#findCase").classList.toggle("on", prefs.findCase);
  $("#findWord").classList.toggle("on", prefs.findWord);
  $("#findRegex").classList.toggle("on", prefs.findRegex);
}
function scheduleFind() {
  clearTimeout(F.timer);
  F.timer = setTimeout(refreshFind, 120);
}
function refreshFind() {
  const tab = S.active;
  if (!tab) return;
  const q = findInput.value;
  const cq = M.compileQuery(q, { regex: prefs.findRegex, caseSensitive: prefs.findCase, wholeWord: prefs.findWord });
  $("#findInput").parentElement.classList.toggle("bad", !!(cq && cq.error));
  F.cq = cq && !cq.error ? cq : null;
  F.matches = F.cq ? M.findAll(tab.buf.doc, F.cq, { limit: 20000, range: F.inSel }) : [];
  F.version = tab.buf.version;
  view.deco.find = F.matches;
  updateFindCurrent();
  view.render();
}
function updateFindCurrent() {
  const tab = S.active;
  const count = $("#findCount");
  if (!F.open) return;
  if (F.cq === null && findInput.value && M.compileQuery(findInput.value, { regex: prefs.findRegex })?.error) {
    count.textContent = "Invalid pattern";
    count.className = "findCount none";
    return;
  }
  const s = tab.buf.main;
  let cur = -1;
  for (let i = 0; i < F.matches.length; i++) {
    if (M.peq(F.matches[i].from, M.sFrom(s)) && M.peq(F.matches[i].to, M.sTo(s))) {
      cur = i;
      break;
    }
  }
  view.deco.current = cur;
  const n = F.matches.length;
  count.className = "findCount" + (findInput.value && !n ? " none" : "");
  count.textContent = !findInput.value ? "No results" : !n ? "No results" : `${cur >= 0 ? (cur + 1).toLocaleString() : "?"} of ${n >= 20000 ? "20,000+" : n.toLocaleString()}`;
}
function findStep(dir) {
  const tab = S.active;
  if (!tab) return;
  if (!F.cq) {
    if (!findInput.value) return openFind();
    refreshFind();
    if (!F.cq) return;
  }
  if (F.version !== tab.buf.version) refreshFind();
  const list = F.matches;
  if (!list.length) return status(`No results for “${findInput.value}”`);
  const s = tab.buf.main;
  let i;
  if (dir > 0) {
    i = list.findIndex((m) => M.cmp(m.from, M.sTo(s)) >= 0 && !(M.peq(m.from, M.sFrom(s)) && M.peq(m.to, M.sTo(s)) && !M.sEmpty(s)));
    if (i < 0) {
      i = 0;
      status("Search passed the end of the document and continued from the top");
    }
  } else {
    i = -1;
    for (let k = list.length - 1; k >= 0; k--) if (M.cmp(list[k].from, M.sFrom(s)) < 0) { i = k; break; }
    if (i < 0) {
      i = list.length - 1;
      status("Search passed the top of the document and continued from the end");
    }
  }
  selectMatch(list[i]);
}
function selectMatch(m) {
  const b = S.active.buf;
  b.setSels([M.S(m.from, m.to)]);
  view.afterMove(false);
  view.reveal(m.from, 0, isVisible(m.from) ? "nearest" : "center");
  view.reveal(m.to);
  updateFindCurrent();
}
function isVisible(p) {
  const c = view.coords(p);
  return c.y >= view.scroller.scrollTop && c.y + view.lh <= view.scroller.scrollTop + view.scroller.clientHeight;
}
function replaceOne() {
  const tab = S.active;
  if (!F.cq) return;
  if (F.version !== tab.buf.version) refreshFind();
  const s = tab.buf.main;
  const m = F.matches.find((x) => M.peq(x.from, M.sFrom(s)) && M.peq(x.to, M.sTo(s)));
  if (m) {
    const text = F.cq.regex ? M.expandReplace(replaceInput.value, m.m) : replaceInput.value;
    edit((b) => b.apply([{ from: m.from, to: m.to, text }], "replace", { noMerge: true }));
    refreshFind();
  }
  findStep(1);
}
function replaceAllFind() {
  const tab = S.active;
  if (!F.cq) return;
  refreshFind();
  const n = F.matches.length;
  if (!n) return status("Nothing to replace");
  edit((b) => M.replaceAll(b, F.matches, replaceInput.value));
  refreshFind();
  status(`Replaced ${plural(n, "occurrence")}`);
}
function selectAllMatches() {
  if (!F.cq) return;
  refreshFind();
  if (!F.matches.length) return;
  S.active.buf.setSels(F.matches.map((m) => M.S(m.from, m.to)), 0);
  view.focus();
  view.afterMove(true);
}
function bindFind() {
  findInput.addEventListener("input", () => {
    refreshFind();
    const tab = S.active;
    if (!F.matches.length || !tab) return;
    const s = tab.buf.main;
    // Like VS Code: typing in Find jumps to the nearest match at or after the cursor.
    const m = F.matches.find((x) => M.cmp(x.from, M.sFrom(s)) >= 0) || F.matches[0];
    const b = tab.buf;
    b.setSels([M.S(m.from, m.to)]);
    view.reveal(m.from, 0, isVisible(m.from) ? "nearest" : "center");
    view.render();
    updateFindCurrent();
  });
  const keys = (e) => {
    const k = keyName(e);
    if (k === "Escape") return closeFind(), true;
    if (e.target === findInput && k === "Enter") return findStep(1), true;
    if (e.target === findInput && k === "Shift+Enter") return findStep(-1), true;
    if (e.target === findInput && k === "Alt+Enter") return selectAllMatches(), true;
    if (e.target === replaceInput && k === "Enter") return replaceOne(), true;
    if (k === "Ctrl+Alt+Enter") return replaceAllFind(), true;
    if (k === "Alt+C") return toggleFindOpt("findCase"), true;
    if (k === "Alt+W") return toggleFindOpt("findWord"), true;
    if (k === "Alt+R") return toggleFindOpt("findRegex"), true;
    if (k === "Alt+L") return toggleFindInSel(), true;
    if (k === "F3") return findStep(1), true;
    if (k === "Shift+F3") return findStep(-1), true;
    if (k === "Tab" && e.target === findInput && F.replace) return replaceInput.focus(), true;
    if (k === "Shift+Tab" && e.target === replaceInput) return findInput.focus(), true;
    return false;
  };
  for (const inp of [findInput, replaceInput]) {
    inp.addEventListener("keydown", (e) => {
      if (keys(e)) {
        e.preventDefault();
        e.stopPropagation();
      }
    });
  }
  $("#findCase").onclick = () => toggleFindOpt("findCase");
  $("#findWord").onclick = () => toggleFindOpt("findWord");
  $("#findRegex").onclick = () => toggleFindOpt("findRegex");
  $("#findNext").onclick = () => findStep(1);
  $("#findPrev").onclick = () => findStep(-1);
  $("#findClose").onclick = closeFind;
  $("#findInSel").onclick = toggleFindInSel;
  $("#findAll").onclick = findAllInDocument;
  $("#findToggle").onclick = () => openFind(!F.replace);
  $("#replaceOne").onclick = replaceOne;
  $("#replaceAllBtn").onclick = replaceAllFind;
}
function toggleFindOpt(k) {
  prefs[k] = !prefs[k];
  savePrefs();
  syncFindOpts();
  refreshFind();
}
function toggleFindInSel() {
  const b = S.active.buf;
  if (F.inSel) F.inSel = null;
  else if (!M.sEmpty(b.main)) F.inSel = { from: M.sFrom(b.main), to: M.sTo(b.main) };
  $("#findInSel").classList.toggle("on", !!F.inSel);
  refreshFind();
}
function findAllInDocument() {
  refreshFind();
  const t = S.active;
  if (!F.matches.length) return status("No results");
  SR.query = findInput.value;
  SR.results = [{ path: t.path || t.id, rel: t.path ? relPath(t.path) : t.name, tab: t, matches: F.matches.slice(0, 5000).map((m) => ({ line: m.from.line, col: m.from.ch, text: t.buf.doc.line(m.from.line).slice(0, 400), ranges: [[m.from.ch, m.from.line === m.to.line ? m.to.ch : t.buf.doc.line(m.from.line).length]], to: m.to })) }];
  SR.summary = `${plural(F.matches.length, "result")} in ${t.name}`;
  SR.scope = "document";
  if (!prefs.sidebar) setPref("sidebar", true);
  showView("search", false);
  renderSearch();
}

// ------------------------------------------------------------------ explorer
const TREE = { children: new Map(), expanded: new Set(), selected: null, loading: new Set() };
async function loadDir(path, render = true) {
  if (TREE.loading.has(path)) return;
  TREE.loading.add(path);
  const r = await api("list_dir", { path });
  TREE.loading.delete(path);
  TREE.children.set(path, r.ok ? r.entries : []);
  if (render) renderTree();
}
async function refreshTree() {
  if (!S.folder) return;
  const dirs = [...TREE.expanded];
  await Promise.all(dirs.map((d) => loadDir(d, false)));
  S.folderFiles = null;
  renderTree();
}
const HIDDEN_NAMES = new Set([".git", ".hg", ".svn", ".DS_Store"]);
function renderTree() {
  const box = $("#tree");
  $("#folderName").textContent = S.folder ? basename(S.folder) || S.folder : "No Folder Opened";
  $("#folderHead").title = S.folder || "";
  box.innerHTML = "";
  if (!S.folder) {
    const recent = S.recentFolders.slice(0, 5);
    box.append(el("div", { class: "empty" }, "You haven't opened a folder yet.",
      el("button", { class: "btn primary", on: { click: () => run("openFolder") } }, "Open Folder"),
      recent.length ? el("div", { style: "margin-top:14px" }, ...recent.map((p) => el("button", { class: "item", style: "width:100%;border:0;background:none;padding-left:0", title: p, on: { click: () => setFolder(p) } }, el("span", { html: svg("folder", "folderIcon") }), el("span", { class: "name" }, basename(p)), el("span", { class: "desc" }, tildePath(dirname(p)))))) : null));
    return;
  }
  const activePath = S.active && S.active.path;
  const frag = document.createDocumentFragment();
  let count = 0;
  const walk = (dir, depth) => {
    const kids = TREE.children.get(dir);
    if (!kids) {
      if (!TREE.loading.has(dir)) loadDir(dir);
      return;
    }
    for (const e of kids) {
      if (HIDDEN_NAMES.has(e.name)) continue;
      if (++count > 20000) return;
      const p = dir === "/" ? "/" + e.name : dir + "/" + e.name;
      const open = e.dir && TREE.expanded.has(p);
      const row = el("div", { class: `item${p === activePath ? " on" : ""}${p === TREE.selected ? " sel" : ""}`, style: `padding-left:${8 + depth * 14}px`, title: p, "data-path": p, "data-dir": e.dir ? "1" : "" });
      if (e.dir) row.innerHTML = `<svg class="chev${open ? "" : " closed"}" viewBox="0 0 16 16"><path d="m4 6 4 4 4-4"/></svg>`;
      else row.innerHTML = `<span style="width:14px;flex:none"></span>${fileIconHtml(LG.detect(e.name, ""), e.name)}`;
      row.append(el("span", { class: "name" }, e.name));
      frag.append(row);
      if (open) walk(p, depth + 1);
    }
  };
  walk(S.folder, 0);
  box.append(frag);
  if (!count) box.append(el("div", { class: "empty" }, "This folder is empty."));
}
function bindTree() {
  const box = $("#tree");
  box.addEventListener("click", (e) => {
    const row = e.target.closest(".item[data-path]");
    if (!row) return;
    const p = row.dataset.path;
    TREE.selected = p;
    if (row.dataset.dir) {
      if (TREE.expanded.has(p)) TREE.expanded.delete(p);
      else {
        TREE.expanded.add(p);
        if (!TREE.children.has(p)) loadDir(p);
      }
      renderTree();
      sessionDirty(true);
    } else openPath(p);
  });
  box.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    const row = e.target.closest(".item[data-path]");
    const p = row ? row.dataset.path : S.folder;
    if (!p) return;
    TREE.selected = p;
    renderTree();
    const isDir = row ? !!row.dataset.dir : true;
    const dir = isDir ? p : dirname(p);
    showMenu([
      { label: "New File…", run: () => fsNew(dir, false) },
      { label: "New Folder…", run: () => fsNew(dir, true) },
      "-",
      !isDir && { label: "Open", run: () => openPath(p) },
      !isDir && { label: "Open in New Window", run: () => post("/spawn", { path: p }) },
      { label: "Reveal in File Manager", run: () => api("reveal", { path: p }) },
      { label: "Open in Terminal", run: () => openTerminal(dir) },
      isDir && { label: "Find in Folder…", run: () => findInFolder(p) },
      "-",
      { label: "Copy Path", run: () => clipSet(p) },
      { label: "Copy Relative Path", run: () => clipSet(relPath(p)) },
      "-",
      row && { label: "Rename…", key: "F2", run: () => fsRename(p) },
      row && { label: "Move to Trash", run: () => fsTrash(p) },
    ].filter((x) => x !== false && x !== null && x !== undefined), e.clientX, e.clientY);
  });
  box.addEventListener("keydown", (e) => {
    if (e.key === "F2" && TREE.selected) fsRename(TREE.selected);
    if (e.key === "Delete" && TREE.selected) fsTrash(TREE.selected);
  });
  $("#exNewFile").onclick = () => S.folder ? fsNew(selectedDir(), false) : run("newTab");
  $("#exNewFolder").onclick = () => S.folder && fsNew(selectedDir(), true);
  $("#exRefresh").onclick = () => refreshTree();
  $("#exCollapse").onclick = () => {
    TREE.expanded = new Set(S.folder ? [S.folder] : []);
    renderTree();
  };
  $("#folderHead").onclick = () => $("#folderSection").classList.toggle("collapsed");
  $("#openEditorsHead").onclick = () => {
    prefs.openEditors = !prefs.openEditors;
    savePrefs();
    renderOpenEditors();
  };
}
function selectedDir() {
  const p = TREE.selected;
  if (!p || !S.folder) return S.folder;
  const parent = dirname(p);
  const kids = TREE.children.get(parent) || [];
  const e = kids.find((k) => k.name === basename(p));
  return e && e.dir ? p : parent;
}
async function fsNew(dir, folder) {
  const name = await promptText({ title: folder ? "New Folder" : "New File", label: `Name, in ${tildePath(dir)}`, value: "" });
  if (!name) return;
  const path = dir.replace(/\/$/, "") + "/" + name;
  const r = await api(folder ? "fs_new_folder" : "fs_new_file", { path });
  if (!r.ok) return alertBox("Couldn't create it", r.error);
  TREE.expanded.add(dir);
  await refreshTree();
  if (!folder) openPath(r.path || path);
}
async function fsRename(p) {
  const old = basename(p);
  const name = await promptText({ title: "Rename", label: `New name for “${old}”`, value: old, selectStem: true });
  if (!name || name === old) return;
  const to = dirname(p) + "/" + name;
  const r = await api("fs_rename", { from: p, to });
  if (!r.ok) return alertBox("Couldn't rename it", r.error);
  for (const t of S.tabs) {
    if (t.path === p || (t.path && t.path.startsWith(p + "/"))) {
      t.path = r.path + t.path.slice(p.length);
      t.name = basename(t.path);
      if (!t.langLocked) setLang(t, LG.detect(t.name, firstLine(t.buf.doc.lines[0])), false);
    }
  }
  if (TREE.expanded.has(p)) {
    TREE.expanded.delete(p);
    TREE.expanded.add(r.path);
  }
  TREE.selected = r.path;
  await refreshTree();
  renderTabs();
  renderChrome();
  sessionDirty();
}
async function fsTrash(p) {
  const c = await ask({ title: "Move to Trash?", body: `“${basename(p)}” will be moved to the Trash. You can restore it from there.`, buttons: [{ label: "Cancel", value: null, cancel: true }, { label: "Move to Trash", value: "ok", primary: true }] });
  if (!c) return;
  const r = await api("fs_trash", { path: p });
  if (!r.ok) return alertBox("Couldn't move it to the Trash", r.error);
  for (const t of S.tabs) if (t.path === p || (t.path && t.path.startsWith(p + "/"))) t.missing = true;
  await refreshTree();
  renderTabs();
}
async function openTerminal(dir) {
  const r = await api("terminal", { path: dir });
  if (!r.ok) toast(r.error, true);
}
const relPath = (p) => (S.folder && p && p.startsWith(S.folder + "/") ? p.slice(S.folder.length + 1) : tildePath(p));
const tildePath = (p) => (p && S.home && (p === S.home || p.startsWith(S.home + "/")) ? "~" + p.slice(S.home.length) : p || "");

function renderOpenEditors() {
  const sec = $("#openEditorsSection");
  sec.classList.toggle("collapsed", !prefs.openEditors);
  const box = $("#openEditors");
  box.innerHTML = "";
  for (const t of S.tabs) {
    const row = el("div", { class: `item${t === S.active ? " on" : ""}`, style: "padding-left:22px", title: t.path || t.name });
    row.innerHTML = fileIconHtml(t.langId, t.name);
    row.append(el("span", { class: "name" }, t.name), el("span", { class: "desc" }, t.path ? tildePath(dirname(t.path)) : ""));
    const x = el("button", { class: `tb small x${t.buf.dirty ? " dirty" : ""}`, title: "Close", html: svg("close") });
    x.onclick = (e) => {
      e.stopPropagation();
      closeTab(t);
    };
    row.append(x);
    row.onclick = () => activate(t);
    box.append(row);
  }
}

// ------------------------------------------------------------------ search across files
const SR = { query: "", replace: "", include: "", exclude: "", results: [], summary: "", busy: false, collapsed: new Set(), seq: 0, scope: "folder", showReplace: false };
function findInFolder(dir) {
  if (!prefs.sidebar) setPref("sidebar", true);
  showView("search", true);
  if (dir && S.folder && dir !== S.folder) {
    SR.include = relPath(dir) + "/**";
    $("#srInclude").value = SR.include;
  }
  const t = S.active;
  if (t && !M.sEmpty(t.buf.main) && M.sFrom(t.buf.main).line === M.sTo(t.buf.main).line) {
    $("#srQuery").value = t.buf.doc.get(M.sFrom(t.buf.main), M.sTo(t.buf.main));
  }
  $("#srQuery").focus();
  $("#srQuery").select();
  runSearch();
}
let searchTimer = 0;
function scheduleSearch() {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(runSearch, 300);
}
async function runSearch() {
  SR.query = $("#srQuery").value;
  SR.include = $("#srInclude").value;
  SR.exclude = $("#srExclude").value;
  SR.replace = $("#srReplace").value;
  const seq = ++SR.seq;
  if (!SR.query) {
    SR.results = [];
    SR.summary = "";
    renderSearch();
    return;
  }
  const opts = { regex: prefs.findRegex, case: prefs.findCase, word: prefs.findWord };
  SR.busy = true;
  SR.summary = "Searching…";
  renderSearchSummary();
  if (S.folder) {
    SR.scope = "folder";
    const r = await api("search", { root: S.folder, query: SR.query, ...opts, include: SR.include, exclude: SR.exclude, useIgnore: true, maxResults: 10000 });
    if (seq !== SR.seq) return;
    SR.busy = false;
    if (!r.ok) {
      SR.results = [];
      SR.summary = r.error;
      renderSearch();
      return;
    }
    // Open tabs with unsaved changes show what's in the editor, not what's on disk.
    const results = r.results.filter((f) => !(findTab(f.path) && findTab(f.path).buf.dirty));
    const dirtyHits = searchTabs(S.tabs.filter((t) => t.buf.dirty && t.path && t.path.startsWith(S.folder + "/")));
    SR.results = [...dirtyHits, ...results].sort((a, b) => a.rel.localeCompare(b.rel));
    const n = SR.results.reduce((a, f) => a + f.matches.reduce((b, m) => b + Math.max(1, m.ranges.length), 0), 0);
    SR.summary = n ? `${plural(n, "result")} in ${plural(SR.results.length, "file")}${r.truncated ? " (stopped at the limit; refine the search)" : ""}` : "No results found. Check your exclude settings and .gitignore.";
  } else {
    SR.scope = "open";
    SR.results = searchTabs(S.tabs);
    SR.busy = false;
    const n = SR.results.reduce((a, f) => a + f.matches.reduce((b, m) => b + Math.max(1, m.ranges.length), 0), 0);
    SR.summary = n ? `${plural(n, "result")} in ${plural(SR.results.length, "open file")} (open a folder to search files on disk)` : "No results in open files. Open a folder to search files on disk.";
  }
  renderSearch();
  sessionDirty(true);
}
function searchTabs(tabs) {
  const cq = M.compileQuery(SR.query, { regex: prefs.findRegex, caseSensitive: prefs.findCase, wholeWord: prefs.findWord });
  if (!cq || cq.error) return [];
  const out = [];
  for (const t of tabs) {
    const ms = M.findAll(t.buf.doc, cq, { limit: 2000 });
    if (!ms.length) continue;
    const byLine = new Map();
    for (const m of ms) {
      if (!byLine.has(m.from.line)) byLine.set(m.from.line, { line: m.from.line, col: m.from.ch, text: t.buf.doc.line(m.from.line).slice(0, 400), ranges: [], to: m.to });
      byLine.get(m.from.line).ranges.push([m.from.ch, m.from.line === m.to.line ? m.to.ch : t.buf.doc.line(m.from.line).length]);
    }
    out.push({ path: t.path || t.id, rel: t.path ? relPath(t.path) : t.name, tab: t, matches: [...byLine.values()] });
  }
  return out;
}
function renderSearchSummary() {
  $("#srSummary").textContent = SR.summary;
}
function renderSearch() {
  renderSearchSummary();
  const box = $("#srResults");
  box.innerHTML = "";
  const frag = document.createDocumentFragment();
  let rows = 0;
  const repl = SR.showReplace && SR.replace !== undefined ? $("#srReplace").value : null;
  for (const f of SR.results) {
    const closed = SR.collapsed.has(f.path);
    const head = el("div", { class: "rfile", title: f.path });
    head.innerHTML = `<svg class="chev${closed ? " closed" : ""}" viewBox="0 0 16 16" style="width:14px;height:14px;${closed ? "transform:rotate(-90deg)" : ""}"><path d="m4 6 4 4 4-4"/></svg>${fileIconHtml(LG.detect(basename(f.rel), ""), f.rel)}`;
    head.append(el("span", { class: "name" }, basename(f.rel)), el("span", { class: "desc", style: "color:var(--faint);font-size:12px;overflow:hidden;text-overflow:ellipsis" }, dirname(f.rel) === f.rel ? "" : dirname(f.rel)), el("span", { class: "badge" }, String(f.matches.reduce((b, m) => b + Math.max(1, m.ranges.length), 0))));
    head.onclick = () => {
      if (closed) SR.collapsed.delete(f.path);
      else SR.collapsed.add(f.path);
      renderSearch();
    };
    frag.append(head);
    if (closed) continue;
    for (const m of f.matches) {
      if (++rows > 3000) break;
      const row = el("div", { class: "rmatch", title: m.text });
      let html = "", at = 0;
      const lead = m.ranges.length && m.ranges[0][0] > 60 ? m.ranges[0][0] - 30 : 0;
      for (const [s, e] of m.ranges) {
        html += esc(m.text.slice(Math.max(at, lead), s));
        if (repl !== null && repl !== "") html += `<del>${esc(m.text.slice(s, e))}</del><ins>${esc(repl)}</ins>`;
        else html += `<mark>${esc(m.text.slice(s, e))}</mark>`;
        at = e;
      }
      html += esc(m.text.slice(at));
      row.innerHTML = `<span class="lnum">${m.line + 1}</span><span class="rtext">${lead ? "…" : ""}${html}</span>`;
      row.onclick = () => openResult(f, m);
      frag.append(row);
    }
  }
  box.append(frag);
  const ab = document.querySelector('.ab[data-view="search"]');
  const total = SR.results.reduce((a, f) => a + f.matches.reduce((b, m) => b + Math.max(1, m.ranges.length), 0), 0);
  let badge = ab.querySelector(".count");
  if (total && SR.query) {
    if (!badge) ab.append((badge = el("span", { class: "count" })));
    badge.textContent = total > 999 ? "1k+" : String(total);
  } else if (badge) badge.remove();
}
async function openResult(f, m) {
  const r0 = m.ranges[0] || [m.col, m.col];
  const at = { line: m.line, ch: r0[0], to: { line: m.line, ch: r0[1] } };
  if (f.tab && S.tabs.includes(f.tab)) {
    activate(f.tab);
    f.tab.buf.setSels([M.S(M.P(at.line, at.ch), M.P(at.line, at.to.ch))]);
    view.afterMove(false);
    view.reveal(f.tab.buf.main.head, 0, "center");
    return;
  }
  const tab = await openPath(f.path);
  if (!tab) return;
  tab.buf.setSels([M.S(tab.buf.doc.clip(M.P(at.line, at.ch)), tab.buf.doc.clip(M.P(at.line, at.to.ch)))]);
  view.afterMove(false);
  view.reveal(tab.buf.main.head, 0, "center");
}
async function replaceInFiles() {
  const repl = $("#srReplace").value;
  if (!SR.results.length) return;
  const files = SR.results;
  const n = files.reduce((a, f) => a + f.matches.reduce((b, m) => b + m.ranges.length, 0), 0);
  const ok = await ask({ title: "Replace all?", body: `Replace ${plural(n, "occurrence")} across ${plural(files.length, "file")} with “${repl}”?`, note: "Files open in tabs change in the editor (unsaved); the others are rewritten on disk.", buttons: [{ label: "Cancel", value: null, cancel: true }, { label: "Replace", value: "ok", primary: true }] });
  if (!ok) return;
  const cq = M.compileQuery(SR.query, { regex: prefs.findRegex, caseSensitive: prefs.findCase, wholeWord: prefs.findWord });
  const disk = [];
  let count = 0;
  for (const f of files) {
    const t = f.tab && S.tabs.includes(f.tab) ? f.tab : findTab(f.path);
    if (t && cq && !cq.error) {
      const ms = M.findAll(t.buf.doc, cq, { limit: 200000 });
      if (ms.length) {
        if (t === S.active) edit((b) => M.replaceAll(b, ms, repl));
        else M.replaceAll(t.buf, ms, repl);
        count += ms.length;
      }
    } else if (!f.tab) disk.push(f.path);
  }
  if (disk.length) {
    const r = await api("replace_in_files", { root: S.folder, query: SR.query, regex: prefs.findRegex, case: prefs.findCase, word: prefs.findWord, include: SR.include, exclude: SR.exclude, useIgnore: true, replacement: repl, paths: disk });
    if (!r.ok) return alertBox("Couldn't replace", r.error);
    count += r.changed.reduce((a, c) => a + c.count, 0);
    if (r.skipped.length) toast(`Skipped ${plural(r.skipped.length, "file")}: ${r.skipped.slice(0, 3).map((s) => `${basename(s.path)} (${s.reason})`).join(", ")}${r.skipped.length > 3 ? "…" : ""}`, true);
    for (const c of r.changed) {
      const t = findTab(c.path);
      if (t && !t.buf.dirty) await reloadTab(t, { quiet: true });
    }
  }
  status(`Replaced ${plural(count, "occurrence")}`);
  runSearch();
}
function bindSearch() {
  const q = $("#srQuery");
  q.value = SR.query;
  $("#srInclude").value = SR.include;
  $("#srExclude").value = SR.exclude;
  for (const id of ["#srQuery", "#srInclude", "#srExclude"]) {
    $(id).addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        runSearch();
      } else if (e.key === "Escape") view.focus();
    });
  }
  q.addEventListener("input", scheduleSearch);
  $("#srReplace").addEventListener("input", () => renderSearch());
  $("#srReplace").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && e.ctrlKey && e.altKey) replaceInFiles();
  });
  const opt = (id, k) => {
    $(id).onclick = () => {
      prefs[k] = !prefs[k];
      savePrefs();
      syncSearchOpts();
      syncFindOpts();
      runSearch();
    };
  };
  opt("#srCase", "findCase");
  opt("#srWord", "findWord");
  opt("#srRegex", "findRegex");
  $("#srToggleReplace").onclick = () => {
    SR.showReplace = !SR.showReplace;
    $("#srReplaceRow").hidden = !SR.showReplace;
    $("#srToggleIcon").setAttribute("d", SR.showReplace ? "m4 6 4 4 4-4" : "m6 4 4 4-4 4");
    renderSearch();
  };
  $("#srReplaceAll").onclick = replaceInFiles;
  $("#srRefresh").onclick = runSearch;
  $("#srClear").onclick = () => {
    q.value = "";
    runSearch();
  };
  $("#srCollapse").onclick = () => {
    for (const f of SR.results) SR.collapsed.add(f.path);
    renderSearch();
  };
  syncSearchOpts();
}
function syncSearchOpts() {
  $("#srCase").classList.toggle("on", prefs.findCase);
  $("#srWord").classList.toggle("on", prefs.findWord);
  $("#srRegex").classList.toggle("on", prefs.findRegex);
}

function renderBookmarks() {
  const box = $("#bmList");
  if (!box || $("#viewBookmarks").hidden) return;
  box.innerHTML = "";
  let any = false;
  for (const t of S.tabs) {
    if (!t.bookmarks.size) continue;
    any = true;
    const head = el("div", { class: "rfile", title: t.path || t.name });
    head.innerHTML = fileIconHtml(t.langId, t.name);
    head.append(el("span", { class: "name" }, t.name), el("span", { class: "badge" }, String(t.bookmarks.size)));
    head.onclick = () => activate(t);
    box.append(head);
    for (const l of [...t.bookmarks].sort((a, b) => a - b)) {
      const row = el("div", { class: "rmatch" });
      row.innerHTML = `<span class="lnum">${l + 1}</span><span class="rtext">${esc(t.buf.doc.line(l).trim().slice(0, 200)) || "<i>empty line</i>"}</span>`;
      row.onclick = () => {
        activate(t);
        t.buf.setSels([M.S(M.P(l, 0))]);
        view.afterMove(false);
        view.reveal(t.buf.main.head, 0, "center");
      };
      box.append(row);
    }
  }
  if (!any) box.append(el("div", { class: "empty" }, "No bookmarks yet. Press Ctrl+F2 or click the margin left of the line numbers to bookmark a line; F2 and Shift+F2 jump between them."));
}

// ------------------------------------------------------------------ side bar
function showView(name, focus = true) {
  prefs.sideView = name;
  savePrefs();
  for (const v of document.querySelectorAll(".view")) v.hidden = v.dataset.view !== name;
  for (const b of document.querySelectorAll(".ab[data-view]")) b.classList.toggle("on", prefs.sidebar && b.dataset.view === name);
  if (name === "search" && focus) setTimeout(() => $("#srQuery").focus(), 0);
  if (name === "bookmarks") renderBookmarks();
}
function toggleView(name) {
  if (prefs.sidebar && prefs.sideView === name) setPref("sidebar", false);
  else {
    if (!prefs.sidebar) setPref("sidebar", true);
    showView(name);
  }
}
function bindSash() {
  const sash = $("#sash"), side = $("#sidebar");
  side.style.width = prefs.sideWidth + "px";
  sash.addEventListener("mousedown", (e) => {
    e.preventDefault();
    sash.classList.add("drag");
    const x0 = e.clientX, w0 = side.getBoundingClientRect().width;
    const mv = (ev) => {
      const w = Math.max(160, Math.min(window.innerWidth * 0.7, w0 + ev.clientX - x0));
      side.style.width = w + "px";
      prefs.sideWidth = Math.round(w);
      view.schedule();
    };
    const up = () => {
      sash.classList.remove("drag");
      savePrefs();
      document.removeEventListener("mousemove", mv);
      document.removeEventListener("mouseup", up);
    };
    document.addEventListener("mousemove", mv);
    document.addEventListener("mouseup", up);
  });
  sash.addEventListener("dblclick", () => {
    side.style.width = "260px";
    prefs.sideWidth = 260;
    savePrefs();
  });
}

// ------------------------------------------------------------------ tab bar
function fileIconHtml(langId) {
  const lang = LG.byId(langId);
  const ic = lang.icon;
  if (!ic || lang.id === "plaintext") return `<span class="fileIcon plain"></span>`;
  const c = ic.color || "#8890b5";
  const rgb = parseInt(c.slice(1).padEnd(6, "0").slice(0, 6), 16);
  const lum = (0.299 * ((rgb >> 16) & 255) + 0.587 * ((rgb >> 8) & 255) + 0.114 * (rgb & 255)) / 255;
  return `<span class="fileIcon" style="background:${c};color:${lum > 0.6 ? "#10142a" : "#fff"}" title="${esc(lang.name)}">${esc((ic.text || "").slice(0, 3))}</span>`;
}
function tabLabels() {
  const names = new Map();
  for (const t of S.tabs) names.set(t.name, (names.get(t.name) || 0) + 1);
  return (t) => (names.get(t.name) > 1 && t.path ? tildePath(dirname(t.path)).split("/").slice(-2).join("/") : "");
}
let tabDrag = null;
function renderTabs() {
  const box = $("#tabs");
  box.innerHTML = "";
  const desc = tabLabels();
  for (const t of S.tabs) {
    const cls = ["tab", t === S.active && "on", t.buf.dirty && "dirty", t.missing && "missing", (t.buf.readonly || t.readonlyFile) && "readonly"].filter(Boolean).join(" ");
    const tip = (t.path ? tildePath(t.path) : t.name) + (t.missing ? " (deleted on disk)" : "") + (t.buf.dirty ? " • unsaved" : "");
    const node = el("div", { class: cls, role: "tab", title: tip, "aria-selected": t === S.active ? "true" : "false" });
    node.innerHTML = fileIconHtml(t.langId, t.name);
    node.append(el("span", { class: "name" }, t.name));
    const d = desc(t);
    if (d) node.append(el("span", { class: "desc" }, d));
    const x = el("button", { class: "x", title: "Close (Ctrl+W)", html: svg("close") });
    x.addEventListener("mousedown", (e) => e.stopPropagation());
    x.onclick = (e) => {
      e.stopPropagation();
      closeTab(t);
    };
    node.append(x);
    node.addEventListener("mousedown", (e) => {
      if (e.button === 1) {
        e.preventDefault();
        closeTab(t);
        return;
      }
      if (e.button !== 0) return;
      if (t !== S.active) activate(t);
      tabDrag = { tab: t, x: e.clientX, moved: false };
    });
    node.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      tabMenu(t, e.clientX, e.clientY);
    });
    node.dataset.id = t.id;
    box.append(node);
  }
  const on = box.querySelector(".tab.on");
  if (on) on.scrollIntoView({ block: "nearest", inline: "nearest" });
  $("#welcome").hidden = !!S.tabs.length;
  if (!S.tabs.length) renderWelcome();
}
function bindTabDrag() {
  const box = $("#tabs");
  document.addEventListener("mousemove", (e) => {
    if (!tabDrag) return;
    if (!(e.buttons & 1)) return endDrag(e);
    if (!tabDrag.moved && Math.abs(e.clientX - tabDrag.x) < 6) return;
    tabDrag.moved = true;
    for (const n of box.children) n.classList.remove("dropBefore", "dropAfter");
    const over = [...box.children].find((n) => {
      const r = n.getBoundingClientRect();
      return e.clientX >= r.left && e.clientX < r.right;
    });
    if (over && over.dataset.id !== tabDrag.tab.id) {
      const r = over.getBoundingClientRect();
      over.classList.add(e.clientX < r.left + r.width / 2 ? "dropBefore" : "dropAfter");
    }
  });
  const endDrag = (e) => {
    if (!tabDrag) return;
    const d = tabDrag;
    tabDrag = null;
    if (!d.moved) return;
    const over = [...box.children].find((n) => {
      const r = n.getBoundingClientRect();
      return e.clientX >= r.left && e.clientX < r.right;
    });
    for (const n of box.children) n.classList.remove("dropBefore", "dropAfter");
    if (!over || over.dataset.id === d.tab.id) return;
    const target = S.tabs.find((t) => t.id === over.dataset.id);
    const r = over.getBoundingClientRect();
    S.tabs.splice(S.tabs.indexOf(d.tab), 1);
    let i = S.tabs.indexOf(target);
    if (e.clientX >= r.left + r.width / 2) i++;
    S.tabs.splice(i, 0, d.tab);
    renderTabs();
    renderOpenEditors();
    sessionDirty();
  };
  document.addEventListener("mouseup", endDrag);
  box.addEventListener("wheel", (e) => {
    if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
      box.scrollLeft += e.deltaY;
      e.preventDefault();
    }
  }, { passive: false });
  $("#tabFill").addEventListener("dblclick", () => run("newTab"));
  $("#newTabBtn").onclick = () => run("newTab");
}
function tabMenu(t, x, y) {
  const i = S.tabs.indexOf(t);
  showMenu([
    { label: "Close", key: "Ctrl+W", run: () => closeTab(t) },
    { label: "Close Others", run: () => closeTabs(S.tabs.filter((o) => o !== t)) },
    { label: "Close to the Right", run: () => closeTabs(S.tabs.slice(i + 1)), enabled: () => i < S.tabs.length - 1 },
    { label: "Close Saved", run: () => closeTabs(S.tabs.filter((o) => !o.buf.dirty)) },
    { label: "Close All", run: () => closeTabs(S.tabs) },
    "-",
    { label: "Copy Path", run: () => clipSet(t.path || ""), enabled: () => !!t.path },
    { label: "Copy Relative Path", run: () => clipSet(relPath(t.path)), enabled: () => !!t.path },
    { label: "Reveal in File Manager", run: () => api("reveal", { path: t.path }), enabled: () => !!t.path },
    { label: "Open in Terminal", run: () => openTerminal(dirname(t.path)), enabled: () => !!t.path },
    { label: "Open in New Window", run: async () => { if (!t.buf.dirty || (await closeTab(t))) post("/spawn", { path: t.path }); }, enabled: () => !!t.path },
    "-",
    { label: "Reload from Disk", run: () => reloadTab(t), enabled: () => !!t.path },
    { label: "Read-Only", checked: () => t.buf.readonly, run: () => { t.buf.readonly = !t.buf.readonly; renderTabs(); renderStatus(); sessionDirty(); } },
  ], x, y);
}

// ------------------------------------------------------------------ status bar, title, breadcrumbs, banner
let chromeRaf = 0;
function scheduleChrome() {
  if (!chromeRaf) chromeRaf = requestAnimationFrame(() => {
    chromeRaf = 0;
    renderStatus();
    syncState();
  });
}
function renderChrome() {
  renderStatus();
  renderCrumbs();
  renderBanner();
  syncState();
  const t = S.active;
  $("#ccText").textContent = S.folder ? basename(S.folder) : t ? t.name : APP;
}
function status(msg) {
  S.status = msg;
  clearTimeout(S.statusTimer);
  S.statusTimer = setTimeout(() => {
    S.status = "";
    renderStatus();
  }, 5000);
  renderStatus();
}
function renderStatus() {
  const left = $("#sbLeft"), right = $("#sbRight");
  const t = S.active;
  const item = (text, o = {}) => el(o.run ? "button" : "span", { class: "si" + (o.cls ? " " + o.cls : ""), title: o.title || "", on: o.run ? { click: o.run } : undefined, html: o.html }, text);
  left.innerHTML = "";
  right.innerHTML = "";
  if (S.macro.recording) left.append(item("● Recording macro", { cls: "rec", title: "Stop recording (Ctrl+Shift+R)", run: () => run("macroRecord") }));
  if (t) {
    const b = t.buf;
    const h = b.head;
    const col = M.vwidth(b.doc.line(h.line), 0, h.ch, t.tabSize) + 1;
    let sel = 0, selLines = 0;
    for (const s of b.sels) {
      if (M.sEmpty(s)) continue;
      sel += b.doc.get(M.sFrom(s), M.sTo(s)).length;
      selLines += M.sTo(s).line - M.sFrom(s).line + 1;
    }
    let pos = `Ln ${(h.line + 1).toLocaleString()}, Col ${col}`;
    if (b.sels.length > 1) pos += ` (${b.sels.length} selections${sel ? `, ${sel.toLocaleString()} chars` : ""})`;
    else if (sel) pos += ` (${sel.toLocaleString()} selected${selLines > 1 ? `, ${selLines} lines` : ""})`;
    left.append(item(pos, { title: "Go to Line (Ctrl+G)", run: () => run("gotoLine") }));
    if (prefs.layout === "notepadpp") left.append(item(`length: ${b.doc.charCount().toLocaleString()}   lines: ${b.doc.size.toLocaleString()}`));
    if (S.overtype) left.append(item("OVR", { cls: "accent", title: "Overtype mode (Insert to toggle)", run: () => run("toggleOvertype") }));
    if (S.columnMode) left.append(item("Column Selection", { cls: "accent", title: "Turn off column selection mode", run: () => run("toggleColumnMode") }));
    if (b.readonly || t.readonlyFile) left.append(item(b.readonly ? "Read-only" : "Read-only file", { cls: "accent", title: b.readonly ? "Turn off Read-Only (Edit menu)" : "You don't have permission to write this file", run: b.readonly ? () => run("toggleReadOnly") : undefined }));
  }
  if (S.status) left.append(item(S.status, { cls: "muted" }));
  if (t) {
    right.append(item(t.insertSpaces ? `Spaces: ${t.tabSize}` : `Tab Size: ${t.tabSize}`, { title: "Select Indentation", run: () => pickIndent(t) }));
    right.append(item(encName(t.encoding), { title: "Select Encoding", run: () => pickEncodingAction(t) }));
    right.append(item(prefs.layout === "notepad" ? eolName(t.eol) : eolShort(t.eol), { title: "Select End of Line Sequence", run: () => pickEol(t) }));
    right.append(item(langOf(t).name, { title: "Select Language Mode", run: () => pickLanguage(t) }));
  }
  right.append(item(`${prefs.zoom}%`, { title: "Zoom (Ctrl+= / Ctrl+- / Ctrl+0)", run: () => run("zoomReset") }));
  right.append(item("", { title: `Layout: ${LAYOUT_NAMES[prefs.layout]}`, html: svg("sidebar"), run: () => pickLayout() }));
}
/** Tells the app window the title, whether the session still needs writing (guard), and
 * whether closing should ask about unsaved changes (ask, with the tab names in dirty). */
function syncState(force = false) {
  const t = S.active;
  const title = t ? `${t.buf.dirty ? "• " : ""}${t.name} - ${APP}` : S.folder ? `${basename(S.folder)} - ${APP}` : APP;
  const dirtyTabs = S.tabs.filter((x) => x.buf.dirty);
  const guard = sessionPending || !!sessionSaving;
  const ask = dirtyTabs.length > 0 && !keepsUnsaved();
  const dirty = dirtyTabs.map((x) => x.name);
  document.title = title;
  const key = JSON.stringify([title, guard, ask, dirty]);
  if (!force && key === syncState.key) return Promise.resolve();
  syncState.key = key;
  return post("/state", { title, guard, ask, dirty }).catch(() => {});
}
function renderCrumbs() {
  const box = $("#crumbs");
  const t = S.active;
  box.innerHTML = "";
  box.hidden = !t || !prefs.breadcrumbs;
  if (!t) return;
  const parts = t.path ? relPath(t.path).split("/") : [t.name];
  parts.forEach((p, i) => {
    if (i) box.append(el("span", { class: "sepc" }, "›"));
    if (i === parts.length - 1) {
      const span = el("span", { class: "cur", html: fileIconHtml(t.langId, t.name) });
      span.style.display = "inline-flex";
      span.style.gap = "6px";
      span.style.alignItems = "center";
      span.append(p);
      box.append(span);
    } else box.append(el("span", {}, p));
  });
}
function renderBanner() {
  const b = $("#banner");
  const t = S.active;
  b.innerHTML = "";
  b.hidden = true;
  if (!t) return;
  const show = (kind, text, actions) => {
    b.className = "banner" + (kind === "info" ? " info" : "");
    b.append(el("span", { class: "grow" }, text));
    for (const [label, fn] of actions) b.append(el("button", { class: "btn", on: { click: fn } }, label));
    b.hidden = false;
  };
  if (t.conflict) {
    return show("warn", `“${t.name}” changed on disk while you had unsaved changes.`, [
      ["Reload from Disk", () => reloadTab(t, { quiet: true })],
      ["Keep Mine", () => { t.conflict = false; t.mtime = Date.now(); renderBanner(); }],
    ]);
  }
  if (t.missing && t.path) return show("warn", `“${t.name}” was deleted or moved on disk. Save to create it again.`, [["Save", () => saveTab(t)], ["Close Tab", () => closeTab(t, { force: true })]]);
  const n = t.notices[0];
  if (n) {
    const actions = [];
    if (n.action) actions.push(n.action);
    actions.push(["Dismiss", () => { t.notices.shift(); renderBanner(); }]);
    show(n.kind, n.text, actions);
  }
}
function renderWelcome() {
  const w = $("#welcome");
  w.innerHTML = "";
  const link = (label, key, fn, p) => el("button", { class: "link", on: { click: fn } }, label, p ? el("span", { class: "p" }, p) : null, key ? el("kbd", {}, key) : null);
  const recent = [...S.recent.slice(0, 6).map((p) => ({ p, folder: false })), ...S.recentFolders.slice(0, 4).map((p) => ({ p, folder: true }))];
  const inner = el("div", { class: "welcomeInner" },
    el("h1", { html: `<img src="logo.png" alt="">${APP}` }),
    el("p", { class: "lead" }, "Text and code, the way Notepad, Notepad++ and Visual Studio Code do it."),
    el("div", { class: "welcomeCols" },
      el("div", {},
        el("h2", {}, "Start"),
        link("New Note", "Ctrl+N", () => run("newTab")),
        link("New Code File…", "Ctrl+Alt+N", () => run("newCodeFile")),
        link("Open File…", "Ctrl+O", () => run("open")),
        link("Open Folder…", "Ctrl+K Ctrl+O", () => run("openFolder")),
        link("Command Palette…", "Ctrl+Shift+P", () => run("palette")),
        S.closed.length ? link("Reopen Closed Tab", "Ctrl+Shift+T", () => run("reopenClosed")) : null),
      el("div", {},
        el("h2", {}, "Recent"),
        ...(recent.length ? recent.map((r) => link(basename(r.p), null, () => (r.folder ? setFolder(r.p) : openPath(r.p)), tildePath(dirname(r.p)))) : [el("p", { class: "muted" }, "Files and folders you open show up here.")]))),
    el("h2", { style: "margin-top:34px" }, "Layout"),
    el("div", { class: "layoutCards" }, ...Object.entries({
      notepad: "Just the text, word wrap on, no line numbers. Like Windows Notepad.",
      notepadpp: "Toolbar, line numbers, folding and bookmarks. Like Notepad++.",
      code: "Explorer, search, minimap and command palette. Like VS Code.",
    }).map(([k, d]) => el("button", { class: `layoutCard${prefs.layout === k ? " on" : ""}`, on: { click: () => { applyLayout(k); renderWelcome(); } } }, el("b", {}, LAYOUT_NAMES[k]), el("span", {}, d)))));
  w.append(inner);
}

// ------------------------------------------------------------------ pickers (quick input)
const Q = { open: false, items: [], shown: [], index: 0, provider: null, onAccept: null, onCancel: null };
const quickInput = $("#quickInput");
function fuzzy(query, text) {
  if (!query) return { score: 0, idx: [] };
  const q = query.toLowerCase(), t = text.toLowerCase();
  const idx = [];
  let ti = 0, score = 0, prev = -2;
  for (let qi = 0; qi < q.length; qi++) {
    const c = q[qi];
    if (c === " ") continue;
    const at = t.indexOf(c, ti);
    if (at < 0) return null;
    idx.push(at);
    const boundary = at === 0 || /[\s\-_./\\:]/.test(text[at - 1]) || (text[at] !== t[at] && text[at - 1] === t[at - 1]);
    score += (at === prev + 1 ? 6 : 1) + (boundary ? 4 : 0);
    prev = at;
    ti = at + 1;
  }
  return { score: score - text.length * 0.02, idx };
}
function hl(text, idx) {
  if (!idx || !idx.length) return esc(text);
  let out = "", set = new Set(idx);
  for (let i = 0; i < text.length; i++) out += set.has(i) ? `<b>${esc(text[i])}</b>` : esc(text[i]);
  return out;
}
/** Opens the quick input. provider(query) → items [{label, desc, key, icon, run?, value, sep}]. */
function quick({ value = "", placeholder = "", provider, onAccept, onCancel }) {
  closeMenus();
  Q.open = true;
  Q.provider = provider;
  Q.onAccept = onAccept;
  Q.onCancel = onCancel;
  $("#quick").hidden = false;
  quickInput.placeholder = placeholder;
  quickInput.value = value;
  quickInput.focus();
  quickInput.setSelectionRange(value.length, value.length);
  refreshQuick();
}
function closeQuick(cancelled = true) {
  if (!Q.open) return;
  Q.open = false;
  $("#quick").hidden = true;
  if (cancelled && Q.onCancel) Q.onCancel();
  if (S.active) view.focus();
}
function refreshQuick() {
  const res = Q.provider(quickInput.value);
  Promise.resolve(res).then((items) => {
    Q.shown = items || [];
    Q.index = Math.max(0, Q.shown.findIndex((i) => !i.sep && i.active));
    if (Q.shown[Q.index] && Q.shown[Q.index].sep) Q.index = Q.shown.findIndex((i) => !i.sep);
    renderQuick();
  });
}
function renderQuick() {
  const box = $("#quickList");
  box.innerHTML = "";
  if (!Q.shown.length) {
    box.append(el("div", { class: "qempty" }, quickInput.value.startsWith(">") ? "No matching commands" : "No matching results"));
    return;
  }
  Q.shown.forEach((it, i) => {
    if (it.sep) return box.append(el("div", { class: "qsep" }, it.sep));
    const row = el("div", { class: `qi${i === Q.index ? " act" : ""}`, role: "option" });
    row.innerHTML = `${it.icon || ""}<span class="ql">${it.html || esc(it.label)}</span>${it.desc ? `<span class="qd">${it.descHtml || esc(it.desc)}</span>` : '<span class="qd"></span>'}${it.key ? `<span class="qk">${it.key.split(" ").map((k) => `<kbd>${esc(k)}</kbd>`).join("")}</span>` : ""}`;
    row.addEventListener("mousedown", (e) => {
      e.preventDefault();
      Q.index = i;
      acceptQuick();
    });
    row.addEventListener("mousemove", () => {
      if (Q.index !== i) {
        Q.index = i;
        for (const n of box.children) n.classList.remove("act");
        row.classList.add("act");
      }
    });
    box.append(row);
  });
  const act = box.querySelector(".act");
  if (act) act.scrollIntoView({ block: "nearest" });
}
function acceptQuick() {
  const it = Q.shown[Q.index];
  const query = quickInput.value;
  const fn = Q.onAccept;
  closeQuick(false);
  if (it && it.run) it.run(query);
  else if (fn) fn(it || null, query);
}
function bindQuick() {
  quickInput.addEventListener("input", refreshQuick);
  quickInput.addEventListener("keydown", (e) => {
    const k = e.key;
    const step = (d) => {
      if (!Q.shown.length) return;
      let i = Q.index;
      do i = (i + d + Q.shown.length) % Q.shown.length; while (Q.shown[i].sep);
      Q.index = i;
      renderQuick();
    };
    if (k === "ArrowDown") step(1);
    else if (k === "ArrowUp") step(-1);
    else if (k === "PageDown") for (let n = 0; n < 10; n++) step(1);
    else if (k === "PageUp") for (let n = 0; n < 10; n++) step(-1);
    else if (k === "Enter") acceptQuick();
    else if (k === "Escape") closeQuick();
    else return;
    e.preventDefault();
    e.stopPropagation();
  });
  quickInput.addEventListener("blur", () => setTimeout(() => {
    if (Q.open && document.activeElement !== quickInput) closeQuick();
  }, 120));
}
function filterItems(items, query, key = (i) => i.label) {
  if (!query) return items;
  const out = [];
  for (const it of items) {
    if (it.sep) continue;
    const m = fuzzy(query, key(it));
    if (m) out.push({ ...it, html: hl(it.label, m.idx.filter((x) => x < it.label.length)), score: m.score });
  }
  return out.sort((a, b) => b.score - a.score);
}
function pick(items, { placeholder = "", value = "" } = {}) {
  return new Promise((resolve) => {
    quick({ value, placeholder, provider: (q) => filterItems(items, q), onAccept: (it) => resolve(it ? it.value ?? it : null), onCancel: () => resolve(null) });
  });
}

// Command palette (>), Go to Line (:), and file search (no prefix).
function openPalette(prefix = ">") {
  quick({ value: prefix, placeholder: "Search files by name, > for commands, : to go to a line", provider: paletteProvider });
}
async function paletteProvider(q) {
  if (q.startsWith(">")) {
    const query = q.slice(1).trim();
    const items = [...CMDS.values()].filter((c) => !c.hidden && (!c.enabled || c.enabled())).map((c) => ({ label: (c.cat ? c.cat + ": " : "") + c.label, key: keyFor(c.id), run: () => run(c.id), id: c.id }));
    const recent = paletteRecent.map((id) => items.find((i) => i.id === id)).filter(Boolean);
    if (!query) return [...(recent.length ? [{ sep: "recently used" }, ...recent, { sep: "all commands" }] : []), ...items.sort((a, b) => a.label.localeCompare(b.label))];
    return filterItems(items, query);
  }
  if (q.startsWith(":")) {
    const t = S.active;
    if (!t) return [{ label: "Open a tab to go to a line", run: () => {} }];
    const m = /^:\s*(\d+)?(?:\s*[:,]\s*(\d+))?\s*$/.exec(q);
    const n = t.buf.doc.size;
    if (!m || !m[1]) return [{ label: `Type a line number between 1 and ${n.toLocaleString()} to go to (add :column for a column)`, run: () => {} }];
    const line = Math.min(n, Math.max(1, Number(m[1]))), col = m[2] ? Number(m[2]) : 1;
    return [{ label: `Go to line ${line}${m[2] ? `, column ${col}` : ""}`, run: () => gotoLine(line, col) }];
  }
  return fileItems(q.trim());
}
const paletteRecent = [];
async function fileItems(query) {
  const open = S.tabs.map((t) => ({ label: t.name, desc: t.path ? tildePath(dirname(t.path)) : "unsaved", path: t.path, tab: t }));
  const recent = S.recent.filter((p) => !findTab(p)).map((p) => ({ label: basename(p), desc: tildePath(dirname(p)), path: p }));
  let folder = [];
  if (S.folder) {
    if (!S.folderFiles || Date.now() - S.folderFilesAt > 30000) {
      const r = await api("list_files", { root: S.folder, limit: 30000 });
      S.folderFiles = r.ok ? r.files : [];
      S.folderFilesAt = Date.now();
    }
    const seen = new Set([...open.map((o) => o.path), ...recent.map((r) => r.path)]);
    folder = S.folderFiles.map((rel) => ({ label: basename(rel), desc: dirname(rel) === rel ? "" : rel.includes("/") ? dirname(rel) : "", path: S.folder + "/" + rel, rel })).filter((f) => !seen.has(f.path));
  }
  const icon = (it) => fileIconHtml(it.tab ? it.tab.langId : LG.detect(it.label, ""), it.label);
  const toItem = (it) => ({ ...it, icon: icon(it), run: () => (it.tab ? activate(it.tab) : openPath(it.path)) });
  if (!query) {
    const out = [];
    if (open.length) out.push({ sep: "open tabs" }, ...open.map(toItem));
    if (recent.length) out.push({ sep: "recently opened" }, ...recent.slice(0, 12).map(toItem));
    if (folder.length) out.push({ sep: basename(S.folder) }, ...folder.slice(0, 40).map(toItem));
    return out;
  }
  const scored = [];
  for (const it of [...open, ...recent, ...folder]) {
    const name = fuzzy(query, it.label);
    const full = name ? null : fuzzy(query, (it.rel || (it.desc ? it.desc + "/" : "") + it.label));
    const m = name || full;
    if (!m) continue;
    const bonus = (it.tab ? 3 : 0) + (name ? 8 : 0);
    scored.push({ ...toItem(it), html: name ? hl(it.label, m.idx) : esc(it.label), score: m.score + bonus });
    if (scored.length > 5000) break;
  }
  return scored.sort((a, b) => b.score - a.score).slice(0, 200);
}
function gotoLine(line, col = 1) {
  const t = S.active;
  if (!t) return;
  const l = Math.min(t.buf.doc.size, Math.max(1, line)) - 1;
  const text = t.buf.doc.line(l);
  const ch = M.offsetAt(text, 0, text.length, Math.max(0, col - 1), t.tabSize);
  t.buf.setSels([M.S(M.P(l, ch))]);
  t.layout.reveal(l);
  view.afterMove(false);
  view.reveal(t.buf.main.head, 0, "center");
  view.focus();
}

async function pickLanguage(t = S.active) {
  if (!t) return;
  const items = [{ label: "Auto Detect", desc: "from the file name and first line", value: "__auto" }, ...LG.languages.map((l) => ({ label: l.name, desc: l.id === t.langId ? "current" : (l.exts || []).slice(0, 4).join(" "), value: l.id, icon: fileIconHtml(l.id), active: l.id === t.langId }))];
  const v = await pick(items, { placeholder: "Select Language Mode" });
  if (!v) return;
  if (v === "__auto") setLang(t, LG.detect(t.path ? t.name : "", firstLine(t.buf.doc.lines[0])), false);
  else setLang(t, v, true);
}
async function pickEncodingAction(t = S.active) {
  if (!t) return;
  const v = await pick([{ label: "Reopen with Encoding", desc: t.path ? "" : "(save the file first)", value: "reopen" }, { label: "Save with Encoding", value: "save" }], { placeholder: `Current encoding: ${encName(t.encoding)}` });
  if (v) pickEncoding(t, v);
}
async function pickEncoding(t, mode) {
  if (mode === "reopen" && !t.path) return status("Save the file first to reopen it with another encoding");
  const items = S.encodings.map((e) => ({ label: e.name, desc: e.id === t.encoding ? "current" : e.id, value: e.id, active: e.id === t.encoding }));
  const v = await pick(items, { placeholder: mode === "reopen" ? "Reopen with encoding" : "Save with encoding" });
  if (!v) return;
  if (mode === "reopen") await reloadTab(t, { encoding: v });
  else {
    t.encoding = v;
    if (t.path) await saveTab(t);
    else t.buf.markDirty();
    renderChrome();
  }
}
async function pickEol(t = S.active) {
  if (!t) return;
  const v = await pick([{ label: "LF", desc: "Unix and macOS", value: "lf", active: t.eol === "lf" }, { label: "CRLF", desc: "Windows", value: "crlf", active: t.eol === "crlf" }, { label: "CR", desc: "classic Mac OS", value: "cr", active: t.eol === "cr" }], { placeholder: "Select End of Line Sequence" });
  if (v) setEol(t, v);
}
function setEol(t, v) {
  if (t.eol === v) return;
  t.eol = v;
  t.buf.markDirty();
  t.lastDirty = true;
  renderTabs();
  renderChrome();
  view.htmlCache.clear();
  view.render();
  sessionDirty();
}
async function pickIndent(t = S.active) {
  if (!t) return;
  const v = await pick([
    { label: "Indent Using Spaces", value: "spaces" }, { label: "Indent Using Tabs", value: "tabs" },
    { label: "Detect Indentation from Content", value: "detect" },
    { label: "Convert Indentation to Spaces", value: "toSpaces" }, { label: "Convert Indentation to Tabs", value: "toTabs" },
  ], { placeholder: `Current: ${t.insertSpaces ? "spaces" : "tabs"}, size ${t.tabSize}` });
  if (!v) return;
  if (v === "detect") {
    const d = M.detectIndent(t.buf.doc);
    if (d) {
      t.insertSpaces = d.insertSpaces;
      if (d.tabSize) t.tabSize = d.tabSize;
    }
    return afterIndentChange(t);
  }
  if (v === "toSpaces" || v === "toTabs") return convertIndent(v === "toTabs");
  const size = await pick([1, 2, 3, 4, 5, 6, 7, 8].map((n) => ({ label: String(n), desc: n === t.tabSize ? "current" : "", value: n, active: n === t.tabSize })), { placeholder: "Select Tab Size" });
  if (size) t.tabSize = size;
  t.insertSpaces = v === "spaces";
  afterIndentChange(t);
}
function afterIndentChange(t) {
  if (t === S.active) {
    view.htmlCache.clear();
    view.afterMove(false);
  }
  renderStatus();
  sessionDirty();
}
function convertIndent(toTabs) {
  const t = S.active;
  edit((b) => M.transformLines(b, (lines) => lines.map((l) => {
    const ws = M.leadingWs(l);
    const w = M.vwidth(ws, 0, ws.length, t.tabSize);
    const ind = toTabs ? "\t".repeat(Math.floor(w / t.tabSize)) + " ".repeat(w % t.tabSize) : " ".repeat(w);
    return ind + l.slice(ws.length);
  }), "indentation"));
  t.insertSpaces = !toTabs;
  afterIndentChange(t);
}
async function pickLayout() {
  const v = await pick(Object.entries(LAYOUT_NAMES).map(([k, n]) => ({ label: n, desc: { notepad: "plain text, word wrap, no line numbers", notepadpp: "toolbar, line numbers, folding, bookmarks", code: "explorer, search, minimap, command palette" }[k], value: k, active: prefs.layout === k })), { placeholder: "Choose a layout (you can still change any part from the View menu)" });
  if (v) applyLayout(v);
}
function applyLayout(k) {
  Object.assign(prefs, LAYOUTS[k], { layout: k });
  savePrefs();
  applyPrefs();
  status(`${LAYOUT_NAMES[k]} layout`);
}

// ------------------------------------------------------------------ commands
const CMDS = new Map();
const KEYS = new Map();
const CHORDS = new Set();
function cmd(id, label, run, o = {}) {
  CMDS.set(id, { id, label, run, ...o });
}
function keyFor(id) {
  const c = CMDS.get(id);
  if (!c || !c.key) return "";
  return Array.isArray(c.key) ? c.key[0] : c.key;
}
function buildKeymap() {
  KEYS.clear();
  CHORDS.clear();
  for (const c of CMDS.values()) {
    for (const k of [].concat(c.key || [])) {
      KEYS.set(k, c.id);
      if (k.includes(" ")) CHORDS.add(k.split(" ")[0]);
    }
  }
}
function run(id, ...args) {
  const c = CMDS.get(id);
  if (!c) return;
  if (c.editor && !S.active) return;
  if (c.enabled && !c.enabled()) return;
  if (S.macro.recording && c.rec) S.macro.steps.push({ t: "cmd", id });
  if (!c.hidden) {
    const i = paletteRecent.indexOf(id);
    if (i >= 0) paletteRecent.splice(i, 1);
    paletteRecent.unshift(id);
    paletteRecent.length = Math.min(paletteRecent.length, 8);
  }
  return c.run(...args);
}
const tab = () => S.active;
const ed = { editor: true, rec: true };
const lineOp = (fn) => () => edit((b) => M.transformLines(b, fn));
const textOp = (fn) => () => {
  const r = edit((b) => M.transformText(b, fn, wordRe()));
  if (r && r.error) toast(r.error, true);
};
const natural = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

function registerCommands() {
  // File
  cmd("newTab", "New Tab", () => newTab(), { cat: "File", key: "Ctrl+N" });
  cmd("newCodeFile", "New Code File…", newCodeFile, { cat: "File", key: "Ctrl+Alt+N" });
  cmd("newWindow", "New Window", () => post("/spawn", {}), { cat: "File", key: "Ctrl+Shift+N" });
  cmd("open", "Open File…", openFileDialog, { cat: "File", key: "Ctrl+O" });
  cmd("openFolder", "Open Folder…", openFolderDialog, { cat: "File", key: "Ctrl+K Ctrl+O" });
  cmd("openRecent", "Open Recent…", () => openPalette(""), { cat: "File", key: "Ctrl+R" });
  cmd("reopenClosed", "Reopen Closed Tab", reopenClosed, { cat: "File", key: "Ctrl+Shift+T" });
  cmd("save", "Save", () => saveTab(), { cat: "File", key: "Ctrl+S", editor: true });
  cmd("saveAs", "Save As…", () => saveTab(S.active, { as: true }), { cat: "File", key: "Ctrl+Shift+S", editor: true });
  cmd("saveAll", "Save All", saveAll, { cat: "File", key: "Ctrl+Alt+S" });
  cmd("reload", "Reload from Disk", () => reloadTab(), { cat: "File", editor: true, enabled: () => !!(tab() && tab().path) });
  cmd("renameFile", "Rename File…", () => fsRename(tab().path), { cat: "File", editor: true, enabled: () => !!(tab() && tab().path) });
  cmd("trashFile", "Move File to Trash", () => fsTrash(tab().path), { cat: "File", editor: true, enabled: () => !!(tab() && tab().path) });
  cmd("revealFile", "Reveal in File Manager", () => api("reveal", { path: tab().path }), { cat: "File", editor: true, enabled: () => !!(tab() && tab().path) });
  cmd("openTerminal", "Open in Terminal", () => openTerminal(tab() && tab().path ? dirname(tab().path) : S.folder || S.home), { cat: "File" });
  cmd("copyPath", "Copy Path of Active File", () => clipSet(tab().path || ""), { cat: "File", editor: true, enabled: () => !!(tab() && tab().path) });
  cmd("copyRelPath", "Copy Relative Path of Active File", () => clipSet(relPath(tab().path)), { cat: "File", editor: true, enabled: () => !!(tab() && tab().path) });
  cmd("copyName", "Copy File Name", () => clipSet(tab().name), { cat: "File", editor: true });
  cmd("print", "Print…", printTab, { cat: "File", editor: true, key: "Ctrl+Alt+P" });
  cmd("closeTab", "Close Tab", () => closeTab(tab()), { cat: "File", key: ["Ctrl+W", "Ctrl+F4"], editor: true });
  cmd("closeAll", "Close All Tabs", () => closeTabs(S.tabs), { cat: "File", key: "Ctrl+K Ctrl+W" });
  cmd("closeSaved", "Close Saved Tabs", () => closeTabs(S.tabs.filter((t) => !t.buf.dirty)), { cat: "File", key: "Ctrl+K U" });
  cmd("closeFolder", "Close Folder", () => setFolder(null), { cat: "File", enabled: () => !!S.folder });
  cmd("exit", "Exit", () => requestClose(), { cat: "File" });
  cmd("nextTab", "Next Tab", () => cycleTab(1), { cat: "View", key: ["Ctrl+Tab", "Ctrl+PageDown"] });
  cmd("prevTab", "Previous Tab", () => cycleTab(-1), { cat: "View", key: ["Ctrl+Shift+Tab", "Ctrl+PageUp"] });
  for (let i = 1; i <= 9; i++) cmd("tab" + i, `Open Tab ${i}`, () => S.tabs[i === 9 ? S.tabs.length - 1 : i - 1] && activate(S.tabs[i === 9 ? S.tabs.length - 1 : i - 1]), { key: "Alt+" + i, hidden: true });
  cmd("moveTabLeft", "Move Tab Left", () => moveTab(-1), { cat: "View", key: "Ctrl+Shift+PageUp", editor: true });
  cmd("moveTabRight", "Move Tab Right", () => moveTab(1), { cat: "View", key: "Ctrl+Shift+PageDown", editor: true });

  // Edit
  cmd("undo", "Undo", () => edit((b) => b.undo()), { cat: "Edit", key: "Ctrl+Z", ...ed });
  cmd("redo", "Redo", () => edit((b) => b.redo()), { cat: "Edit", key: ["Ctrl+Y", "Ctrl+Shift+Z"], ...ed });
  cmd("cut", "Cut", () => copy(true), { cat: "Edit", key: ["Ctrl+X", "Shift+Delete"], editor: true });
  cmd("copy", "Copy", () => copy(false), { cat: "Edit", key: ["Ctrl+C", "Ctrl+Insert"], editor: true });
  cmd("paste", "Paste", () => paste(), { cat: "Edit", key: ["Ctrl+V", "Shift+Insert"], editor: true });
  cmd("delete", "Delete", () => edit((b) => M.deleteForward(b)), { cat: "Edit", key: "Delete", ...ed });
  cmd("selectAll", "Select All", () => { const b = tab().buf; b.setSels([M.S(M.P(0, 0), b.doc.end())]); view.afterMove(false); }, { cat: "Selection", key: "Ctrl+A", editor: true });
  cmd("find", "Find", () => openFind(false), { cat: "Edit", key: "Ctrl+F", editor: true });
  cmd("replace", "Replace", () => openFind(true), { cat: "Edit", key: "Ctrl+H", editor: true });
  cmd("findNext", "Find Next", () => findStep(1), { cat: "Edit", key: "F3", editor: true });
  cmd("findPrev", "Find Previous", () => findStep(-1), { cat: "Edit", key: "Shift+F3", editor: true });
  cmd("findSelNext", "Find Next Selection", () => { openFind(false); findStep(1); view.focus(); }, { cat: "Edit", key: "Ctrl+F3", editor: true });
  cmd("findInFiles", "Find in Files", () => findInFolder(null), { cat: "Edit", key: "Ctrl+Shift+F" });
  cmd("replaceInFiles", "Replace in Files", () => { findInFolder(null); if (!SR.showReplace) $("#srToggleReplace").click(); $("#srReplace").focus(); }, { cat: "Edit", key: "Ctrl+Shift+H" });
  cmd("toggleComment", "Toggle Line Comment", () => edit((b, t) => M.toggleLineComment(b, langOf(t))), { cat: "Edit", key: ["Ctrl+/", "Ctrl+Q", "Ctrl+K Ctrl+C"], ...ed });
  cmd("blockComment", "Toggle Block Comment", () => edit((b, t) => M.toggleBlockComment(b, langOf(t))), { cat: "Edit", key: ["Shift+Alt+A", "Ctrl+Shift+Q"], ...ed });
  cmd("insertDateTime", "Insert Time/Date", () => typedRaw(new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) + " " + new Date().toLocaleDateString()), { cat: "Edit", key: "F5", ...ed });
  cmd("insertIsoDate", "Insert ISO Date and Time", () => typedRaw(new Date().toISOString().replace(/\.\d+Z$/, "Z")), { cat: "Edit", ...ed });
  cmd("insertUuid", "Insert UUID", () => typedRaw(uuid()), { cat: "Edit", ...ed });
  cmd("toggleReadOnly", "Read-Only", () => { tab().buf.readonly = !tab().buf.readonly; renderTabs(); renderStatus(); sessionDirty(); }, { cat: "Edit", editor: true, checked: () => !!(tab() && tab().buf.readonly) });
  cmd("toggleOvertype", "Toggle Insert/Overtype Mode", () => { S.overtype = !S.overtype; renderStatus(); view.render(); }, { cat: "Edit", key: "Insert", editor: true, checked: () => S.overtype });
  cmd("macroRecord", "Start/Stop Recording Macro", toggleMacro, { cat: "Macro", key: "Ctrl+Shift+R", editor: true, checked: () => S.macro.recording });
  cmd("macroPlay", "Play Back Macro", () => playMacro(1), { cat: "Macro", key: "Ctrl+Alt+R", editor: true, enabled: () => !!S.macro.last.length });
  cmd("macroPlayN", "Run Macro Multiple Times…", playMacroN, { cat: "Macro", editor: true, enabled: () => !!S.macro.last.length });
  cmd("macroPlayEnd", "Run Macro Until End of File", () => playMacro(Infinity), { cat: "Macro", editor: true, enabled: () => !!S.macro.last.length });

  // Line operations
  cmd("duplicateLine", "Duplicate Line", () => edit((b) => M.copyLines(b, 1)), { cat: "Line", key: "Ctrl+Shift+D", ...ed });
  cmd("copyLineUp", "Copy Line Up", () => edit((b) => M.copyLines(b, -1)), { cat: "Line", key: "Shift+Alt+Up", ...ed });
  cmd("copyLineDown", "Copy Line Down", () => edit((b) => M.copyLines(b, 1)), { cat: "Line", key: "Shift+Alt+Down", ...ed });
  cmd("moveLineUp", "Move Line Up", () => edit((b) => M.moveLines(b, -1)), { cat: "Line", key: ["Alt+Up", "Ctrl+Shift+Up"], ...ed });
  cmd("moveLineDown", "Move Line Down", () => edit((b) => M.moveLines(b, 1)), { cat: "Line", key: ["Alt+Down", "Ctrl+Shift+Down"], ...ed });
  cmd("deleteLine", "Delete Line", () => edit((b) => M.deleteLines(b)), { cat: "Line", key: "Ctrl+Shift+K", ...ed });
  cmd("joinLines", "Join Lines", () => edit((b) => M.joinLines(b)), { cat: "Line", key: "Ctrl+J", ...ed });
  cmd("insertLineBelow", "Insert Line Below", () => edit((b, t) => M.insertLine(b, editOpts(t), 1)), { cat: "Line", key: "Ctrl+Enter", ...ed });
  cmd("insertLineAbove", "Insert Line Above", () => edit((b, t) => M.insertLine(b, editOpts(t), -1)), { cat: "Line", key: "Ctrl+Shift+Enter", ...ed });
  cmd("sortAsc", "Sort Lines Ascending", lineOp((l) => [...l].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))), { cat: "Line", ...ed });
  cmd("sortDesc", "Sort Lines Descending", lineOp((l) => [...l].sort((a, b) => (a < b ? 1 : a > b ? -1 : 0))), { cat: "Line", ...ed });
  cmd("sortCi", "Sort Lines Ascending, Ignoring Case", lineOp((l) => [...l].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }))), { cat: "Line", ...ed });
  cmd("sortNatural", "Sort Lines Naturally (numbers as numbers)", lineOp((l) => [...l].sort(natural.compare)), { cat: "Line", ...ed });
  cmd("sortLength", "Sort Lines by Length", lineOp((l) => [...l].sort((a, b) => a.length - b.length)), { cat: "Line", ...ed });
  cmd("reverseLines", "Reverse Line Order", lineOp((l) => [...l].reverse()), { cat: "Line", ...ed });
  cmd("shuffleLines", "Shuffle Lines", lineOp((l) => { const a = [...l]; for (let i = a.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [a[i], a[j]] = [a[j], a[i]]; } return a; }), { cat: "Line", ...ed });
  cmd("uniqueLines", "Remove Duplicate Lines", lineOp((l) => { const seen = new Set(); return l.filter((x) => !seen.has(x) && seen.add(x)); }), { cat: "Line", ...ed });
  cmd("uniqueConsecutive", "Remove Consecutive Duplicate Lines", lineOp((l) => l.filter((x, i) => i === 0 || x !== l[i - 1])), { cat: "Line", ...ed });
  cmd("removeEmpty", "Remove Empty Lines", lineOp((l) => l.filter((x) => x !== "")), { cat: "Line", ...ed });
  cmd("removeBlank", "Remove Empty Lines (Containing Blank Characters)", lineOp((l) => l.filter((x) => x.trim() !== "")), { cat: "Line", ...ed });
  cmd("trimTrailing", "Trim Trailing Whitespace", lineOp((l) => l.map((x) => x.replace(/[ \t]+$/, ""))), { cat: "Line", key: "Ctrl+K Ctrl+X", ...ed });
  cmd("trimLeading", "Trim Leading Whitespace", lineOp((l) => l.map((x) => x.replace(/^[ \t]+/, ""))), { cat: "Line", ...ed });
  cmd("trimBoth", "Trim Leading and Trailing Whitespace", lineOp((l) => l.map((x) => x.trim())), { cat: "Line", ...ed });
  cmd("tabsToSpaces", "Convert Tabs to Spaces", lineOp((l) => l.map((x) => expandTabs(x, tab().tabSize))), { cat: "Line", ...ed });
  cmd("indentToSpaces", "Convert Indentation to Spaces", () => convertIndent(false), { cat: "Line", editor: true });
  cmd("indentToTabs", "Convert Indentation to Tabs", () => convertIndent(true), { cat: "Line", editor: true });
  cmd("indent", "Indent Line", () => edit((b, t) => M.indentLines(b, editOpts(t), 1)), { cat: "Line", key: "Ctrl+]", ...ed });
  cmd("outdent", "Outdent Line", () => edit((b, t) => M.indentLines(b, editOpts(t), -1)), { cat: "Line", key: "Ctrl+[", ...ed });

  // Case and conversions
  cmd("upper", "Transform to Uppercase", textOp((s) => s.toUpperCase()), { cat: "Convert", key: "Ctrl+Shift+U", ...ed });
  cmd("lower", "Transform to Lowercase", textOp((s) => s.toLowerCase()), { cat: "Convert", key: "Ctrl+U", ...ed });
  cmd("title", "Transform to Title Case", textOp((s) => s.toLowerCase().replace(/(^|[^\p{L}\p{N}'’])(\p{L})/gu, (m, a, b) => a + b.toUpperCase())), { cat: "Convert", ...ed });
  cmd("sentence", "Transform to Sentence Case", textOp((s) => s.toLowerCase().replace(/(^\s*|[.!?]\s+)(\p{L})/gu, (m, a, b) => a + b.toUpperCase())), { cat: "Convert", ...ed });
  cmd("invertCase", "Invert Case", textOp((s) => [...s].map((c) => (c === c.toUpperCase() ? c.toLowerCase() : c.toUpperCase())).join("")), { cat: "Convert", ...ed });
  cmd("snakeCase", "Transform to snake_case", textOp((s) => s.replace(/([a-z0-9])([A-Z])/g, "$1_$2").replace(/[\s-]+/g, "_").toLowerCase()), { cat: "Convert", ...ed });
  cmd("camelCase", "Transform to camelCase", textOp((s) => s.toLowerCase().replace(/[_\s-]+(\p{L})/gu, (m, c) => c.toUpperCase())), { cat: "Convert", ...ed });
  cmd("base64Encode", "Base64 Encode", textOp((s) => b64encode(s)), { cat: "Convert", ...ed });
  cmd("base64Decode", "Base64 Decode", textOp((s) => b64decode(s)), { cat: "Convert", ...ed });
  cmd("urlEncode", "URL Encode", textOp((s) => encodeURIComponent(s)), { cat: "Convert", ...ed });
  cmd("urlDecode", "URL Decode", textOp((s) => decodeURIComponent(s.replace(/\+/g, " "))), { cat: "Convert", ...ed });
  cmd("htmlEscape", "Escape HTML", textOp((s) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c])), { cat: "Convert", ...ed });
  cmd("htmlUnescape", "Unescape HTML", textOp((s) => { const d = document.createElement("textarea"); d.innerHTML = s; return d.value; }), { cat: "Convert", ...ed });
  cmd("formatJson", "Format JSON", () => formatJson(false), { cat: "Format", key: "Shift+Alt+F", editor: true });
  cmd("minifyJson", "Minify JSON", () => formatJson(true), { cat: "Format", editor: true });

  // Selection and cursors
  cmd("selectLine", "Expand Line Selection", () => { M.selectLines(tab().buf); view.afterMove(true); }, { cat: "Selection", key: "Ctrl+L", editor: true, rec: true });
  cmd("addCursorAbove", "Add Cursor Above", () => { M.addCursorVertical(tab().buf, tab().layout, -1); view.afterMove(true); }, { cat: "Selection", key: "Ctrl+Alt+Up", editor: true, rec: true });
  cmd("addCursorBelow", "Add Cursor Below", () => { M.addCursorVertical(tab().buf, tab().layout, 1); view.afterMove(true); }, { cat: "Selection", key: "Ctrl+Alt+Down", editor: true, rec: true });
  cmd("cursorsLineEnds", "Add Cursors to Line Ends", cursorsToLineEnds, { cat: "Selection", key: "Shift+Alt+I", editor: true });
  cmd("addNextOcc", "Add Selection to Next Find Match", addNextOcc, { cat: "Selection", key: "Ctrl+D", editor: true, rec: true });
  cmd("selectAllOcc", "Select All Occurrences of Find Match", () => { const n = M.selectAllOccurrences(tab().buf, { caseSensitive: true, wordRe: wordRe() }); if (n) status(`${plural(n, "occurrence")} selected`); view.afterMove(false); }, { cat: "Selection", key: "Ctrl+Shift+L", editor: true });
  cmd("toggleColumnMode", "Column Selection Mode", () => { S.columnMode = !S.columnMode; renderStatus(); }, { cat: "Selection", key: "Shift+Alt+Insert", checked: () => S.columnMode });
  cmd("gotoBracket", "Go to Bracket", gotoBracket, { cat: "Go", key: "Ctrl+Shift+\\", editor: true, rec: true });
  cmd("selectToBracket", "Select to Bracket", () => gotoBracket(true), { cat: "Selection", editor: true });

  // View
  cmd("palette", "Show All Commands", () => openPalette(">"), { cat: "View", key: ["Ctrl+Shift+P", "F1"] });
  cmd("quickOpen", "Go to File…", () => openPalette(""), { cat: "Go", key: "Ctrl+P" });
  cmd("gotoLine", "Go to Line/Column…", () => openPalette(":"), { cat: "Go", key: "Ctrl+G", editor: true });
  cmd("toggleSidebar", "Toggle Side Bar", () => togglePref("sidebar"), { cat: "View", key: "Ctrl+B", checked: () => prefs.sidebar });
  cmd("showExplorer", "Show Explorer", () => { if (!prefs.sidebar) setPref("sidebar", true); showView("explorer"); $("#tree").focus(); }, { cat: "View", key: "Ctrl+Shift+E" });
  cmd("showSearch", "Show Search", () => findInFolder(null), { cat: "View" });
  cmd("showBookmarks", "Show Bookmarks", () => { if (!prefs.sidebar) setPref("sidebar", true); showView("bookmarks"); }, { cat: "View" });
  cmd("toggleActivityBar", "Activity Bar", () => togglePref("activityBar"), { cat: "View", checked: () => prefs.activityBar });
  cmd("toggleStatusBar", "Status Bar", () => togglePref("statusBar"), { cat: "View", checked: () => prefs.statusBar });
  cmd("toggleToolbar", "Toolbar", () => togglePref("toolbar"), { cat: "View", checked: () => prefs.toolbar });
  cmd("toggleTabs", "Tab Bar", () => togglePref("tabs"), { cat: "View", checked: () => prefs.tabs });
  cmd("toggleBreadcrumbs", "Breadcrumbs", () => togglePref("breadcrumbs"), { cat: "View", checked: () => prefs.breadcrumbs });
  cmd("toggleMinimap", "Minimap", () => togglePref("minimap"), { cat: "View", checked: () => prefs.minimap });
  cmd("toggleWrap", "Word Wrap", () => togglePref("wrap"), { cat: "View", key: "Alt+Z", checked: () => prefs.wrap });
  cmd("lineNumbersOn", "Line Numbers", () => setPref("lineNumbers", prefs.lineNumbers === "off" ? "on" : "off"), { cat: "View", checked: () => prefs.lineNumbers !== "off" });
  cmd("lineNumbersRelative", "Relative Line Numbers", () => setPref("lineNumbers", prefs.lineNumbers === "relative" ? "on" : "relative"), { cat: "View", checked: () => prefs.lineNumbers === "relative" });
  cmd("showWhitespace", "Show Whitespace", () => setPref("whitespace", prefs.whitespace === "all" ? "none" : "all"), { cat: "View", checked: () => prefs.whitespace === "all" });
  cmd("showAllChars", "Show All Characters", () => { const on = !(prefs.whitespace === "all" && prefs.eol); prefs.eol = on; setPref("whitespace", on ? "all" : "none"); }, { cat: "View", checked: () => prefs.whitespace === "all" && prefs.eol });
  cmd("showEol", "Show End of Line", () => togglePref("eol"), { cat: "View", checked: () => prefs.eol });
  cmd("toggleGuides", "Indent Guides", () => togglePref("indentGuides"), { cat: "View", checked: () => prefs.indentGuides });
  cmd("toggleCurrentLine", "Highlight Current Line", () => togglePref("currentLine"), { cat: "View", checked: () => prefs.currentLine });
  cmd("toggleFolding", "Folding Margin", () => togglePref("folding"), { cat: "View", checked: () => prefs.folding });
  cmd("toggleGlyph", "Bookmark Margin", () => togglePref("glyphMargin"), { cat: "View", checked: () => prefs.glyphMargin });
  cmd("toggleHighlight", "Syntax Highlighting", () => { togglePref("highlight"); for (const t of S.tabs) t.buf.hl.setLang(prefs.highlight ? t.langId : "plaintext"); view.htmlCache.clear(); view.render(); }, { cat: "View", checked: () => prefs.highlight });
  cmd("zoomIn", "Zoom In", () => zoomBy(1), { cat: "View", key: ["Ctrl+=", "Ctrl+Shift+="] });
  cmd("zoomOut", "Zoom Out", () => zoomBy(-1), { cat: "View", key: "Ctrl+-" });
  cmd("zoomReset", "Restore Default Zoom", () => setPref("zoom", 100), { cat: "View", key: "Ctrl+0" });
  cmd("fullscreen", "Toggle Full Screen", () => NATIVE ? window.ipc.postMessage(JSON.stringify({ action: "fullscreen" })) : document.documentElement.requestFullscreen?.(), { cat: "View", key: "F11" });
  cmd("themeDark", "Dark Theme", () => setPref("theme", "dark"), { cat: "Preferences", checked: () => prefs.theme === "dark" });
  cmd("themeLight", "Light Theme", () => setPref("theme", "light"), { cat: "Preferences", checked: () => prefs.theme === "light" });
  cmd("toggleTheme", "Toggle Light/Dark Theme", () => setPref("theme", prefs.theme === "dark" ? "light" : "dark"), { cat: "Preferences", key: "Ctrl+K Ctrl+T" });
  for (const k of Object.keys(LAYOUTS)) cmd("layout_" + k, `${LAYOUT_NAMES[k]} Layout`, () => applyLayout(k), { cat: "View", checked: () => prefs.layout === k });
  cmd("settings", "Settings", openSettings, { cat: "Preferences", key: "Ctrl+," });
  cmd("font", "Font…", () => openSettings("text"), { cat: "Format" });
  cmd("shortcuts", "Keyboard Shortcuts", showShortcuts, { cat: "Help", key: "Ctrl+K Ctrl+S" });
  cmd("summary", "Document Summary", showSummary, { cat: "View", editor: true });
  cmd("about", `About ${APP}`, showAbout, { cat: "Help" });

  // Folding and bookmarks
  cmd("fold", "Fold", () => foldCursor(true), { cat: "View", key: "Ctrl+Shift+[", editor: true });
  cmd("unfold", "Unfold", () => foldCursor(false), { cat: "View", key: "Ctrl+Shift+]", editor: true });
  cmd("toggleFold", "Toggle Fold", () => { const t = tab(); const l = t.buf.head.line; if (t.layout.foldAt(l)) t.layout.unfold(l), view.afterMove(false); else foldCursor(true); }, { cat: "View", key: "Ctrl+K Ctrl+L", editor: true });
  cmd("foldAll", "Fold All", () => foldAll(0), { cat: "View", key: ["Ctrl+K Ctrl+0", "Alt+0"], editor: true });
  cmd("unfoldAll", "Unfold All", unfoldAll, { cat: "View", key: ["Ctrl+K Ctrl+J", "Alt+Shift+0"], editor: true });
  for (let n = 1; n <= 5; n++) cmd("foldLevel" + n, `Fold Level ${n}`, () => foldAll(n), { cat: "View", key: `Ctrl+K Ctrl+${n}`, editor: true });
  cmd("bookmarkToggle", "Toggle Bookmark", () => toggleBookmark(), { cat: "Go", key: "Ctrl+F2", editor: true });
  cmd("bookmarkNext", "Next Bookmark", () => gotoBookmark(1), { cat: "Go", key: "F2", editor: true });
  cmd("bookmarkPrev", "Previous Bookmark", () => gotoBookmark(-1), { cat: "Go", key: "Shift+F2", editor: true });
  cmd("bookmarkClear", "Clear All Bookmarks", () => { tab().bookmarks.clear(); view.render(); renderBookmarks(); sessionDirty(); }, { cat: "Go", key: "Ctrl+Shift+F2", editor: true });
  cmd("bookmarkCopyLines", "Copy Bookmarked Lines", () => { const t = tab(); clipSet([...t.bookmarks].sort((a, b) => a - b).map((l) => t.buf.doc.line(l)).join("\n")); }, { cat: "Go", editor: true });

  // Format
  cmd("eolLf", "Line Endings: Unix (LF)", () => setEol(tab(), "lf"), { cat: "Format", editor: true, checked: () => tab() && tab().eol === "lf" });
  cmd("eolCrlf", "Line Endings: Windows (CRLF)", () => setEol(tab(), "crlf"), { cat: "Format", editor: true, checked: () => tab() && tab().eol === "crlf" });
  cmd("eolCr", "Line Endings: Macintosh (CR)", () => setEol(tab(), "cr"), { cat: "Format", editor: true, checked: () => tab() && tab().eol === "cr" });
  cmd("changeLanguage", "Change Language Mode", () => pickLanguage(), { cat: "Format", key: "Ctrl+K M", editor: true });
  cmd("changeEncoding", "Change File Encoding", () => pickEncodingAction(), { cat: "Format", editor: true });
  cmd("reopenEncoding", "Reopen with Encoding…", () => pickEncoding(tab(), "reopen"), { cat: "Format", editor: true, enabled: () => !!(tab() && tab().path) });
  cmd("saveEncoding", "Save with Encoding…", () => pickEncoding(tab(), "save"), { cat: "Format", editor: true });
  cmd("changeIndent", "Indentation…", () => pickIndent(), { cat: "Format", editor: true });
  cmd("chooseLayout", "Choose Layout…", pickLayout, { cat: "View" });

  // Cursor movement and editing keys (not in menus).
  const hid = { editor: true, hidden: true, rec: true };
  const pages = () => view.pageRows();
  const k = (id, key, fn) => cmd(id, id, fn, { ...hid, key });
  k("left", "Left", () => moveH(-1, false, false));
  k("right", "Right", () => moveH(1, false, false));
  k("selLeft", "Shift+Left", () => (S.columnMode ? boxExtend(0, -1) : moveH(-1, true, false)));
  k("selRight", "Shift+Right", () => (S.columnMode ? boxExtend(0, 1) : moveH(1, true, false)));
  k("wordLeft", "Ctrl+Left", () => moveH(-1, false, true));
  k("wordRight", "Ctrl+Right", () => moveH(1, false, true));
  k("selWordLeft", "Ctrl+Shift+Left", () => moveH(-1, true, true));
  k("selWordRight", "Ctrl+Shift+Right", () => moveH(1, true, true));
  k("up", "Up", () => moveV(-1, false));
  k("down", "Down", () => moveV(1, false));
  k("selUp", "Shift+Up", () => (S.columnMode ? boxExtend(-1, 0) : moveV(-1, true)));
  k("selDown", "Shift+Down", () => (S.columnMode ? boxExtend(1, 0) : moveV(1, true)));
  k("pageUp", "PageUp", () => moveV(-pages(), false));
  k("pageDown", "PageDown", () => moveV(pages(), false));
  k("selPageUp", "Shift+PageUp", () => moveV(-pages(), true));
  k("selPageDown", "Shift+PageDown", () => moveV(pages(), true));
  k("home", "Home", () => move((s) => (prefs.smartHome ? M.homePos(tab().layout, s) : M.P(s.head.line, 0))));
  k("end", "End", () => move((s) => M.endPos(tab().layout, s)));
  k("selHome", "Shift+Home", () => move((s) => (prefs.smartHome ? M.homePos(tab().layout, s) : M.P(s.head.line, 0)), true));
  k("selEnd", "Shift+End", () => move((s) => M.endPos(tab().layout, s), true));
  k("docStart", "Ctrl+Home", () => move(() => M.P(0, 0)));
  k("docEnd", "Ctrl+End", () => move(() => tab().buf.doc.end()));
  k("selDocStart", "Ctrl+Shift+Home", () => move(() => M.P(0, 0), true));
  k("selDocEnd", "Ctrl+Shift+End", () => move(() => tab().buf.doc.end(), true));
  k("scrollUp", "Ctrl+Up", () => { view.scroller.scrollTop -= view.lh; });
  k("scrollDown", "Ctrl+Down", () => { view.scroller.scrollTop += view.lh; });
  k("boxUp", ["Ctrl+Shift+Alt+Up", "Alt+Shift+PageUp"], () => boxExtend(-1, 0));
  k("boxDown", ["Ctrl+Shift+Alt+Down", "Alt+Shift+PageDown"], () => boxExtend(1, 0));
  k("boxLeft", ["Shift+Alt+Left", "Ctrl+Shift+Alt+Left"], () => boxExtend(0, -1));
  k("boxRight", ["Shift+Alt+Right", "Ctrl+Shift+Alt+Right"], () => boxExtend(0, 1));
  k("enter", ["Enter", "Shift+Enter"], () => typed("\n"));
  k("tab", "Tab", () => edit((b, t) => {
    const multi = b.sels.some((s) => M.sFrom(s).line !== M.sTo(s).line);
    return multi ? M.indentLines(b, editOpts(t), 1) : M.insertTab(b, editOpts(t));
  }));
  k("shiftTab", "Shift+Tab", () => edit((b, t) => M.indentLines(b, editOpts(t), -1)));
  k("backspace", ["Backspace", "Shift+Backspace"], () => edit((b, t) => M.backspace(b, editOpts(t))));
  k("deleteWordLeft", "Ctrl+Backspace", () => edit((b) => M.deleteWord(b, -1, wordRe())));
  k("deleteWordRight", "Ctrl+Delete", () => edit((b) => M.deleteWord(b, 1, wordRe())));
  k("deleteToStart", "Ctrl+Shift+Backspace", () => edit((b) => M.deleteToLineEdge(b, -1)));
  k("deleteToEnd", "Ctrl+Shift+Delete", () => edit((b) => M.deleteToLineEdge(b, 1)));
  k("escape", "Escape", () => {
    if (F.open) return closeFind();
    const b = tab().buf;
    if (b.sels.length > 1) b.setSels([b.main]);
    else if (!M.sEmpty(b.main)) b.setSels([M.S(b.main.head)]);
    view.afterMove(false);
  });
  buildKeymap();
}
function typedRaw(text) {
  edit((b) => M.insertText(b, text, "insert"));
}
function cycleTab(d) {
  if (S.tabs.length < 2) return;
  const i = S.tabs.indexOf(S.active);
  activate(S.tabs[(i + d + S.tabs.length) % S.tabs.length]);
}
function moveTab(d) {
  const i = S.tabs.indexOf(S.active);
  const j = i + d;
  if (j < 0 || j >= S.tabs.length) return;
  S.tabs.splice(i, 1);
  S.tabs.splice(j, 0, S.active);
  renderTabs();
  sessionDirty();
}
function expandTabs(line, size) {
  let out = "", col = 0;
  for (const c of line) {
    if (c === "\t") {
      const n = size - (col % size);
      out += " ".repeat(n);
      col += n;
    } else {
      out += c;
      col++;
    }
  }
  return out;
}
function b64encode(s) {
  const bytes = new TextEncoder().encode(s);
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin);
}
function b64decode(s) {
  const bin = atob(s.replace(/\s+/g, "").replace(/-/g, "+").replace(/_/g, "/"));
  const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
  return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
}
function uuid() {
  const b = crypto.getRandomValues(new Uint8Array(16));
  b[6] = (b[6] & 0x0f) | 0x40;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = [...b].map((x) => x.toString(16).padStart(2, "0")).join("");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}
function formatJson(minify) {
  const t = tab();
  const b = t.buf;
  const sel = !M.sEmpty(b.main) ? { from: M.sFrom(b.main), to: M.sTo(b.main) } : null;
  const src = sel ? b.doc.get(sel.from, sel.to) : b.doc.text();
  let obj;
  try {
    obj = JSON.parse(src);
  } catch (e) {
    return toast(`That isn't valid JSON: ${e.message}`, true);
  }
  const out = minify ? JSON.stringify(obj) : JSON.stringify(obj, null, t.insertSpaces ? t.tabSize : "\t");
  if (sel) edit((bb) => bb.apply([{ from: sel.from, to: sel.to, text: out, sel: (c) => M.S(c.from, c.newTo) }], "format", { noMerge: true }));
  else edit((bb) => bb.setText(out, "format"));
  if (t.langId === "plaintext") setLang(t, "json", false);
}
function cursorsToLineEnds() {
  const b = tab().buf;
  const out = [];
  for (const s of b.sels) {
    const f = M.sFrom(s), t = M.sTo(s);
    if (f.line === t.line) {
      out.push(M.S(M.P(t.line, b.doc.line(t.line).length)));
      continue;
    }
    for (let l = f.line; l <= t.line; l++) {
      if (l === t.line && t.ch === 0) break;
      out.push(M.S(M.P(l, l === t.line ? t.ch : b.doc.line(l).length)));
    }
  }
  b.setSels(out);
  view.afterMove(false);
}
let occWord = false;
function addNextOcc() {
  const b = tab().buf;
  if (b.sels.length === 1 && M.sEmpty(b.main)) occWord = true;
  else if (b.sels.length === 1) occWord = false;
  const r = M.addNextOccurrence(b, { wholeWord: occWord, caseSensitive: true, wordRe: wordRe() });
  if (r) view.afterMove(false), view.reveal(r.sel.head, 0, isVisible(r.sel.head) ? "nearest" : "center");
}
function gotoBracket(select = false) {
  const t = tab();
  const b = t.buf;
  let m = M.matchBracket(b, b.head);
  if (!m) {
    // Not next to a bracket: jump to the bracket enclosing the cursor, like VS Code.
    for (const [o, c] of [["(", ")"], ["[", "]"], ["{", "}"]]) {
      const p = M.matchBackward(b, b.head, o, c);
      if (p && (!m || M.cmp(p, m.a) > 0)) m = { a: p, b: null };
    }
    if (!m) return;
    m = M.matchBracket(b, M.P(m.a.line, m.a.ch + 1)) || m;
  }
  if (!m.b) return;
  const target = M.cmp(m.b, b.head) > 0 ? M.P(m.b.line, m.b.ch + (select ? 1 : 0)) : m.b;
  if (select) {
    const from = M.pmin(m.a, m.b), to = M.pmax(m.a, m.b);
    b.setSels([M.S(from, M.P(to.line, to.ch + 1))]);
  } else b.setSels([M.S(target)]);
  view.afterMove(true);
}

// ------------------------------------------------------------------ macros (Notepad++)
function toggleMacro() {
  const m = S.macro;
  if (m.recording) {
    m.recording = false;
    m.last = m.steps;
    m.steps = [];
    status(m.last.length ? `Recorded a macro with ${plural(m.last.length, "step")} (Ctrl+Alt+R plays it back)` : "Nothing was recorded");
  } else {
    m.recording = true;
    m.steps = [];
    status("Recording a macro… press Ctrl+Shift+R to stop");
  }
  renderStatus();
}
function playMacro(times) {
  const steps = S.macro.last;
  if (!steps.length || !S.active) return;
  const b = S.active.buf;
  let n = 0;
  while (n < times) {
    const before = b.version, line = b.head.line;
    for (const s of steps) {
      if (s.t === "text") typed(s.v);
      else if (s.t === "paste") pasteText(s.v);
      else if (s.id !== "macroRecord" && s.id !== "macroPlay") run(s.id);
    }
    n++;
    if (times === Infinity && (b.head.line >= b.doc.size - 1 || (b.version === before && b.head.line === line))) break;
    if (n > 100000) break;
  }
  status(`Ran the macro ${plural(n, "time")}`);
}
async function playMacroN() {
  const v = await promptText({ title: "Run a Macro Multiple Times", label: "How many times?", value: "10" });
  const n = Number(v);
  if (n > 0) playMacro(Math.min(n, 100000));
}

// ------------------------------------------------------------------ keyboard
const CODE_NAMES = { Slash: "/", Backslash: "\\", BracketLeft: "[", BracketRight: "]", Equal: "=", Minus: "-", Backquote: "`", Comma: ",", Period: ".", Semicolon: ";", Quote: "'", NumpadAdd: "=", NumpadSubtract: "-" };
const KEY_NAMES = { ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right", " ": "Space", Esc: "Escape" };
function keyName(e) {
  let k = e.key;
  const c = e.code || "";
  if (/^Key[A-Z]$/.test(c)) k = c.slice(3);
  else if (/^Digit\d$/.test(c)) k = c.slice(5);
  else if (/^Numpad\d$/.test(c) && (e.ctrlKey || e.altKey)) k = c.slice(6);
  else if (CODE_NAMES[c] && (e.ctrlKey || e.altKey || e.key.length === 1)) k = CODE_NAMES[c];
  else if (KEY_NAMES[k]) k = KEY_NAMES[k];
  else if (k && k.length === 1) k = k.toUpperCase();
  if (["Control", "Shift", "Alt", "Meta", "AltGraph"].includes(k)) return "";
  return (e.ctrlKey ? "Ctrl+" : "") + (e.altKey ? "Alt+" : "") + (e.shiftKey ? "Shift+" : "") + (e.metaKey ? "Meta+" : "") + k;
}
let chord = null, chordTimer = 0;
/** Returns true when the key ran a command. inEditor: the keystroke came from the text view. */
function onKey(e, inEditor) {
  const name = keyName(e);
  if (!name) return false;
  if (chord) {
    const full = chord + " " + name;
    chord = null;
    clearTimeout(chordTimer);
    renderStatus();
    const id = KEYS.get(full);
    if (id) {
      run(id);
      return true;
    }
    status(`The key combination (${full}) is not a command.`);
    return true;
  }
  if (CHORDS.has(name)) {
    chord = name;
    status(`(${name}) was pressed. Waiting for the second key of the chord…`);
    chordTimer = setTimeout(() => { chord = null; renderStatus(); }, 4000);
    return true;
  }
  const id = KEYS.get(name);
  if (!id) return false;
  const c = CMDS.get(id);
  if (c.editor && !inEditor && !e.ctrlKey && !e.altKey && !/^F\d+$/.test(name.split("+").pop())) return false;
  if (c.editor && !S.active) return false;
  run(id);
  return true;
}
function bindGlobalKeys() {
  // Open menus take keys first, before the text view sees them.
  document.addEventListener("keydown", (e) => {
    if (menuStack.length) menuKey(e);
  }, true);
  document.addEventListener("keydown", (e) => {
    if (menuStack.length || e.defaultPrevented) return;
    const t = e.target;
    const typing = t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT") && t !== view.input;
    if (!$("#modal").hidden) return;
    if (Q.open && t === quickInput) return;
    if (e.altKey && !e.ctrlKey && !e.shiftKey && /^Key[A-Z]$/.test(e.code)) {
      const mn = MENUS.find((m) => m.mn === e.code.slice(3));
      if (mn && !KEYS.has(keyName(e))) {
        e.preventDefault();
        openTopMenu(mn.id, true);
        return;
      }
    }
    if (typing) {
      // Inside text fields only file and window shortcuts apply; editing keys stay with the field.
      const name = keyName(e);
      const id = KEYS.get(name);
      const c = id && CMDS.get(id);
      if (!c || c.editor && !["save", "saveAs", "closeTab", "find", "replace", "findNext", "findPrev"].includes(c.id) || ["selectAll", "undo", "redo", "cut", "copy", "paste"].includes(id)) return;
      if (!/Ctrl|Alt|F\d/.test(name)) return;
      e.preventDefault();
      run(id);
      return;
    }
    if (onKey(e, false)) e.preventDefault();
  });
}

// ------------------------------------------------------------------ menus
const MENUS = [
  { id: "file", label: "File", mn: "F", items: () => ["newTab", "newCodeFile", "newWindow", "-", "open", "openFolder", { label: "Open Recent", items: recentMenu }, "reopenClosed", "-", "save", "saveAs", "saveAll", "-", "reload", "renameFile", "trashFile", "revealFile", "openTerminal", "-", "print", "-", "closeTab", "closeAll", "closeFolder", "-", "exit"] },
  { id: "edit", label: "Edit", mn: "E", items: () => ["undo", "redo", "-", "cut", "copy", "paste", "delete", "selectAll", "-", "find", "replace", "findNext", "findPrev", "findInFiles", "replaceInFiles", "-", "toggleComment", "blockComment",
    { label: "Line Operations", items: ["duplicateLine", "deleteLine", "moveLineUp", "moveLineDown", "joinLines", "insertLineAbove", "insertLineBelow", "-", "sortAsc", "sortDesc", "sortCi", "sortNatural", "sortLength", "reverseLines", "shuffleLines", "-", "uniqueLines", "uniqueConsecutive", "removeEmpty", "removeBlank", "-", "trimTrailing", "trimLeading", "trimBoth", "-", "indent", "outdent"] },
    { label: "Convert Case", items: ["upper", "lower", "title", "sentence", "invertCase", "camelCase", "snakeCase"] },
    { label: "Convert", items: ["base64Encode", "base64Decode", "urlEncode", "urlDecode", "htmlEscape", "htmlUnescape", "-", "tabsToSpaces", "indentToSpaces", "indentToTabs"] },
    { label: "Insert", items: ["insertDateTime", "insertIsoDate", "insertUuid"] },
    "-", { label: "Copy to Clipboard", items: ["copyPath", "copyRelPath", "copyName"] }, "toggleReadOnly", "toggleOvertype",
    { label: "Macro", items: ["macroRecord", "macroPlay", "macroPlayN", "macroPlayEnd"] }] },
  { id: "selection", label: "Selection", mn: "S", items: () => ["selectAll", "selectLine", "selectToBracket", "-", "copyLineUp", "copyLineDown", "moveLineUp", "moveLineDown", "-", "addCursorAbove", "addCursorBelow", "cursorsLineEnds", "addNextOcc", "selectAllOcc", "-", "toggleColumnMode"] },
  { id: "view", label: "View", mn: "V", items: () => ["palette", "quickOpen", "-", { label: "Layout", items: [...Object.keys(LAYOUTS).map((k) => "layout_" + k), "-", "toggleActivityBar", "toggleSidebar", "toggleToolbar", "toggleTabs", "toggleBreadcrumbs", "toggleMinimap", "toggleStatusBar"] },
    "showExplorer", "showSearch", "showBookmarks", "-", "toggleWrap", "lineNumbersOn", "lineNumbersRelative", "showWhitespace", "showEol", "showAllChars", "toggleGuides", "toggleCurrentLine", "toggleFolding", "toggleGlyph", "toggleHighlight", "-",
    { label: "Folding", items: ["fold", "unfold", "toggleFold", "foldAll", "unfoldAll", "-", "foldLevel1", "foldLevel2", "foldLevel3", "foldLevel4", "foldLevel5"] }, "-", "zoomIn", "zoomOut", "zoomReset", "fullscreen", "-", { label: "Theme", items: ["themeDark", "themeLight"] }, "summary"] },
  { id: "go", label: "Go", mn: "G", items: () => ["quickOpen", "gotoLine", "gotoBracket", "-", "nextTab", "prevTab", "-", "bookmarkToggle", "bookmarkNext", "bookmarkPrev", "bookmarkClear", "bookmarkCopyLines"] },
  { id: "format", label: "Format", mn: "O", items: () => ["toggleWrap", "font", "-", { label: "Encoding", items: ["reopenEncoding", "saveEncoding"] }, { label: "Line Endings", items: ["eolCrlf", "eolLf", "eolCr"] }, { label: "Indentation", items: indentMenu }, { label: "Language", items: languageMenu }, "-", "formatJson", "minifyJson", "trimTrailing"] },
  { id: "tools", label: "Tools", mn: "T", items: () => ["palette", "-", "macroRecord", "macroPlay", "macroPlayN", "-", "summary", "-", "settings", "shortcuts"] },
  { id: "help", label: "Help", mn: "H", items: () => ["shortcuts", "palette", "-", "about"] },
];
function recentMenu() {
  const files = S.recent.slice(0, 12).map((p) => ({ label: tildePath(p), run: () => openPath(p) }));
  const folders = S.recentFolders.slice(0, 8).map((p) => ({ label: tildePath(p) + "/", run: () => setFolder(p) }));
  if (!files.length && !folders.length) return [{ label: "No recent files", enabled: () => false }];
  return [...files, ...(folders.length ? ["-", ...folders] : []), "-", { label: "Clear Recently Opened", run: () => { S.recent = []; S.recentFolders = []; api("recent_clear", { kind: "all" }); } }];
}
function indentMenu() {
  const t = S.active;
  if (!t) return [{ label: "Open a tab first", enabled: () => false }];
  return [
    { label: "Indent Using Spaces", checked: () => t.insertSpaces, run: () => { t.insertSpaces = true; afterIndentChange(t); } },
    { label: "Indent Using Tabs", checked: () => !t.insertSpaces, run: () => { t.insertSpaces = false; afterIndentChange(t); } },
    "-", ...[2, 4, 8].map((n) => ({ label: `Tab Size: ${n}`, checked: () => t.tabSize === n, run: () => { t.tabSize = n; afterIndentChange(t); } })),
    { label: "Other Size…", run: () => pickIndent(t) }, "-", "indentToSpaces", "indentToTabs",
  ];
}
function languageMenu() {
  const t = S.active;
  const groups = new Map();
  for (const l of LG.languages) {
    const g = l.id === "plaintext" ? "" : l.name[0].toUpperCase();
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g).push(l);
  }
  const out = [{ label: "Auto Detect", run: () => t && setLang(t, LG.detect(t.path ? t.name : "", firstLine(t.buf.doc.lines[0])), false) }, "-"];
  for (const [g, list] of groups) {
    const items = list.map((l) => ({ label: l.name, checked: () => t && t.langId === l.id, run: () => t && setLang(t, l.id, true) }));
    if (!g || list.length === 1) out.push(...items);
    else out.push({ label: g, items });
  }
  out.push("-", { label: "Choose…", key: "Ctrl+K M", run: () => pickLanguage() });
  return out;
}

let menuStack = [];
let menubarOpen = null;
function closeMenus() {
  for (const m of menuStack) m.el.remove();
  menuStack = [];
  if (menubarOpen) {
    for (const b of document.querySelectorAll(".mb")) b.classList.remove("open");
    menubarOpen = null;
  }
}
function itemSpec(it) {
  if (it === "-") return { sep: true };
  if (typeof it === "string") {
    const c = CMDS.get(it);
    if (!c) return null;
    return { label: c.label, key: keyFor(it), checked: c.checked, enabled: () => (!c.editor || S.active) && (!c.enabled || c.enabled()), run: () => run(it) };
  }
  return it;
}
function showMenu(items, x, y, { level = 0, anchor = null } = {}) {
  while (menuStack.length > level) menuStack.pop().el.remove();
  const box = el("div", { class: "menu" + (level ? " sub" : ""), role: "menu" });
  const specs = (typeof items === "function" ? items() : items).map(itemSpec).filter(Boolean);
  const entry = { el: box, specs, index: -1, buttons: [] };
  let lastSep = true;
  specs.forEach((s, i) => {
    if (s.sep) {
      if (!lastSep) box.append(el("div", { class: "msep" }));
      lastSep = true;
      return;
    }
    lastSep = false;
    const enabled = !s.enabled || s.enabled();
    const checked = s.checked ? s.checked() : false;
    const b = el("button", { class: "mi", role: "menuitem", disabled: !enabled });
    b.innerHTML = `<span class="check">${checked ? "✓" : ""}</span><span class="lbl">${esc(s.label)}</span>${s.key ? `<span class="key">${esc(s.key)}</span>` : ""}${s.items ? '<svg class="arrow" viewBox="0 0 16 16"><path d="m6 4 4 4-4 4"/></svg>' : ""}`;
    const openSub = () => {
      const r = b.getBoundingClientRect();
      showMenu(s.items, r.right - 2, r.top - 5, { level: level + 1, anchor: b });
    };
    b.addEventListener("mouseenter", () => {
      entry.index = i;
      for (const o of entry.buttons) o.classList.remove("act");
      b.classList.add("act");
      clearTimeout(entry.hoverTimer);
      if (s.items) entry.hoverTimer = setTimeout(openSub, 110);
      else entry.hoverTimer = setTimeout(() => { while (menuStack.length > level + 1) menuStack.pop().el.remove(); }, 160);
    });
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      if (!enabled) return;
      if (s.items) return openSub();
      closeMenus();
      s.run && s.run();
    });
    b._spec = s;
    b._index = i;
    entry.buttons.push(b);
    box.append(b);
  });
  if (box.lastChild && box.lastChild.classList.contains("msep")) box.lastChild.remove();
  document.body.append(box);
  const r = box.getBoundingClientRect();
  if (x + r.width > window.innerWidth - 4) x = anchor ? anchor.getBoundingClientRect().left - r.width + 2 : window.innerWidth - r.width - 4;
  if (y + r.height > window.innerHeight - 4) y = Math.max(4, window.innerHeight - r.height - 4);
  box.style.left = Math.max(4, x) + "px";
  box.style.top = Math.max(4, y) + "px";
  menuStack.push(entry);
  return entry;
}
function openTopMenu(id, keyboard = false) {
  const m = MENUS.find((x) => x.id === id);
  const btn = document.querySelector(`.mb[data-menu="${id}"]`);
  closeMenus();
  menubarOpen = id;
  btn.classList.add("open");
  const r = btn.getBoundingClientRect();
  const entry = showMenu(m.items, r.left, r.bottom + 2);
  if (keyboard) moveMenuIndex(entry, 1);
}
function moveMenuIndex(entry, d) {
  const btns = entry.buttons.filter((b) => !b.disabled);
  if (!btns.length) return;
  let i = btns.findIndex((b) => b._index === entry.index);
  i = (i + d + btns.length) % btns.length;
  for (const b of entry.buttons) b.classList.remove("act");
  btns[i].classList.add("act");
  entry.index = btns[i]._index;
  btns[i].scrollIntoView({ block: "nearest" });
}
function menuKey(e) {
  const entry = menuStack[menuStack.length - 1];
  const cur = entry.buttons.find((b) => b._index === entry.index);
  const k = e.key;
  if (k === "Escape") {
    if (menuStack.length > 1) menuStack.pop().el.remove();
    else closeMenus(), S.active && view.focus();
  } else if (k === "ArrowDown") moveMenuIndex(entry, 1);
  else if (k === "ArrowUp") moveMenuIndex(entry, -1);
  else if (k === "ArrowRight") {
    if (cur && cur._spec.items) {
      const r = cur.getBoundingClientRect();
      moveMenuIndex(showMenu(cur._spec.items, r.right - 2, r.top - 5, { level: menuStack.length, anchor: cur }), 1);
    } else if (menubarOpen) {
      const i = MENUS.findIndex((m) => m.id === menubarOpen);
      openTopMenu(MENUS[(i + 1) % MENUS.length].id, true);
    }
  } else if (k === "ArrowLeft") {
    if (menuStack.length > 1) menuStack.pop().el.remove();
    else if (menubarOpen) {
      const i = MENUS.findIndex((m) => m.id === menubarOpen);
      openTopMenu(MENUS[(i - 1 + MENUS.length) % MENUS.length].id, true);
    }
  } else if (k === "Enter" || k === " ") {
    if (cur) cur.click();
  } else if (k === "Tab" || k === "Alt" || k.length === 1) {
    closeMenus();
    if (k === "Alt") return;
  } else return;
  e.preventDefault();
  e.stopPropagation();
}
function buildMenubar() {
  const bar = $("#menubar");
  for (const m of MENUS) {
    const b = el("button", { class: "mb", "data-menu": m.id, role: "menuitem" });
    b.innerHTML = `${esc(m.label)}`;
    b.addEventListener("mousedown", (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (menubarOpen === m.id) closeMenus();
      else openTopMenu(m.id);
    });
    b.addEventListener("mouseenter", () => {
      if (menubarOpen && menubarOpen !== m.id) openTopMenu(m.id);
    });
    bar.append(b);
  }
  document.addEventListener("mousedown", (e) => {
    if (menuStack.length && !e.target.closest(".menu") && !e.target.closest(".mb")) closeMenus();
  });
  window.addEventListener("blur", closeMenus);
  window.addEventListener("resize", closeMenus);
}
function editorContextMenu(e) {
  const t = S.active;
  showMenu(["cut", "copy", "paste", "-", "selectAll", "selectAllOcc", "-", "toggleComment", "blockComment", "formatJson", "-",
    { label: "Convert Case", items: ["upper", "lower", "title", "sentence", "invertCase"] },
    { label: "Line Operations", items: ["duplicateLine", "deleteLine", "sortAsc", "sortDesc", "uniqueLines", "trimTrailing", "joinLines"] },
    "-", "bookmarkToggle", { label: "Change Language Mode…", run: () => pickLanguage(t) }, "-", "palette"], e.clientX, e.clientY);
}

// ------------------------------------------------------------------ toolbar (Notepad++ layout)
function buildToolbar() {
  const bar = $("#toolbar");
  const items = [["newTab", "newFile"], ["open", "open"], ["save", "save"], ["saveAll", "saveAll"], ["closeTab", "close"], "-", ["print", "print"], "-", ["cut", "cut"], ["copy", "copy"], ["paste", "paste"], "-", ["undo", "undo"], ["redo", "redo"], "-", ["find", "find"], ["replace", "replace"], "-", ["zoomIn", "zoomIn"], ["zoomOut", "zoomOut"], "-", ["toggleWrap", "wrap", true], ["showAllChars", "pilcrow", true], ["toggleGuides", "guides", true], "-", ["macroRecord", "record", true], ["macroPlay", "play"], "-", ["bookmarkToggle", "bookmark"], ["toggleSidebar", "sidebar", true], ["palette", "palette"]];
  for (const it of items) {
    if (it === "-") {
      bar.append(el("span", { class: "sep" }));
      continue;
    }
    const [id, icon, toggle] = it;
    const c = CMDS.get(id);
    const b = el("button", { class: "tb", title: `${c.label}${keyFor(id) ? ` (${keyFor(id)})` : ""}`, html: svg(icon), "data-cmd": id });
    b.addEventListener("mousedown", (e) => e.preventDefault());
    b.onclick = () => {
      run(id);
      syncToolbar();
    };
    if (toggle) b.dataset.toggle = "1";
    bar.append(b);
  }
}
function syncToolbar() {
  for (const b of document.querySelectorAll("#toolbar .tb[data-toggle]")) {
    const c = CMDS.get(b.dataset.cmd);
    b.classList.toggle("on", !!(c.checked && c.checked()));
  }
}

// ------------------------------------------------------------------ dialogs
function modal(build, { wide = false } = {}) {
  return new Promise((resolve) => {
    const back = $("#modal"), box = $("#dialog");
    box.className = "dialog" + (wide ? " wide" : "");
    box.innerHTML = "";
    const done = (v) => {
      back.hidden = true;
      box.innerHTML = "";
      document.removeEventListener("keydown", onKey, true);
      resolve(v);
      if (S.active) view.focus();
    };
    const onKey = (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        done(null);
      }
    };
    document.addEventListener("keydown", onKey, true);
    build(box, done);
    back.hidden = false;
    const f = box.querySelector("[autofocus]") || box.querySelector("input,select,.btn.primary");
    if (f) f.focus();
  });
}
function ask({ title, body, note, buttons }) {
  return modal((box, done) => {
    box.append(el("h3", {}, title), typeof body === "string" ? el("p", {}, body) : body);
    if (note) box.append(el("p", { class: "small" }, note));
    const row = el("div", { class: "buttons" });
    for (const b of buttons) row.append(el("button", { class: "btn" + (b.primary ? " primary" : "") + (b.danger ? " danger" : ""), on: { click: () => done(b.value) } }, b.label));
    box.append(row);
  });
}
function alertBox(title, body) {
  return ask({ title, body, buttons: [{ label: "OK", value: true, primary: true }] });
}
function promptText({ title, label, value = "", selectStem = false }) {
  return modal((box, done) => {
    const inp = el("input", { type: "text", value, spellcheck: "false", autofocus: true });
    inp.addEventListener("keydown", (e) => {
      if (e.key === "Enter") done(inp.value.trim() || null);
    });
    box.append(el("h3", {}, title), el("p", {}, label), inp, el("div", { class: "buttons" }, el("button", { class: "btn", on: { click: () => done(null) } }, "Cancel"), el("button", { class: "btn primary", on: { click: () => done(inp.value.trim() || null) } }, "OK")));
    setTimeout(() => {
      inp.focus();
      const dot = value.lastIndexOf(".");
      if (selectStem && dot > 0) inp.setSelectionRange(0, dot);
      else inp.select();
    }, 0);
  });
}

const SETTINGS = {
  text: { label: "Text Editor", items: [
    ["fontFamily", "Font family", "font", "Empty uses Cascadia Mono, then other monospace fonts."],
    ["fontSize", "Font size", "number", "", 6, 48],
    ["lineHeight", "Line height", "select", "", [[1.2, "Compact"], [1.35, "Snug"], [1.5, "Normal"], [1.7, "Relaxed"]]],
    ["wrap", "Word wrap", "bool", "Wrap long lines at the window edge (Alt+Z)."],
    ["wrapColumn", "Wrap column", "number", "0 wraps at the window edge.", 0, 400],
    ["cursorStyle", "Cursor style", "select", "", [["line", "Line"], ["block", "Block"], ["underline", "Underline"]]],
    ["cursorBlink", "Blinking cursor", "bool"],
    ["rulers", "Rulers", "text", "Columns for vertical rulers, like 80, 120."],
  ] },
  editing: { label: "Editing", items: [
    ["tabSize", "Tab size", "number", "Used when indentation isn't detected.", 1, 16],
    ["insertSpaces", "Insert spaces", "bool", "Tab inserts spaces instead of a tab character."],
    ["detectIndent", "Detect indentation", "bool", "Pick tabs or spaces and the size from each file."],
    ["autoClose", "Auto-close brackets and quotes", "bool"],
    ["autoIndent", "Auto-indent", "bool", "Keep and extend indentation on Enter."],
    ["smartHome", "Smart Home", "bool", "Home goes to the first non-blank character first."],
    ["matchBrackets", "Highlight matching brackets", "bool"],
    ["occurrences", "Highlight occurrences", "bool", "Mark other copies of the word or selection."],
  ] },
  view: { label: "Appearance", items: [
    ["theme", "Theme", "select", "", [["dark", "Dark"], ["light", "Light"]]],
    ["layout", "Layout", "select", "Sets the parts below to match each app.", [["notepad", "Notepad"], ["notepadpp", "Notepad++"], ["code", "Code"]]],
    ["lineNumbers", "Line numbers", "select", "", [["on", "On"], ["relative", "Relative"], ["off", "Off"]]],
    ["minimap", "Minimap", "bool"],
    ["whitespace", "Show whitespace", "select", "", [["none", "None"], ["boundary", "Boundary"], ["trailing", "Trailing"], ["all", "All"]]],
    ["indentGuides", "Indent guides", "bool"],
    ["currentLine", "Highlight current line", "bool"],
    ["highlight", "Syntax highlighting", "bool"],
    ["scrollBeyond", "Scroll beyond last line", "bool"],
  ] },
  files: { label: "Files", items: [
    ["restoreSession", "Remember open tabs", "bool", "Tabs you had open come back the next time you start."],
    ["unsavedOnClose", "Unsaved changes when closing the window", "select", "Keep brings them back next time without asking, like Windows 11 Notepad and Notepad++.", [["ask", "Ask to save"], ["keep", "Keep for next time"]]],
    ["defaultEol", "Line endings for new files", "select", "", [["lf", "LF (Unix)"], ["crlf", "CRLF (Windows)"]]],
    ["defaultEncoding", "Encoding for new files", "encoding"],
    ["trimTrailing", "Trim trailing whitespace on save", "bool"],
    ["finalNewline", "Insert final newline on save", "bool"],
  ] },
};
function openSettings(section = "text") {
  return modal((box, done) => {
    const nav = el("nav");
    const pane = el("div", { class: "pane" });
    const show = (key) => {
      for (const b of nav.children) b.classList.toggle("on", b.dataset.k === key);
      pane.innerHTML = "";
      for (const [k, label, type, desc, a, b] of SETTINGS[key].items) {
        let ctl;
        const id = "set-" + k;
        if (type === "bool") {
          ctl = el("input", { type: "checkbox", id });
          ctl.checked = !!prefs[k];
          ctl.onchange = () => (k in LAYOUTS.code ? (prefs.layout = prefs.layout, setPref(k, ctl.checked)) : setPref(k, ctl.checked));
        } else if (type === "number") {
          ctl = el("input", { type: "number", id, min: a, max: b, value: prefs[k], style: "width:90px" });
          ctl.onchange = () => setPref(k, Math.max(a, Math.min(b, Number(ctl.value) || DEFAULTS[k])));
        } else if (type === "select" || type === "encoding") {
          const opts = type === "encoding" ? S.encodings.map((e) => [e.id, e.name]) : a;
          ctl = el("select", { id, style: "max-width:220px" }, ...opts.map(([v, n]) => el("option", { value: String(v) }, n)));
          ctl.value = String(prefs[k]);
          ctl.onchange = () => {
            const v = typeof DEFAULTS[k] === "number" ? Number(ctl.value) : ctl.value;
            if (k === "layout") applyLayout(v);
            else setPref(k, v);
            if (k === "layout") show(key);
          };
        } else if (type === "font") {
          const list = el("datalist", { id: "fontList" }, ...S.fonts.map((f) => el("option", { value: f })));
          ctl = el("input", { type: "text", id, list: "fontList", value: prefs[k], placeholder: "Cascadia Mono", style: "width:220px" });
          ctl.onchange = () => setPref(k, ctl.value.trim().replace(/^["']|["']$/g, ""));
          pane.append(list);
        } else {
          ctl = el("input", { type: "text", id, value: prefs[k], style: "width:220px" });
          ctl.onchange = () => setPref(k, ctl.value);
        }
        pane.append(el("div", { class: "setting" }, el("label", { for: id }, label), ctl, desc ? el("span", { class: "d" }, desc) : null));
      }
    };
    for (const [key, s] of Object.entries(SETTINGS)) nav.append(el("button", { "data-k": key, on: { click: () => show(key) } }, s.label));
    box.append(el("h3", {}, "Settings"), el("div", { class: "settings" }, nav, pane), el("div", { class: "buttons" }, el("button", { class: "btn", on: { click: () => { Object.assign(prefs, DEFAULTS); savePrefs(); applyPrefs(); show(section in SETTINGS ? section : "text"); status("Settings were reset to their defaults"); } } }, "Reset All"), el("button", { class: "btn primary", on: { click: () => done(true) } }, "Done")));
    show(section in SETTINGS ? section : "text");
  }, { wide: true });
}
function showShortcuts() {
  return modal((box, done) => {
    const grid = el("div", { class: "keys" });
    const cats = new Map();
    for (const c of CMDS.values()) {
      if (!c.key || c.hidden) continue;
      const cat = c.cat || "Other";
      if (!cats.has(cat)) cats.set(cat, []);
      cats.get(cat).push(c);
    }
    for (const [cat, list] of cats) {
      grid.append(el("h4", {}, cat));
      for (const c of list) grid.append(el("span", {}, c.label), el("span", { class: "k", html: [].concat(c.key).map((k) => k.split(" ").map((p) => `<kbd>${esc(p)}</kbd>`).join(" ")).join(" <span class='muted'>or</span> ") }));
    }
    grid.append(el("h4", {}, "Mouse"), el("span", {}, "Add a cursor"), el("span", { class: "k", html: "<kbd>Alt</kbd>+click or <kbd>Ctrl</kbd>+click" }), el("span", {}, "Column selection"), el("span", { class: "k", html: "<kbd>Alt</kbd>+drag or <kbd>Shift</kbd>+<kbd>Alt</kbd>+drag" }), el("span", {}, "Select word / line"), el("span", { class: "k", html: "double-click / triple-click" }), el("span", {}, "Bookmark a line"), el("span", { class: "k", html: "click the bookmark margin" }), el("span", {}, "Zoom"), el("span", { class: "k", html: "<kbd>Ctrl</kbd>+wheel" }));
    box.append(el("h3", {}, "Keyboard Shortcuts"), grid, el("div", { class: "buttons" }, el("button", { class: "btn primary", on: { click: () => done(true) } }, "Close")));
  }, { wide: true });
}
function showSummary() {
  const t = S.active;
  const b = t.buf;
  const text = b.doc.text();
  const words = (text.match(/[\p{L}\p{N}_'’-]+/gu) || []).length;
  const sel = b.sels.filter((s) => !M.sEmpty(s)).map((s) => b.doc.get(M.sFrom(s), M.sTo(s))).join("\n");
  const rows = [
    ["File", t.path ? tildePath(t.path) : "Not saved yet"], ["Language", langOf(t).name], ["Encoding", encName(t.encoding)], ["Line endings", eolName(t.eol)],
    ["Characters", text.length.toLocaleString()], ["Characters (no spaces)", text.replace(/\s/g, "").length.toLocaleString()], ["Words", words.toLocaleString()], ["Lines", b.doc.size.toLocaleString()],
    ["Size on disk", t.size ? `${t.size.toLocaleString()} bytes` : "—"], ["Indentation", t.insertSpaces ? `${t.tabSize} spaces` : `Tabs (size ${t.tabSize})`],
  ];
  if (sel) rows.push(["Selected characters", sel.length.toLocaleString()], ["Selected words", ((sel.match(/[\p{L}\p{N}_'’-]+/gu) || []).length).toLocaleString()]);
  return alertBox("Summary", el("div", { class: "stats" }, ...rows.flatMap(([a, v]) => [el("span", {}, a), el("span", {}, v)])));
}
function showAbout() {
  return alertBox(APP, el("div", {}, el("p", {}, `Version ${S.version || "dev"}. Part of the Axentra suite.`), el("p", {}, `A text and code editor modelled on Windows Notepad, Notepad++ and Visual Studio Code: tabs that survive restarts, syntax highlighting for ${LG.languages.length - 1} languages, multiple cursors, column selection, folding, bookmarks, macros, find and replace across files, and 25 text encodings.`)));
}
function printTab() {
  const t = S.active;
  let p = $("#print");
  if (!p) document.body.append((p = el("div", { id: "print" })));
  p.textContent = t.buf.doc.text();
  window.print();
}

// ------------------------------------------------------------------ zoom, prefs and layout
function zoomBy(d) {
  const steps = [30, 50, 67, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500];
  const z = prefs.zoom;
  const next = d > 0 ? steps.find((s) => s > z) ?? 500 : [...steps].reverse().find((s) => s < z) ?? 30;
  setPref("zoom", next);
}
function applyPrefs() {
  const root = document.documentElement;
  root.dataset.theme = prefs.theme;
  const body = document.body;
  body.className = `layout-${prefs.layout}`;
  body.classList.toggle("noActivity", !prefs.activityBar);
  body.classList.toggle("noSidebar", !prefs.sidebar);
  body.classList.toggle("noToolbar", !prefs.toolbar);
  body.classList.toggle("noStatus", !prefs.statusBar);
  body.classList.toggle("noTabs", !prefs.tabs);
  body.classList.toggle("noCrumbs", !prefs.breadcrumbs);
  if (NATIVE) root.classList.add("nativeWindow");
  if (view) {
    view.colors = null;
    configureView();
    view.render();
  }
  showView(prefs.sideView || "explorer", false);
  $("#toggleSidebarBtn").classList.toggle("on", prefs.sidebar);
  syncToolbar();
  renderStatus();
  renderCrumbs();
}

// ------------------------------------------------------------------ window
/** Whether closing the window keeps unsaved changes for next time instead of asking (Windows 11 Notepad style). */
function keepsUnsaved() {
  return S.primary && prefs.restoreSession && prefs.unsavedOnClose === "keep";
}

/** Resolves with the promise's value, or with `fallback` once `ms` have passed. */
function withTimeout(promise, ms, fallback) {
  return Promise.race([promise, new Promise((resolve) => setTimeout(() => resolve(fallback), ms))]);
}

/**
 * File › Exit and the title-bar ×. In the app window GTK asks about unsaved changes (it can draw
 * even if this page can't) and answers through axnCloseChoice; elsewhere the page asks itself.
 */
async function requestClose() {
  closeMenus();
  if (!NATIVE) return closeInPage();
  await syncState(true);
  window.ipc.postMessage(JSON.stringify({ action: "close" }));
}

async function closeInPage() {
  const dirty = S.tabs.filter((t) => t.buf.dirty);
  if (dirty.length && !keepsUnsaved()) {
    const names = dirty.slice(0, 5).map((t) => "• " + t.name).join("\n") + (dirty.length > 5 ? `\n…and ${dirty.length - 5} more` : "");
    const c = await ask({ title: APP, body: dirty.length === 1 ? `Do you want to save the changes you made to “${dirty[0].name}”?` : `Do you want to save the changes to ${dirty.length} files?`, note: dirty.length > 1 ? names : "Your changes will be lost if you don't save them.", buttons: [{ label: "Cancel", value: null, cancel: true }, { label: "Don't Save", value: "discard" }, { label: dirty.length > 1 ? "Save All" : "Save", value: "save", primary: true }] });
    if (!c) return;
    return finishClose(c);
  }
  return finishClose("flush");
}

let closing = false;
/** Saves ("save") or drops ("discard") unsaved tabs, writes the session and quits. A cancelled Save As keeps the window open. */
async function finishClose(choice) {
  if (closing) return;
  closing = true;
  // Tell the window we're on it, so it doesn't offer to close anyway while a Save As is open.
  if (NATIVE) window.ipc.postMessage(JSON.stringify({ action: "closing" }));
  try {
    const dirty = S.tabs.filter((t) => t.buf.dirty);
    if (choice === "save") {
      for (const t of dirty) {
        if (!t.path) activate(t);
        if (!(await saveTab(t))) return;
      }
    } else if (choice === "discard") {
      // Untitled tabs go away; files reopen as they are on disk.
      for (const t of dirty) {
        if (t.path) t.discarded = true;
        else S.tabs.splice(S.tabs.indexOf(t), 1);
      }
      if (!S.tabs.includes(S.active)) S.active = S.tabs[0] || null;
    }
    if (S.primary) await withTimeout(saveSession(), 2500);
    if (NATIVE) window.ipc.postMessage(JSON.stringify({ action: "quit" }));
  } finally {
    closing = false;
  }
}
window.axnCloseChoice = (choice) => finishClose(choice);
window.axnRequestClose = () => closeInPage();
window.axnOpenPaths = (paths) => openPaths(paths);

function bindWindow() {
  if (!NATIVE) return;
  const sendWindow = (action, fields = {}) => window.ipc.postMessage(JSON.stringify({ action, ...fields }));
  $("#windowControls").hidden = false;
  $("#windowMinimize").addEventListener("click", () => sendWindow("minimize"));
  $("#windowMaximize").addEventListener("click", () => sendWindow("toggleMaximize"));
  $("#windowClose").addEventListener("click", () => requestClose());
  window.axnWindowState = ({ maximized, fullscreen }) => {
    document.documentElement.classList.toggle("maximized", !!maximized);
    document.documentElement.classList.toggle("fullscreen", !!fullscreen);
    $("#windowMaximize").title = maximized ? "Restore" : "Maximize";
    $("#windowMaximizeIcon").setAttribute("d", maximized ? "M5 3V1h10v10h-2M1 5h10v10H1z" : "M3 3h10v10H3z");
  };
  const interactive = (e) => e.target.closest("button,input,select,textarea,a,label,.commandCenter");
  let press = null;
  $("#titlebar").addEventListener("mousedown", (e) => {
    if (e.button !== 0 || interactive(e)) return;
    e.preventDefault();
    press = { x: e.screenX, y: e.screenY };
  });
  document.addEventListener("mousemove", (e) => {
    if (!press) return;
    if (!(e.buttons & 1)) {
      press = null;
      return;
    }
    if (Math.hypot(e.screenX - press.x, e.screenY - press.y) < 4) return;
    press = null;
    sendWindow("drag");
  });
  document.addEventListener("mouseup", () => { press = null; });
  $("#titlebar").addEventListener("dblclick", (e) => {
    if (!interactive(e)) sendWindow("toggleMaximize");
  });
  for (const edge of ["n", "ne", "e", "se", "s", "sw", "w", "nw"]) {
    const h = el("div", { class: `resizeEdge resize-${edge}`, "aria-hidden": "true" });
    h.addEventListener("mousedown", (e) => {
      if (e.button === 0) {
        e.preventDefault();
        sendWindow("resize", { edge });
      }
    });
    document.body.append(h);
  }
  sendWindow("state");
}

const toastEl = $("#toast");
let toastTimer = 0;
function toast(text, bad = false) {
  toastEl.textContent = text;
  toastEl.classList.toggle("bad", bad);
  toastEl.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toastEl.hidden = true), bad ? 7000 : 4000);
}

// ------------------------------------------------------------------ start
let booted = false;
const host = {
  keydown: (e) => onKey(e, true),
  typed,
  paste: (t) => pasteText(t),
  copyText: (cut) => (S.active ? selectionText(cut) : ""),
  contextMenu: (e) => editorContextMenu(e),
  toggleFold: (line) => toggleFoldAt(line),
  toggleBookmark: (line) => toggleBookmark(line),
  selectionChanged: () => onSelectionChanged(),
  selectionMade: () => {},
  langOf: (t) => langOf(t),
  zoom: (d) => zoomBy(d),
  focusChanged: () => {},
  idleTokenize: () => scheduleIdle(),
  overtype: () => S.overtype,
  columnMode: () => S.columnMode,
};

(async () => {
  view = new EditorView($("#editor"), host);
  registerCommands();
  buildMenubar();
  buildToolbar();
  bindFind();
  bindTree();
  bindSearch();
  bindQuick();
  bindSash();
  bindTabDrag();
  bindGlobalKeys();
  bindWindow();
  applyPrefs();
  $("#commandCenter").onclick = () => openPalette("");
  $("#toggleSidebarBtn").onclick = () => run("toggleSidebar");
  $("#layoutBtn").onclick = () => pickLayout();
  $("#abSettings").onclick = () => run("settings");
  $("#bmClear").onclick = () => {
    for (const t of S.tabs) t.bookmarks.clear();
    renderBookmarks();
    view.render();
    sessionDirty();
  };
  for (const b of document.querySelectorAll(".ab[data-view]")) b.onclick = () => toggleView(b.dataset.view);
  document.addEventListener("contextmenu", (e) => {
    if (!e.target.closest("input,textarea") || e.target === view.input) e.preventDefault();
  });
  window.addEventListener("beforeunload", (e) => {
    if (!NATIVE && S.tabs.some((t) => t.buf.dirty)) {
      e.preventDefault();
      e.returnValue = "";
    }
  });
  window.addEventListener("focus", () => checkDisk());
  document.addEventListener("visibilitychange", () => { if (!document.hidden) checkDisk(); });

  const init = await api("init");
  S.primary = init.primary !== false;
  S.home = init.home || "";
  S.recent = init.recent || [];
  S.recentFolders = init.recentFolders || [];
  S.encodings = init.encodings || [{ id: "utf-8", name: "UTF-8" }];
  S.fonts = init.fonts || [];
  S.version = init.version || "";
  if (!init.ok) toast(init.error || `${APP} couldn't start its file service.`, true);
  if (S.primary && prefs.restoreSession && init.session) {
    try {
      await restoreSession(init.session, init.buffers || {});
    } catch (e) {
      toast("Couldn't restore the previous session: " + e, true);
    }
  }
  const st = init.startup || {};
  if (st.folder) await setFolder(st.folder);
  for (const d of st.docs || []) openDoc(d);
  for (const e of st.errors || []) toast(e, true);
  booted = true;
  if (!S.active && S.tabs.length) activate(S.tabs[0]);
  renderTabs();
  renderChrome();
  renderOpenEditors();
  renderTree();
  if (SR.query) {
    $("#srQuery").value = SR.query;
    $("#srInclude").value = SR.include;
    $("#srExclude").value = SR.exclude;
  }
  if (!S.active) renderWelcome();
  setInterval(checkDisk, 2500);
  syncState();
})();
