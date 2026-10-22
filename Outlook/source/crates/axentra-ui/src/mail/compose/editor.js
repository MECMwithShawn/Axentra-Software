// Compose body editor bridge (runs inside `document::eval`).
//
// Rust embeds the config ({id, root, html, plain}) where the placeholder
// below is. The script reports to Rust with `dioxus.send(...)`:
//   {t:"ready", html, text}           content loaded (baseline snapshot)
//   {t:"input"}                       content changed (throttled)
//   {t:"snap", html, text}            debounced snapshot (text = new text only)
//   {t:"fmt", ...FormatState}         selection formatting changed
//   {t:"file", name, type, size, data} a file dropped on the form (base64)
//   {t:"mention", q, x, y}            "@q" typed before the caret (q null: ended)
//   {t:"mkey", key}                   Up/Down/Enter/Tab/Esc while the mention list is open
//   {t:"leave"}                       focus left the whole form (save now)
// Commands and reads arrive as one-shot evals calling
// `window.axComposeEditors[id].run(cmd, val)` / `.snapshot()` / `.getLink()`.
// (`dioxus.recv()` is not used: under liveview it blocks the page while
// it waits.)
const cfg = /*AX_CONFIG*/null;
// Flyouts (table, emoji, colours) open under the ribbon command that was
// used last; remember where that was.
if (!window.axCommandTracker) {
  window.axCommandTracker = true;
  const track = (e) => {
    const t = e.target;
    const b = t && t.closest && (t.closest(".ax-cmd-split") || t.closest(".ax-cmd"));
    if (b) window.axLastCommandRect = b.getBoundingClientRect();
  };
  document.addEventListener("pointerdown", track, true);
  document.addEventListener("focusin", track, true);
  // Picking from the mention list must not take focus from the body.
  document.addEventListener("mousedown", (e) => {
    if (e.target && e.target.closest && e.target.closest(".ax-compose-mention")) e.preventDefault();
  }, true);
}
let el = null;
for (let i = 0; i < 100 && !el; i++) {
  el = document.getElementById(cfg.id);
  if (!el) await new Promise((r) => setTimeout(r, 20));
}
if (!el) return;
const root = document.getElementById(cfg.root) || el.parentElement;
const appEl = el.closest(".ax-app");
function computeDark() {
  if (appEl && appEl.classList.contains("ax-theme-dark")) return true;
  if (appEl && appEl.classList.contains("ax-theme-light")) return false;
  return !!(window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches);
}
const state = { saved: null, plainNext: false, plain: !!cfg.plain, dark: computeDark(), lastFmt: "", alive: true, mention: null, mentionCount: 0 };
const SENTINEL = "rgb(1, 2, 3)";

// ------------------------------------------------------------ content safety
function clean(node) {
  node.querySelectorAll("script,style,link,meta,base,iframe,object,embed,form,input,textarea,select,button").forEach((n) => n.remove());
  node.querySelectorAll("*").forEach((n) => {
    for (const a of Array.from(n.attributes)) {
      const name = a.name.toLowerCase();
      if (name.startsWith("on") || name === "contenteditable") n.removeAttribute(a.name);
      else if ((name === "href" || name === "src" || name === "xlink:href") && /^\s*(javascript|vbscript):/i.test(a.value)) n.removeAttribute(a.name);
    }
  });
}

// ------------------------------------------------------------ dark mode
// Message colours were chosen for a white page. In dark mode, dark text and
// light backgrounds are remapped for display only; the originals are kept
// in data attributes and restored in every snapshot.
function parseRgb(s) {
  const m = /rgba?\(([\d.]+),\s*([\d.]+),\s*([\d.]+)(?:,\s*([\d.]+))?\)/.exec(s || "");
  return m ? { r: +m[1], g: +m[2], b: +m[3], a: m[4] === undefined ? 1 : +m[4] } : null;
}
function lum(c) {
  const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); };
  return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b);
}
function invertLightness(c) {
  // Keep the hue, flip the lightness (HSL), so red stays red but readable.
  const r = c.r / 255, g = c.g / 255, b = c.b / 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  let h = 0, s = 0; const l = (max + min) / 2;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
    h /= 6;
  }
  return `hsl(${Math.round(h * 360)}, ${Math.round(s * 100)}%, ${Math.round((1 - l) * 100)}%)`;
}
function darkAdjust(scope) {
  if (!state.dark) return;
  (scope || el).querySelectorAll("[style], font[color], [bgcolor]").forEach((n) => {
    if (n.dataset.axDark) return;
    const cs = getComputedStyle(n);
    const fg = parseRgb(cs.color);
    if ((n.style.color || n.getAttribute("color")) && fg && lum(fg) < 0.2) {
      n.dataset.axDark = "1";
      n.dataset.axOgc = n.style.color || "";
      if (n.hasAttribute("color")) { n.dataset.axOgca = n.getAttribute("color"); n.removeAttribute("color"); }
      n.style.color = invertLightness(fg);
    }
    const bg = parseRgb(cs.backgroundColor);
    if ((n.style.backgroundColor || n.getAttribute("bgcolor")) && bg && bg.a > 0 && lum(bg) > 0.4) {
      n.dataset.axDark = "1";
      n.dataset.axOgb = n.style.backgroundColor || "";
      if (n.hasAttribute("bgcolor")) { n.dataset.axOgba = n.getAttribute("bgcolor"); n.removeAttribute("bgcolor"); }
      n.style.backgroundColor = invertLightness(bg);
    }
  });
}
function undoDark(scope) {
  scope.querySelectorAll("[data-ax-dark]").forEach((n) => {
    if ("axOgc" in n.dataset) n.style.color = n.dataset.axOgc;
    if ("axOgca" in n.dataset) n.setAttribute("color", n.dataset.axOgca);
    if ("axOgb" in n.dataset) n.style.backgroundColor = n.dataset.axOgb;
    if ("axOgba" in n.dataset) n.setAttribute("bgcolor", n.dataset.axOgba);
    for (const k of ["axDark", "axOgc", "axOgca", "axOgb", "axOgba"]) delete n.dataset[k];
    if (n.getAttribute("style") === "") n.removeAttribute("style");
  });
}

// ------------------------------------------------------------ snapshots
function blockText(node) {
  // textContent with line breaks for blocks, for the attachment check.
  const c = node.cloneNode(true);
  c.querySelectorAll("br").forEach((b) => b.replaceWith("\n"));
  c.querySelectorAll("div,p,li,h1,h2,h3,blockquote,pre,tr").forEach((b) => b.append("\n"));
  return c.textContent || "";
}
function snapshot() {
  const c = el.cloneNode(true);
  undoDark(c);
  c.querySelectorAll("[data-ax-placeholder]").forEach((n) => n.remove());
  const html = state.plain ? plainHtml(c) : c.innerHTML;
  // New text only: drop the signature and the quoted history.
  c.querySelectorAll("[data-axentra-signature], #divRplyFwdMsg, .ax-quoted").forEach((n) => n.remove());
  c.querySelectorAll("hr").forEach((n) => { if (n.getAttribute("tabindex") === "-1") n.remove(); });
  return { html, text: blockText(c) };
}
function plainHtml(c) {
  // Plain text: keep line structure only.
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  return blockText(c).replace(/\n$/, "").split("\n").map((l) => `<div>${l ? esc(l) : "<br>"}</div>`).join("");
}

let snapTimer = null, inputAt = 0;
function changed() {
  const now = Date.now();
  if (now - inputAt > 400) { inputAt = now; dioxus.send({ t: "input" }); }
  clearTimeout(snapTimer);
  snapTimer = setTimeout(() => dioxus.send({ t: "snap", ...snapshot() }), 300);
}

// ------------------------------------------------------------ selection
function inEditor(node) { return node && (node === el || el.contains(node)); }
function rememberSelection() {
  const sel = document.getSelection();
  if (sel && sel.rangeCount && inEditor(sel.anchorNode)) state.saved = sel.getRangeAt(0).cloneRange();
}
function restoreSelection() {
  if (document.activeElement !== el) el.focus({ preventScroll: true });
  const sel = document.getSelection();
  if (state.saved && inEditor(state.saved.startContainer)) {
    sel.removeAllRanges();
    sel.addRange(state.saved);
  } else if (!sel.rangeCount || !inEditor(sel.anchorNode)) {
    caretAt(true);
  }
}
function caretAt(start) {
  const r = document.createRange();
  if (start) {
    // Deepest first container, stopping above void elements (<br>, <img>).
    let n = el;
    while (n.firstChild && !(n.firstChild.nodeType === 1 && /^(BR|IMG|HR|TABLE)$/.test(n.firstChild.tagName))) n = n.firstChild;
    r.setStart(n, 0);
    r.collapse(true);
  } else {
    r.selectNodeContents(el);
    r.collapse(false);
  }
  const sel = document.getSelection();
  sel.removeAllRanges();
  sel.addRange(r);
  state.saved = r.cloneRange();
}
function caretElement() {
  const sel = document.getSelection();
  if (!sel || !sel.rangeCount || !inEditor(sel.anchorNode)) return null;
  const n = sel.anchorNode;
  return n.nodeType === 1 ? n : n.parentElement;
}
function firstFamily(f) { return (f || "").split(",")[0].trim().replace(/^['"]|['"]$/g, ""); }
function q(cmd) { try { return document.queryCommandState(cmd); } catch (e) { return false; } }
function fmtState() {
  const n = caretElement();
  if (!n) return null;
  const cs = getComputedStyle(n);
  let align = cs.textAlign;
  if (align === "start" || align === "-webkit-auto" || align === "auto") align = cs.direction === "rtl" ? "right" : "left";
  if (align === "end") align = cs.direction === "rtl" ? "left" : "right";
  if (align === "-webkit-center") align = "center";
  const block = n.closest("h1,h2,h3,blockquote,pre");
  return {
    bold: q("bold"), italic: q("italic"), underline: q("underline"), strike: q("strikeThrough"),
    sub: q("subscript"), sup: q("superscript"), ul: q("insertUnorderedList"), ol: q("insertOrderedList"),
    align, font: firstFamily(cs.fontFamily), size: Math.round(parseFloat(cs.fontSize) * 0.75 * 2) / 2,
    link: !!n.closest("a"), block: block && el.contains(block) ? block.tagName.toLowerCase() : "div",
    dir: cs.direction,
  };
}
let fmtQueued = false;
function reportFmt() {
  if (fmtQueued) return;
  fmtQueued = true;
  requestAnimationFrame(() => {
    fmtQueued = false;
    const f = fmtState();
    if (!f) return;
    const s = JSON.stringify(f);
    if (s !== state.lastFmt) { state.lastFmt = s; dioxus.send({ t: "fmt", ...f }); }
  });
}
function onSelectionChange() {
  if (!el.isConnected) return teardown();
  const sel = document.getSelection();
  if (sel && sel.rangeCount && inEditor(sel.anchorNode)) {
    rememberSelection();
    reportFmt();
    // The caret left the "@name" being typed: close the list.
    if (state.mention) {
      const m = mentionAtCaret();
      if (!m || m.node !== state.mention.node || m.start !== state.mention.start) endMention();
    }
  }
}

// ------------------------------------------------------------ mentions
function mentionAtCaret() {
  const sel = document.getSelection();
  if (state.plain || !sel || !sel.rangeCount || !sel.isCollapsed || !inEditor(sel.anchorNode)) return null;
  const node = sel.anchorNode;
  if (node.nodeType !== 3 || (node.parentElement && node.parentElement.closest("a"))) return null;
  const before = node.data.slice(0, sel.anchorOffset);
  const m = /(^|[\s\u00a0(])@([^\s@]{0,40})$/u.exec(before);
  if (!m) return null;
  return { node, start: sel.anchorOffset - m[2].length - 1, end: sel.anchorOffset, query: m[2] };
}
function checkMention() {
  const m = mentionAtCaret();
  if (!m) return endMention();
  const prev = state.mention;
  state.mention = m;
  if (prev && prev.query === m.query && prev.node === m.node) return;
  const r = document.createRange();
  r.setStart(m.node, m.start);
  r.setEnd(m.node, m.end);
  const rect = r.getBoundingClientRect();
  dioxus.send({ t: "mention", q: m.query, x: rect.left, y: rect.bottom + 4 });
}
function endMention() {
  if (!state.mention) return;
  state.mention = null;
  state.mentionCount = 0;
  dioxus.send({ t: "mention", q: null });
}
function insertMention(v) {
  const m = state.mention || mentionAtCaret();
  state.mention = null;
  state.mentionCount = 0;
  if (!m || !m.node.isConnected) return;
  const r = document.createRange();
  r.setStart(m.node, Math.min(m.start, m.node.length));
  r.setEnd(m.node, Math.min(m.end, m.node.length));
  r.deleteContents();
  const a = document.createElement("a");
  a.href = "mailto:" + v.address;
  a.setAttribute("data-ax-mention", "");
  a.textContent = "@" + v.name;
  const space = document.createTextNode("\u00a0");
  r.insertNode(space);
  r.insertNode(a);
  const after = document.createRange();
  after.setStartAfter(space);
  after.collapse(true);
  const sel = document.getSelection();
  sel.removeAllRanges();
  sel.addRange(after);
}

// ------------------------------------------------------------ commands
function exec(cmd, val) { try { return document.execCommand(cmd, false, val); } catch (e) { return false; } }
function withCss(on, f) { exec("styleWithCSS", on); try { f(); } finally { exec("styleWithCSS", false); } }
function blocksInSelection() {
  const sel = document.getSelection();
  if (!sel.rangeCount) return [];
  const range = sel.getRangeAt(0);
  const isBlock = (n) => n.nodeType === 1 && /^(DIV|P|LI|H1|H2|H3|BLOCKQUOTE|PRE|TD)$/.test(n.tagName) && n !== el;
  const out = [];
  const start = caretElement();
  const own = start && start.closest("div,p,li,h1,h2,h3,blockquote,pre,td");
  if (own && el.contains(own)) out.push(own);
  if (!range.collapsed) {
    el.querySelectorAll("div,p,li,h1,h2,h3,blockquote,pre,td").forEach((b) => {
      if (isBlock(b) && range.intersectsNode(b) && !b.querySelector("div,p,li,h1,h2,h3,blockquote,pre") && !out.includes(b)) out.push(b);
    });
  }
  if (!out.length) {
    // Bare text directly in the editor: wrap the line in a div first.
    exec("formatBlock", "div");
    const again = caretElement();
    const b = again && again.closest("div,p,li,h1,h2,h3,blockquote,pre");
    if (b && el.contains(b)) out.push(b);
  }
  return out;
}
function setFontSize(pt) {
  withCss(false, () => exec("fontSize", "7"));
  el.querySelectorAll('font[size="7"]').forEach((f) => {
    const span = document.createElement("span");
    span.style.fontSize = pt + "pt";
    for (const a of Array.from(f.attributes)) if (a.name !== "size") span.setAttribute(a.name, a.name === "style" ? span.getAttribute("style") + ";" + a.value : a.value);
    if (f.getAttribute("face")) { span.style.fontFamily = f.getAttribute("face"); span.removeAttribute("face"); }
    if (f.getAttribute("color")) { span.style.color = f.getAttribute("color"); span.removeAttribute("color"); }
    while (f.firstChild) span.appendChild(f.firstChild);
    f.replaceWith(span);
    // Nested explicit sizes inside the new span would win; drop them.
    span.querySelectorAll("[style]").forEach((n) => { n.style.fontSize = ""; if (n.getAttribute("style") === "") n.removeAttribute("style"); });
    const r = document.createRange();
    r.selectNodeContents(span);
    const sel = document.getSelection();
    sel.removeAllRanges();
    sel.addRange(r);
  });
}
function clearColor(prop, cmd) {
  // Apply a sentinel colour, then remove it: "Automatic" / "No color".
  withCss(true, () => exec(cmd, SENTINEL));
  el.querySelectorAll("[style]").forEach((n) => {
    if (n.style[prop] === SENTINEL) n.style[prop] = "";
    if (n.getAttribute("style") === "") n.removeAttribute("style");
    if (n.tagName === "SPAN" && !n.attributes.length) n.replaceWith(...n.childNodes);
  });
  el.querySelectorAll('font[color="#010203"]').forEach((n) => n.removeAttribute("color"));
}
function escapeHtml(s) { return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"); }
function currentLink() {
  const n = caretElement();
  const a = n && n.closest("a");
  return a && el.contains(a) ? a : null;
}
function insertHtml(html) {
  const tmp = document.createElement("div");
  tmp.innerHTML = html;
  clean(tmp);
  exec("insertHTML", tmp.innerHTML);
}
function inList() { const n = caretElement(); return !!(n && n.closest("li")); }

const ops = {
  bold: () => exec("bold"),
  italic: () => exec("italic"),
  underline: () => exec("underline"),
  strike: () => exec("strikeThrough"),
  sub: () => exec("subscript"),
  sup: () => exec("superscript"),
  bullets: () => exec("insertUnorderedList"),
  numbering: () => exec("insertOrderedList"),
  bullet_style: (v) => { if (!q("insertUnorderedList")) exec("insertUnorderedList"); const n = caretElement(); const l = n && n.closest("ul"); if (l) l.style.listStyleType = v; },
  number_style: (v) => { if (!q("insertOrderedList")) exec("insertOrderedList"); const n = caretElement(); const l = n && n.closest("ol"); if (l) l.style.listStyleType = v; },
  indent: () => exec("indent"),
  outdent: () => exec("outdent"),
  left: () => exec("justifyLeft"),
  center: () => exec("justifyCenter"),
  right: () => exec("justifyRight"),
  justify: () => exec("justifyFull"),
  clear: () => { exec("removeFormat"); exec("formatBlock", "div"); },
  // This body's own steps only (WebKit's stack is shared with the other
  // fields of the page; kit/typed_text.js).
  undo: () => (window.axTypedText && axTypedText.history ? axTypedText.history(el, "undo") : exec("undo")),
  redo: () => (window.axTypedText && axTypedText.history ? axTypedText.history(el, "redo") : exec("redo")),
  font: (v) => withCss(true, () => exec("fontName", v)),
  size: (v) => setFontSize(v),
  color: (v) => v === "auto" ? clearColor("color", "foreColor") : withCss(true, () => exec("foreColor", v)),
  highlight: (v) => v === "none" ? clearColor("backgroundColor", "hiliteColor") : withCss(true, () => exec("hiliteColor", v)),
  block: (v) => exec("formatBlock", v),
  line_spacing: (v) => blocksInSelection().forEach((b) => { b.style.lineHeight = v; }),
  dir: (v) => blocksInSelection().forEach((b) => { b.setAttribute("dir", v); }),
  hr: () => exec("insertHorizontalRule"),
  html: (v) => insertHtml(v),
  text: (v) => exec("insertText", v),
  image: (v) => {
    const size = v.width ? ` width="${Math.round(+v.width)}" height="${Math.round(+v.height)}"` : "";
    insertHtml(`<img src="${escapeHtml(v.src)}" alt="${escapeHtml(v.alt || "")}"${size} style="max-width:100%;height:auto">`);
  },
  table: (v) => {
    const w = Math.floor(576 / v.cols);
    const cell = `<td style="width:${w}px;border:1px solid #808080;padding:0 7px;vertical-align:top"><br></td>`;
    const row = `<tr>${cell.repeat(v.cols)}</tr>`;
    insertHtml(`<table style="border-collapse:collapse;border:none"><tbody>${row.repeat(v.rows)}</tbody></table><div><br></div>`);
  },
  link: (v) => {
    const a = currentLink();
    if (a) {
      a.setAttribute("href", v.url);
      if (v.text && v.text !== a.textContent) a.textContent = v.text;
      return;
    }
    const sel = document.getSelection();
    const selected = sel.rangeCount ? sel.toString() : "";
    if (selected && (!v.text || v.text === selected)) exec("createLink", v.url);
    else insertHtml(`<a href="${escapeHtml(v.url)}">${escapeHtml(v.text || v.url)}</a>`);
  },
  unlink: () => {
    const a = currentLink();
    if (a) {
      const r = document.createRange();
      r.selectNodeContents(a);
      const sel = document.getSelection();
      sel.removeAllRanges();
      sel.addRange(r);
    }
    exec("unlink");
  },
  signature: (v) => {
    const existing = el.querySelector("[data-axentra-signature]");
    if (existing) { existing.innerHTML = v; clean(existing); return; }
    insertHtml(`<div id="Signature" data-axentra-signature="default">${v}</div>`);
  },
  plain: (v) => {
    state.plain = !!v;
    if (state.plain) { const c = el.cloneNode(true); undoDark(c); el.innerHTML = plainHtml(c); }
    el.classList.toggle("ax-plain", state.plain);
  },
  mention: (v) => insertMention(v),
  mention_count: (v) => { state.mentionCount = v || 0; if (!state.mentionCount) state.mention = null; },
  focus_start: () => { el.focus({ preventScroll: true }); caretAt(true); el.scrollTop = 0; },
  focus: () => restoreSelection(),
  set_html: (v) => { el.innerHTML = v || "<div><br></div>"; clean(el); darkAdjust(); },
  dark: (v) => { if (v) { state.dark = true; darkAdjust(); } else { undoDark(el); state.dark = false; } },
};
const noRestore = new Set(["focus_start", "set_html", "dark", "plain", "mention_count"]);
// Commands that leave the content alone (no "changed" report, so opening
// a draft or moving the caret never marks it as edited).
const quiet = new Set(["focus_start", "focus", "dark", "mention_count"]);
function run(cmd, val) {
  const op = ops[cmd];
  if (!op) return;
  if (!noRestore.has(cmd)) restoreSelection();
  op(val);
  if (quiet.has(cmd)) return;
  rememberSelection();
  reportFmt();
  changed();
}

// ------------------------------------------------------------ events
function onKey(e) {
  if (e.isComposing || e.keyCode === 229) return;
  const ctrl = e.ctrlKey || e.metaKey, sh = e.shiftKey, alt = e.altKey;
  if (state.mention && state.mentionCount > 0 && !ctrl && !alt && ["ArrowDown", "ArrowUp", "Enter", "Tab", "Escape"].includes(e.key)) {
    // The open mention list gets these keys, not the body or the form.
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") { state.mention = null; state.mentionCount = 0; }
    dioxus.send({ t: "mkey", key: e.key });
    return;
  }
  const letter = e.code && e.code.startsWith("Key") ? e.code.slice(3).toLowerCase() : (e.key || "").toLowerCase();
  const go = (cmd, val) => { e.preventDefault(); run(cmd, val); };
  if (ctrl && !alt) {
    if (!sh && letter === "b") return go("bold");
    if (!sh && letter === "i") return go("italic");
    if (!sh && letter === "u") return go("underline");
    if (sh && letter === "l") return go("bullets");
    if (!sh && letter === "e") return go("center");
    if (!sh && letter === "l") return go("left");
    if (!sh && letter === "r") return go("right");
    if (sh && letter === "j") return go("justify");
    if (!sh && letter === "t") return go("indent");
    if (sh && letter === "t") return go("outdent");
    if (!sh && letter === "z") return go("undo");
    if (!sh && letter === "y") return go("redo");
    if (sh && letter === "z") return go("redo");
    if (!sh && e.code === "Space") return go("clear");
    if (!sh && e.key === "]") return go("size", nextSize(true));
    if (!sh && e.key === "[") return go("size", nextSize(false));
    if (sh && (e.key === ">" || e.code === "Period")) return go("size", nextSize(true));
    if (sh && (e.key === "<" || e.code === "Comma")) return go("size", nextSize(false));
    if (sh && letter === "v") { state.plainNext = true; return; }
  }
  // By code too: WebKitGTK reports Shift+Tab as key "Unidentified".
  if ((e.key === "Tab" || e.code === "Tab") && !ctrl && !alt && inList()) return go(sh ? "outdent" : "indent");
}
const SIZES = [8, 9, 10, 10.5, 11, 12, 14, 16, 18, 20, 24, 28, 36, 48, 72];
function nextSize(up) {
  const f = fmtState();
  const cur = f ? f.size : 12;
  if (up) return SIZES.find((s) => s > cur + 0.01) || 72;
  return SIZES.slice().reverse().find((s) => s < cur - 0.01) || 8;
}
function onRootKey(e) {
  // Browser defaults that would fire before the app sees these keys.
  const ctrl = e.ctrlKey || e.metaKey;
  const letter = e.code && e.code.startsWith("Key") ? e.code.slice(3).toLowerCase() : "";
  if ((ctrl && !e.altKey && (letter === "s" || letter === "k" || e.key === "Enter")) || (e.altKey && !ctrl && letter === "s")) e.preventDefault();
}
function onPaste(e) {
  const dt = e.clipboardData;
  if (!dt) return;
  const images = Array.from(dt.items || []).filter((i) => i.kind === "file" && i.type.startsWith("image/"));
  if (images.length && !state.plain) {
    e.preventDefault();
    images.forEach((i) => {
      const r = new FileReader();
      r.onload = () => run("image", { src: r.result, alt: "" });
      r.readAsDataURL(i.getAsFile());
    });
    return;
  }
  if (state.plainNext || state.plain || !dt.types.includes("text/html")) {
    e.preventDefault();
    state.plainNext = false;
    exec("insertText", dt.getData("text/plain"));
    return;
  }
  setTimeout(() => { clean(el); darkAdjust(); changed(); }, 0);
}
// Files dropped anywhere on the form become attachments: read here and
// sent to Rust (works the same in the desktop webview and in liveview).
const MAX_FILE = 25 * 1024 * 1024;
let dragDepth = 0;
const hasFiles = (e) => !!(e.dataTransfer && Array.from(e.dataTransfer.types || []).includes("Files"));
function onDragEnter(e) {
  if (!hasFiles(e)) return;
  e.preventDefault();
  dragDepth++;
  root.setAttribute("data-drag", "");
}
function onDragOver(e) {
  if (!hasFiles(e)) return;
  e.preventDefault();
  e.dataTransfer.dropEffect = "copy";
}
function onDragLeave(e) {
  if (!hasFiles(e)) return;
  dragDepth = Math.max(0, dragDepth - 1);
  if (!dragDepth) root.removeAttribute("data-drag");
}
function onDrop(e) {
  if (!hasFiles(e)) return;
  e.preventDefault();
  e.stopPropagation();
  dragDepth = 0;
  root.removeAttribute("data-drag");
  Array.from(e.dataTransfer.files || []).forEach((f) => {
    if (f.size > MAX_FILE) { dioxus.send({ t: "file", name: f.name, type: f.type, size: f.size, data: null }); return; }
    const r = new FileReader();
    r.onload = () => {
      const s = String(r.result || "");
      dioxus.send({ t: "file", name: f.name, type: f.type, size: f.size, data: s.slice(s.indexOf(",") + 1) });
    };
    r.onerror = () => dioxus.send({ t: "file", name: f.name, type: f.type, size: f.size, data: null });
    r.readAsDataURL(f);
  });
}
function onClick(e) {
  // Links in the body are edited, not followed (Ctrl+click follows them).
  // Stop the click here: the desktop shell opens any clicked link.
  if (e.target.closest && e.target.closest("a") && !e.ctrlKey) {
    e.preventDefault();
    e.stopPropagation();
  }
}

el.addEventListener("input", () => { changed(); checkMention(); });
el.addEventListener("keydown", onKey);
el.addEventListener("keyup", reportFmt);
el.addEventListener("mouseup", reportFmt);
el.addEventListener("paste", onPaste);
el.addEventListener("click", onClick);
el.addEventListener("blur", () => {
  clearTimeout(snapTimer);
  dioxus.send({ t: "snap", ...snapshot() });
  // Leaving the body closes the mention list (unless focus comes back).
  setTimeout(() => { if (document.activeElement !== el) endMention(); }, 300);
});
document.addEventListener("selectionchange", onSelectionChange);
// Focus leaving the form (not just moving between its fields) saves at once.
function onRootFocusOut() {
  setTimeout(() => {
    const a = document.activeElement;
    if (state.alive && root.isConnected && !(a && root.contains(a)) && !(a && a.closest && a.closest(".ax-dialog-overlay, .ax-menu, .ax-compose-flyout"))) {
      dioxus.send({ t: "leave" });
    }
  }, 0);
}
if (root) {
  root.addEventListener("focusout", onRootFocusOut);
  root.addEventListener("keydown", onRootKey);
  root.addEventListener("dragenter", onDragEnter);
  root.addEventListener("dragover", onDragOver);
  root.addEventListener("dragleave", onDragLeave);
  root.addEventListener("drop", onDrop);
}
// Follow theme changes (app setting or the desktop's light/dark switch).
const themeChanged = () => { const d = computeDark(); if (d !== state.dark) ops.dark(d); };
const themeObserver = new MutationObserver(themeChanged);
if (appEl) themeObserver.observe(appEl, { attributes: true, attributeFilter: ["class"] });
const media = window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null;
if (media && media.addEventListener) media.addEventListener("change", themeChanged);
function teardown() {
  if (!state.alive) return;
  state.alive = false;
  themeObserver.disconnect();
  if (media && media.removeEventListener) media.removeEventListener("change", themeChanged);
  document.removeEventListener("selectionchange", onSelectionChange);
  clearInterval(state.timer);
  // Kept a little longer: a form that has just gone reads its body once
  // more (the editor is detached but still holds the text).
  setTimeout(() => { if (window.axComposeEditors && window.axComposeEditors[cfg.id] === api) delete window.axComposeEditors[cfg.id]; }, 10000);
  if (state.finish) state.finish();
}

// Entry points for Rust's one-shot evals.
const api = {
  run,
  snapshot,
  getLink: () => {
    const a = currentLink();
    if (a) return { text: a.textContent, url: a.getAttribute("href") || "", editing: true };
    const sel = document.getSelection();
    return { text: sel && sel.rangeCount && inEditor(sel.anchorNode) ? sel.toString() : "", url: "", editing: false };
  },
};
window.axComposeEditors = window.axComposeEditors || {};
window.axComposeEditors[cfg.id] = api;

el.innerHTML = cfg.html || "<div><br></div>";
clean(el);
if (state.plain) { el.classList.add("ax-plain"); }
darkAdjust();
dioxus.send({ t: "ready", ...snapshot() });

// Stay alive while the editor is on the page: returning would close the
// channel that `dioxus.send` reports through.
await new Promise((resolve) => {
  state.finish = resolve;
  state.timer = setInterval(() => { if (!el.isConnected) teardown(); }, 3000);
});
