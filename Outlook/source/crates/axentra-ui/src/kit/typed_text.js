// Typed-text guard, installed with the shell runtime (kit::dom::install).
//
// Text fields are controlled from Rust. Each key goes to the app at once
// (the desktop webview sends events as synchronous requests, so the app
// has a key's value before the next key), but the field's value comes back
// with the app's next render, and the webview applies renders one batch at
// a time: a render can show the field as it was a few keys ago. Taking that
// value would put older text back, so newer letters are lost and the caret
// jumps to the end.
//
// The guard sits where the page applies the app's renders
// (`window.interpreter`): every value the app writes to a text field passes
// through it, writes that change nothing included. Each field remembers,
// oldest first, what the user's input made it and how many batches had been
// applied by then. The app makes a batch only after the page has applied
// the previous one, so a batch has seen every key that came before the
// previous batch was applied. A write is a late echo, and is skipped, when
// it is a value the field held from just before the keys the batch may not
// have seen, and the field still shows the newest typed text. Any other
// write (cleared, a chip made, a suggestion picked, a value set from
// elsewhere) is the app's own and goes through.
//
// A skipped write can still be the app's own (it set the field back to the
// value it had before those keys). So after a skip the page asks the app
// for a render barrier (`kit::dom::TypedTextSync`): once the barrier's batch
// is applied, the app has rendered everything after those keys, and its
// last value for the field wins.
//
// Fields that a key commits and empties are emptied here as the key goes
// down, before the next key can land behind the old text:
// * `data-ax-enter-clears`: Enter, when the app took it (preventDefault),
//   e.g. To Do's "Add a task". A failed add keeps the text, selected.
// * `data-ax-escape-clears`: Escape, when the app took it (the header
//   search box, when Escape ends the search).
// `data-ax-enter-keeps-typing` (the header search box): keys typed straight
// after an Enter the app took go back to the field even when the app has
// moved focus on; `data-ax-tab-keeps-typing` (the message body, rich text):
// the same after a Tab that left it while typing there (see the stray-key
// handling below).
// * `data-ax-address-list` (recipient and attendee boxes): Enter and Tab
//   with text in the box; `;` or `,` when the app took the key ("keys"), or
//   typed at the end outside quotes and angle brackets ("split", where the
//   app splits what is typed); in "keys" boxes also a pasted list, which
//   the app adds whole.
//
// Keys pressed in a text field before a click, but delivered after it (a
// busy page), act in that field, chords included, by their press times
// (timeStamp) on a timeline of what had focus before each click.
//
// Also here: Ctrl+Z undoes and Ctrl+Y or Ctrl+Shift+Z redoes (WebKitGTK binds
// no keys for them). WebKit keeps one undo stack for the whole page, so its
// undo in a field with nothing left to undo reverts the last edit in some
// other field (two Ctrl+Z in Subject emptied the message body). Text inputs
// and text areas therefore keep their own history here, one per field;
// rich-text fields (`history()`, also used by the compose body) use
// WebKit's stack but skip the steps of other fields, and a pause there
// ends a step. Further:
// `quiet()`, which lets compose actions wait until keys that are still
// arriving have landed before they read the form, and `hold()` /
// `release()`, which carry keys over to the same field when a form is
// replaced (compose pop-out and back).
(function (g) {
  if (g.axTypedText) return;
  const MAX_LOG = 400;
  const BARRIER_TIMEOUT_MS = 5000;
  const NO_BARRIER_SETTLE_MS = 1500;

  // ------------------------------------------------------------ decisions
  // The logic without the DOM (tested in gjs). A field's state is
  // { base, log: [{ v, e }], app }: `base` the value the app had before the
  // keys in `log`; each entry the value one key (or a page-made edit) gave
  // the field and the number of batches applied by then; `app` the last
  // value the app wrote.

  // The app writes `w` while the field shows `cur`, `batches` batches having
  // been applied. `epochs`: batches prove what the app has seen (desktop;
  // liveview sends events without waiting for them). True: skip the write.
  // An empty log afterwards means the field and the app agree: forget it.
  const appWrite = (st, w, cur, batches, epochs) => {
    st.app = w;
    if (epochs) {
      // Seen by this batch: every key before the previous batch was applied.
      let k = 0;
      while (k < st.log.length && st.log[k].e < batches) k++;
      if (k) {
        st.base = st.log[k - 1].v;
        st.log.splice(0, k);
      }
    }
    if (!st.log.length) return false;
    if (cur !== st.log[st.log.length - 1].v) {
      // Changed by something else since the last key.
      st.log.length = 0;
      return false;
    }
    if (w === st.base) return true;
    const i = st.log.findIndex((x) => x.v === w);
    if (i < 0) {
      st.log.length = 0;
      return false;
    }
    st.base = w;
    st.log.splice(0, i + 1);
    // An echo of the newest key: the app caught up.
    return st.log.length > 0;
  };

  // A barrier came back after the field's last key: what the field should
  // show now (the app's last value), or null to leave it as it is.
  const settle = (st, cur) => {
    const newest = st.log.length ? st.log[st.log.length - 1].v : undefined;
    st.log.length = 0;
    if (newest === undefined || cur !== newest || st.app === undefined || st.app === cur) return null;
    return st.app;
  };

  // Entries of an address list that a separator (";" or "," outside quotes
  // and angle brackets) has ended, as compose/address.rs parse_list splits
  // them: [how many, whether the text ends with one].
  const endedEntries = (text) => {
    let quoted = false, angle = false, escaped = false, entry = "", ended = false, n = 0;
    for (const c of text) {
      ended = false;
      if (escaped) {
        entry += c;
        escaped = false;
      } else if (c === "\\" && quoted) {
        entry += c;
        escaped = true;
      } else if (c === '"') {
        quoted = !quoted;
        entry += c;
      } else if (c === "<" && !quoted) {
        angle = true;
        entry += c;
      } else if (c === ">" && !quoted) {
        angle = false;
        entry += c;
      } else if (c === ":" && !quoted && !angle && !entry.includes("@")) {
        entry = "";
      } else if ((c === ";" || c === ",") && !quoted && !angle) {
        ended = entry.trim() !== "";
        if (ended) n++;
        entry = "";
      } else entry += c;
    }
    return [n, ended];
  };
  // "split" address boxes: the text ends with an entry separator after a
  // non-empty entry; the app then makes the chip and keeps nothing in the box.
  const entryEnded = (text) => endedEntries(text)[1];
  // "keys" address boxes (event attendees): a paste (the box grew by more
  // than one character) that holds an ended entry; the app then adds every
  // entry in it (calendar/fields.rs pasted_list) and keeps nothing.
  const pasteEnds = (before, now) => [...now].length > [...before].length + 1 && endedEntries(now)[0] > 0;

  // Keys typed "straight on" after Enter (the header search box) or Tab
  // (the message body) moved focus to the results or the Send button: the
  // first within ENTER_GAP_MS of that key, the rest each within STRAY_GAP_MS
  // of the one before, by when they were pressed. `st` = { ts, keys, last,
  // enter, edits }; `enter`: Enter counts as typing too (a new line in the
  // body); `edits`: so do Backspace and Delete (fixing the search just
  // made, where the results would archive or delete the top message).
  // "type": the key goes back to the field; "end": typing went elsewhere
  // (any other key, a chord, a pause), forget it; "wait": a modifier, decide
  // on the next key.
  const STRAY_GAP_MS = 250;
  const ENTER_GAP_MS = 600;
  const MODIFIER_KEYS = ["Shift", "Control", "Alt", "AltGraph", "Meta", "OS", "Super", "Hyper", "CapsLock", "NumLock", "Dead", "Process", "Compose"];
  const straightOn = (st, key, ts, chord) => {
    if (!key || MODIFIER_KEYS.includes(key)) return "wait";
    if (chord || !(key.length === 1 || (st.enter && key === "Enter") || (st.edits && (key === "Backspace" || key === "Delete")))) return "end";
    const gap = ts - (st.keys ? st.last : st.ts);
    if (!(gap >= 0 && gap <= (st.keys ? STRAY_GAP_MS : ENTER_GAP_MS))) return "end";
    st.keys++;
    st.last = ts;
    return "type";
  };
  // The focus timeline: one entry per click, oldest first, `until` its
  // press time, with what had focus up to it. For a key pressed at `t` (its
  // timeStamp, or null), the entry of the focus it was pressed in, or null
  // when it came after every click (it goes where focus is). Keys arrive in
  // the order they were pressed, so entries a key is past are dropped.
  const pressedIn = (timeline, t) => {
    if (t === null) timeline.length = 0;
    while (timeline.length && timeline[0].until <= t) timeline.shift();
    return timeline[0] || null;
  };
  // What a key pressed in a text field does there when it arrives after a
  // click: "type" (a character, Backspace, Delete), "word" (with Ctrl),
  // "undo" / "redo" (its own history), "paste" (Ctrl+V); "drop": other
  // plain keys and text chords, never acted on where not aimed; "through":
  // app shortcuts (Ctrl+S, Ctrl+1, Alt+...), wherever focus is.
  const lateKind = (key, ctrl, alt, shift) => {
    const k = key.length === 1 ? key.toLowerCase() : key;
    if (alt) return "through";
    if (!ctrl) return key.length === 1 || k === "Backspace" || k === "Delete" ? "type" : "drop";
    if (k === "Backspace" || k === "Delete") return "word";
    if (k === "z" || (k === "y" && !shift)) return k === "y" || shift ? "redo" : "undo";
    if (k === "v") return "paste";
    return /^([acx]|Arrow.*|Home|End)$/.test(k) ? "drop" : "through";
  };
  // Whether an event's timeStamp `t` is a press time on the page's clock at
  // `now` (performance.now()). Until one event has shown it is (`known`: it
  // arrived within CLOCK_PROVEN_MS of its timeStamp), only times up to
  // CLOCK_UNKNOWN_MS old are trusted; after that, up to LATE_MAX_MS: a page
  // more than 10 s behind (a very busy machine) still knows which keys came
  // before a click.
  const CLOCK_PROVEN_MS = 2000;
  const CLOCK_UNKNOWN_MS = 10000;
  const LATE_MAX_MS = 120000;
  // `key` (text, "Backspace" or "Delete"; `word`: a word, as with Ctrl)
  // typed into text `v` with the selection s..e, as a text field takes it:
  // { v, at, from, to, text } (the new text, the caret after it, and the
  // range replaced by `text`), or null when it changes nothing (Backspace
  // at the start, a full field). Surrogate pairs count as one character.
  const editAt = (v, s, e, key, max, word) => {
    if (s === null || e === null || s === undefined || e === undefined) s = e = v.length;
    let text = key;
    const space = (c) => /\s/.test(c);
    if (key === "Backspace" || key === "Delete") {
      text = "";
      if (s === e) {
        if (key === "Backspace") {
          if (s === 0) return null;
          if (!word) s -= s > 1 && /[\uDC00-\uDFFF]/.test(v[s - 1]) && /[\uD800-\uDBFF]/.test(v[s - 2]) ? 2 : 1;
          else for (const skip of [true, false]) while (s > 0 && space(v[s - 1]) === skip) s--;
        } else {
          if (e >= v.length) return null;
          if (!word) e += /[\uD800-\uDBFF]/.test(v[e]) && /[\uDC00-\uDFFF]/.test(v[e + 1] || "") ? 2 : 1;
          else for (const skip of [true, false]) while (e < v.length && space(v[e]) === skip) e++;
        }
      }
    } else if (max >= 0 && v.length - (e - s) + key.length > max) return null;
    return { v: v.slice(0, s) + text + v.slice(e), at: s + text.length, from: s, to: e, text };
  };
  const onPageClock = (t, now, known) => typeof t === "number" && t > 0 && t <= now + 50 && now - t <= (known ? LATE_MAX_MS : CLOCK_UNKNOWN_MS);
  const provesClock = (t, now) => onPageClock(t, now, false) && now - t <= CLOCK_PROVEN_MS;

  // Undo history of one text field: { undo: [{ v, s, e }], redo, post,
  // kind, at }. `post` is the value after the last edit it knows of: when
  // the field shows anything else, something other than an edit set it (the
  // app, a chip made) and the history starts over. Typing (or deleting) in
  // a run, each key within MERGE_MS of the one before, is one step.
  const UNDO_MAX = 100;
  const MERGE_MS = 1000;
  const freshHistory = (v) => ({ undo: [], redo: [], post: v, kind: null, at: 0 });
  // An edit of `kind` ("type", "delete" or "other") is about to change a
  // field showing `v` (selection s..e) at `now`; `same`: the last edit on the
  // page was in this field. Returns the field's history.
  const willEdit = (h, v, s, e, kind, now, same) => {
    if (!h || h.post !== v) h = freshHistory(v);
    const merge = kind !== "other" && kind === h.kind && same && now - h.at < MERGE_MS && h.undo.length > 0;
    if (!merge) {
      h.undo.push({ v, s, e });
      if (h.undo.length > UNDO_MAX) h.undo.shift();
    }
    h.redo.length = 0;
    h.kind = kind;
    h.at = now;
    return h;
  };
  // Ctrl+Z ("undo") or Ctrl+Y ("redo") in a field showing `v` (selection
  // s..e): the { v, s, e } to put back, or null for nothing to do (never
  // another field's edit).
  const historyStep = (h, cmd, v, s, e) => {
    if (!h || h.post !== v) return null;
    const from = cmd === "undo" ? h.undo : h.redo;
    const to = cmd === "undo" ? h.redo : h.undo;
    while (from.length && from[from.length - 1].v === v) from.pop();
    const st = from.pop();
    if (!st) return null;
    to.push({ v, s, e });
    h.post = st.v;
    h.kind = null;
    return st;
  };
  const editKind = (t) =>
    t === "insertText" || t === "insertCompositionText" ? "type" : t && t.startsWith("delete") && t !== "deleteByCut" && t !== "deleteByDrag" ? "delete" : "other";

  const api = (g.axTypedText = { appWrite, settle, entryEnded, endedEntries, pasteEnds, straightOn, pressedIn, lateKind, onPageClock, provesClock, editAt, willEdit, historyStep, editKind, skipped: 0, settled: 0 });
  if (typeof document === "undefined" || typeof HTMLInputElement === "undefined") return;

  // ------------------------------------------------------------ page
  const TEXT_TYPES = ["text", "search", "email", "url", "tel", "password", "number", "date", "time", "datetime-local", "month", "week"];
  const isText = (el) =>
    el instanceof HTMLTextAreaElement || (el instanceof HTMLInputElement && TEXT_TYPES.includes((el.type || "text").toLowerCase()));
  const desc = (el) => Object.getOwnPropertyDescriptor(el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, "value");
  const getValue = (el) => desc(el).get.call(el);
  const setValue = (el, v) => desc(el).set.call(el, v);

  const states = new WeakMap();
  const pending = new Set();
  const before = new WeakMap();
  let batches = 0;
  let seq = 0;
  let lastKeyAt = 0;
  let epochs = true;

  const record = (el, v, base) => {
    let st = states.get(el);
    if (!st) states.set(el, (st = { base, log: [], app: undefined, s: 0 }));
    st.log.push({ v, e: batches });
    if (st.log.length > MAX_LOG) st.base = st.log.shift().v;
    st.s = ++seq;
  };
  // A page-made edit (a box emptied by its commit key), recorded like
  // typing so late echoes of the text it replaced stay skipped.
  const setTyped = (el, v) => {
    const was = getValue(el);
    setValue(el, v);
    record(el, v, was);
  };

  // ---- render barrier
  let pinger = null;
  let pings = 0;
  let out = 0;
  let outAt = 0;
  let upToOut = 0;
  let barrierSeen = 0;
  let fallback = 0;
  const resolve = (upTo) => {
    for (const el of pending) {
      const st = states.get(el);
      if (!st || !el.isConnected) {
        pending.delete(el);
        continue;
      }
      // Typed since the barrier was asked for: the next one decides.
      if (st.s > upTo) continue;
      const v = settle(st, getValue(el));
      states.delete(el);
      pending.delete(el);
      if (v !== null) {
        setValue(el, v);
        api.settled++;
      }
    }
    if (pending.size) wantBarrier();
  };
  const wantBarrier = () => {
    if (!pinger) {
      // Not connected (yet): settle once the app has been quiet a while.
      clearTimeout(fallback);
      fallback = setTimeout(() => resolve(seq), NO_BARRIER_SETTLE_MS);
      return;
    }
    if (out && Date.now() - outAt < BARRIER_TIMEOUT_MS) return;
    out = ++pings;
    outAt = Date.now();
    upToOut = seq;
    try {
      pinger(out);
    } catch (_) {
      out = 0;
    }
  };
  api.connect = (send) => {
    pinger = send;
    if (pending.size) wantBarrier();
  };

  // ---- the app's writes
  const onWrite = (node, w) => {
    const st = states.get(node);
    if (!st) return false;
    const skip = appWrite(st, w, getValue(node), batches, epochs);
    if (!st.log.length) {
      states.delete(node);
      pending.delete(node);
    } else if (skip) {
      pending.add(node);
      wantBarrier();
    }
    if (skip) api.skipped++;
    return skip;
  };
  const hook = (I) => {
    if (!I || typeof I.setAttributeInner !== "function" || typeof I.run_from_bytes !== "function") return false;
    if (I.__axTypedText) return true;
    I.__axTypedText = true;
    epochs = !I.liveview;
    const setAttr = I.setAttributeInner;
    I.setAttributeInner = function (node, field, value, ns) {
      if (!ns && field === "value" && isText(node) && onWrite(node, String(value))) return;
      if (!ns && field === "data-ax-typed-sync") barrierSeen = Math.max(barrierSeen, Number(value) || 0);
      return setAttr.call(this, node, field, value, ns);
    };
    const run = I.run_from_bytes;
    I.run_from_bytes = function () {
      try {
        return run.apply(this, arguments);
      } finally {
        batches++;
        if (out && barrierSeen >= out) {
          out = 0;
          resolve(upToOut);
        }
      }
    };
    return true;
  };
  if (!hook(g.interpreter)) {
    let tries = 0;
    const retry = () => {
      if (!hook(g.interpreter) && ++tries < 200) setTimeout(retry, 25);
    };
    retry();
  }

  // ---- undo history (see willEdit / historyStep)
  const histories = new WeakMap();
  let lastEdited = null;
  const selection = (el) => {
    try {
      return [el.selectionStart, el.selectionEnd];
    } catch (_) {
      return [null, null];
    }
  };
  const isHistory = (t) => t === "historyUndo" || t === "historyRedo";
  // Rich-text fields: how many of their own undos can be redone (a new edit
  // ends that, as it would anywhere else; WebKitGTK keeps the old steps).
  const redoable = new WeakMap();

  // ---- what the user does
  // Each text field's value just before the latest edit.
  const prior = new WeakMap();
  document.addEventListener(
    "beforeinput",
    (e) => {
      const el = e.target;
      if (!isText(el)) return;
      if (!states.has(el)) before.set(el, getValue(el));
      prior.set(el, getValue(el));
      if (isHistory(e.inputType)) return;
      const [s, end] = selection(el);
      histories.set(el, willEdit(histories.get(el), getValue(el), s, end, editKind(e.inputType), Date.now(), lastEdited === el));
      lastEdited = el;
    },
    true
  );
  // When the platform says the event was pressed (its timeStamp), or null
  // when that is not on the page's clock (then only the order the page got
  // events in is known).
  let clockKnown = false;
  const pressedAt = (e) => {
    const now = performance.now();
    const t = e.timeStamp;
    if (!clockKnown && provesClock(t, now)) clockKnown = true;
    return onPageClock(t, now, clockKnown) ? t : null;
  };
  // Put `key` (as editAt takes it) into text field `el` at its own
  // selection, without focusing it, as typed input (an undo step too).
  const typeInto = (el, key, word) => {
    if (el.readOnly || el.disabled) return;
    const v = getValue(el);
    const [s, end] = selection(el);
    const n = editAt(v, s, end, key, el.maxLength, word);
    if (!n) return;
    const del = key === "Backspace" || key === "Delete";
    const inputType = del ? (word ? "deleteWord" : "deleteContent") + (key === "Backspace" ? "Backward" : "Forward") : key.length > 1 ? "insertFromPaste" : "insertText";
    if (!states.has(el)) before.set(el, v);
    histories.set(el, willEdit(histories.get(el), v, s, end, editKind(inputType), Date.now(), lastEdited === el));
    lastEdited = el;
    try {
      el.setRangeText(n.text, n.from, n.to, "end");
    } catch (_) {
      setValue(el, n.v);
    }
    el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType, data: n.text || null }));
  };
  // Keys pressed in a text field before a click, but delivered after it: a
  // busy page queues keys and WebKit hands it the later click first. Each
  // key acts in the field that had focus when it was pressed (`pressedIn`,
  // `lateKind`), never in the one clicked. The click can also remove the
  // field (another task opened in To Do): a field marked
  // `data-ax-late-keys="<prefix>:<rest>"` then still takes the keys, and
  // the page hands its value to the app (`lateTo`, see
  // kit::dom::use_late_keys), which saves it where the field would have.
  // Entries { until, el, v, s, e }: the click's press time, the text field
  // focused up to it (or null), its value and selection with late keys.
  const timeline = [];
  const LATE_SEND_MS = 100;
  const lateSenders = new Map();
  const lateTimers = new WeakMap();
  api.lateTo = (prefix, send) => lateSenders.set(prefix, send);
  const lateTag = (el) => (el.getAttribute && el.getAttribute("data-ax-late-keys")) || null;
  // `value`: the field as the user left it at the click with the late keys
  // in it (`v` above), not the element's value: once the app has dropped
  // the field it renders it no more, and a settled render barrier can put
  // an older value of the app's back into the element (seen under load:
  // " r" typed before the click undone, then "e100" typed after it).
  const sendLate = (el, value) => {
    const tag = lateTag(el);
    const send = tag && lateSenders.get(tag.split(":")[0]);
    if (!send) return;
    clearTimeout(lateTimers.get(el));
    lateTimers.set(el, setTimeout(() => send([tag, value]), LATE_SEND_MS));
  };
  // A late key of `kind` for timeline entry `c` (`text`: key or pasted text).
  const lateEdit = (c, kind, text) => {
    const el = c.el;
    if ((kind === "undo" || kind === "redo") && el.isConnected) undoIn(el, kind);
    if (kind !== "type" && kind !== "word") return;
    if (el instanceof HTMLInputElement) text = text.replace(/\r?\n/g, " ");
    if (el.isConnected) typeInto(el, text, kind === "word");
    const n = editAt(c.v, c.s, c.e, text, el.maxLength, kind === "word");
    if (n) {
      c.v = n.v;
      c.s = c.e = n.at;
      sendLate(el, c.v);
    }
  };
  // A late Ctrl+V goes on for the webview to read the clipboard (its paste
  // event comes at once, also on a button): the text goes into the field.
  let latePaste = null;
  document.addEventListener("paste", (e) => {
    const c = latePaste;
    latePaste = null;
    if (!c) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    const text = (e.clipboardData && e.clipboardData.getData("text/plain")) || "";
    if (text) lateEdit(c, "type", text);
  }, true);
  // After Enter in a field marked `data-ax-enter-keeps-typing` (the header
  // search box), when the app took the key and moves focus to the results,
  // and after Tab out of a rich-text field marked `data-ax-tab-keeps-typing`
  // (the message body) while typing there: keys typed straight on (see
  // `straightOn`) go back to the field, and focus with them. The search is
  // refined instead of Space opening a result; the message goes on instead
  // of Space or Enter pressing Send. Any other key, a click or a pause ends
  // it.
  let straight = null;
  let lastRich = null;
  let richPause = 0;
  const TYPED_BEFORE_TAB_MS = 1000;
  document.addEventListener("pointerdown", (e) => {
    straight = latePaste = null;
    const ts = pressedAt(e);
    const a = document.activeElement;
    const el = a && isText(a) && !(a.readOnly || a.disabled) ? a : null;
    const [s, end] = el ? selection(el) : [null, null];
    if (ts === null) timeline.length = 0;
    else timeline.push({ until: ts, el, v: el ? getValue(el) : "", s, e: end });
  }, true);
  const chordOf = (e) => e.ctrlKey || e.altKey || e.metaKey;
  // Focus the field again for keys typed straight on: a text field with
  // the caret at the end, a rich-text field where its caret was.
  const refocus = (p) => {
    const el = p.from;
    const had = document.activeElement === el;
    el.focus({ preventScroll: true });
    if (isText(el)) {
      const n = getValue(el).length;
      try {
        el.setSelectionRange(n, n);
      } catch (_) {}
      return;
    }
    // Focusing it puts the caret at the start: put it back where it was.
    if (!had && p.range) {
      const sel = getSelection();
      sel.removeAllRanges();
      sel.addRange(p.range);
    }
  };
  const keepRange = (p) => {
    const sel = getSelection();
    if (!isText(p.from) && sel.rangeCount && p.from.contains(sel.getRangeAt(0).startContainer)) p.range = sel.getRangeAt(0).cloneRange();
  };
  g.addEventListener(
    "keydown",
    (e) => {
      const t = e.target;
      const c = pressedIn(timeline, pressedAt(e));
      const kind = c && c.el && t !== c.el && !e.isComposing && !MODIFIER_KEYS.includes(e.key) && (c.el.isConnected || lateTag(c.el)) ? lateKind(e.key || "", e.ctrlKey || e.metaKey, e.altKey, e.shiftKey) : "through";
      latePaste = kind === "paste" ? c : null;
      if (kind !== "through") {
        lastKeyAt = Date.now();
        if (kind === "paste") return;
        e.preventDefault();
        e.stopImmediatePropagation();
        return lateEdit(c, kind, e.key);
      }
      // Keys typed straight on: in the field while focus is still there,
      // else focus goes back to it with the key.
      const p = straight;
      if (!p || e.isComposing) return;
      const el = p.from;
      const key = e.code === "Tab" ? "Tab" : e.key;
      const verdict = el.isConnected ? straightOn(p, key, pressedAt(e) ?? performance.now(), chordOf(e)) : "end";
      if (verdict === "wait") return;
      if (verdict === "end") {
        straight = null;
        return;
      }
      if (t === el || el.contains(t)) return;
      if (isText(t) || (t && t.isContentEditable)) {
        straight = null;
        return;
      }
      e.preventDefault();
      e.stopImmediatePropagation();
      lastKeyAt = Date.now();
      refocus(p);
      if (key === "Enter") document.execCommand("insertParagraph");
      else if (key === "Backspace" || key === "Delete") document.execCommand(key === "Delete" ? "forwardDelete" : "delete");
      else document.execCommand("insertText", false, key);
      keepRange(p);
    },
    true
  );
  // While keys typed straight on are still coming, the app's move of focus
  // (to the search results) is put off: focus stays in the field.
  document.addEventListener(
    "focusin",
    (e) => {
      const p = straight;
      if (!p || !p.keys || e.target === p.from || p.from.contains(e.target) || isText(e.target) || !p.from.isConnected || Date.now() - lastKeyAt > STRAY_GAP_MS) return;
      p.from.focus({ preventScroll: true });
    },
    true
  );
  document.addEventListener(
    "input",
    (e) => {
      const el = e.target;
      lastKeyAt = Date.now();
      if (!isText(el)) {
        if (el && el.isContentEditable) {
          if (!isHistory(e.inputType)) redoable.delete(el);
          lastRich = { el, at: lastKeyAt };
          // A pause ends WebKit's typing step (setting the selection does),
          // so Ctrl+Z undoes one burst at a time there, as in text fields.
          clearTimeout(richPause);
          if (!e.isComposing && !isHistory(e.inputType))
            richPause = setTimeout(() => {
              const sel = getSelection();
              if (!sel.rangeCount || !el.contains(sel.anchorNode)) return;
              const r = sel.getRangeAt(0);
              sel.removeAllRanges();
              sel.addRange(r);
            }, MERGE_MS);
        }
        return;
      }
      const h = histories.get(el);
      if (h) h.post = getValue(el);
      const base = before.get(el);
      before.delete(el);
      record(el, getValue(el), base);
    },
    true
  );
  // A list pasted into an address box one per line (a column from a
  // sheet; WebKit would join the lines with spaces into one address) or
  // with spaces only between addresses: pasted as "a; b; c", which the app
  // splits.
  document.addEventListener("paste", (e) => {
    const el = e.target, text = (e.clipboardData && e.clipboardData.getData("text/plain")) || "";
    if (!isText(el) || !el.hasAttribute("data-ax-address-list")) return;
    const lines = text.split(/[\r\n\t]+/).map((l) => l.trim()).filter(Boolean);
    const words = text.trim().split(/\s+/);
    const list = lines.length > 1 ? lines : words.length > 1 && words.every((w) => w.includes("@") && !/[<>"]/.test(w)) ? words : null;
    if (!list) return;
    e.preventDefault();
    document.execCommand("insertText", false, list.join("; "));
  });
  // After the app has handled the input (bubbling, past the app's root).
  document.addEventListener("input", (e) => {
    const el = e.target;
    if (!isText(el) || e.isComposing) return;
    const list = el.getAttribute("data-ax-address-list");
    const v = getValue(el);
    if (list === "split" && (v.endsWith(";") || v.endsWith(",")) && entryEnded(v)) setTyped(el, "");
    // A pasted list in a "keys" box: the app added every entry in it, so
    // an address typed straight on starts a new one.
    else if (list === "keys" && prior.has(el) && pasteEnds(prior.get(el), v)) setTyped(el, "");
  });
  for (const t of ["keydown", "keyup", "compositionend"]) document.addEventListener(t, () => (lastKeyAt = Date.now()), true);

  // Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z.
  const undoCommand = (e) => {
    const k = (e.key || "").toLowerCase();
    return k === "z" ? (e.shiftKey ? "redo" : "undo") : k === "y" && !e.shiftKey ? "redo" : null;
  };
  // In a text input or text area, focused or not: its own history, put
  // back as an edit (late echoes of the text it replaces stay skipped).
  const undoIn = (el, cmd) => {
    if (el.readOnly || el.disabled) return;
    const v = getValue(el);
    const [s, end] = selection(el);
    const st = historyStep(histories.get(el), cmd, v, s, end);
    if (!st) return;
    if (!states.has(el)) before.set(el, v);
    setValue(el, st.v);
    try {
      if (st.s !== null) el.setSelectionRange(st.s, st.e);
    } catch (_) {}
    lastEdited = el;
    el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: cmd === "undo" ? "historyUndo" : "historyRedo" }));
  };
  // In rich-text field `root`: WebKit's undo or redo, skipping (cancelling
  // before they apply) the steps that belong to other fields; true when one
  // of this field's own was undone or redone.
  api.history = (root, cmd) => {
    if (cmd === "redo" && !(redoable.get(root) > 0)) return false;
    let mine = false;
    const guard = (e) => {
      if (!isHistory(e.inputType)) return;
      if (root.contains(e.target)) mine = true;
      else e.preventDefault();
    };
    document.addEventListener("beforeinput", guard, true);
    try {
      for (let i = 0; i < 500 && !mine && document.queryCommandEnabled(cmd); i++) document.execCommand(cmd);
    } catch (_) {
    } finally {
      document.removeEventListener("beforeinput", guard, true);
    }
    if (mine) redoable.set(root, (redoable.get(root) || 0) + (cmd === "undo" ? 1 : -1));
    return mine;
  };
  // After the app has handled the key (bubbling, past the app's root).
  document.addEventListener("keydown", (e) => {
    const el = e.target;
    if (e.isComposing || e.altKey || e.metaKey) return;
    if (!isText(el)) {
      // Tab out of the message body while typing there (see `straight`).
      if ((e.key === "Tab" || e.code === "Tab") && !e.shiftKey && !e.ctrlKey && !e.defaultPrevented && el && el.closest) {
        const root = el.closest("[data-ax-tab-keeps-typing]");
        if (root && lastRich && root.contains(lastRich.el) && Date.now() - lastRich.at <= TYPED_BEFORE_TAB_MS) {
          straight = { from: root, enter: true, ts: pressedAt(e) ?? performance.now(), keys: 0, last: 0, range: null };
          keepRange(straight);
        }
        return;
      }
      // Rich-text fields without their own handling (the signature editor).
      const cmd = e.ctrlKey && el && el.isContentEditable ? undoCommand(e) : null;
      if (cmd && !e.defaultPrevented) {
        e.preventDefault();
        const root = el.closest('[contenteditable]:not([contenteditable="false"])') || el;
        api.history(root, cmd);
      }
      return;
    }
    if (e.ctrlKey) {
      const cmd = undoCommand(e);
      if (cmd && !e.defaultPrevented) {
        e.preventDefault();
        undoIn(el, cmd);
      }
      return;
    }
    if (!getValue(el).trim()) return;
    const list = el.getAttribute("data-ax-address-list");
    let clear = false;
    if (e.key === "Enter" && e.defaultPrevented && !e.shiftKey && el.hasAttribute("data-ax-enter-keeps-typing"))
      straight = { from: el, enter: false, edits: true, ts: pressedAt(e) ?? performance.now(), keys: 0, last: 0 };
    if (e.key === "Enter") {
      const clears = el.hasAttribute("data-ax-enter-clears");
      clear = !e.shiftKey && (!!list || (clears && e.defaultPrevented));
      // Not taken (an invalid entry, a failed add): the text stays to fix,
      // selected, so an entry typed straight on replaces it instead of
      // running into it ("mtypo", Enter, "m1@blk.test" blocked
      // "mtypom1@blk.test").
      if (clears && !list && !e.shiftKey && !e.defaultPrevented) el.select();
    }
    else if (e.key === "Escape" || e.key === "Esc") clear = el.hasAttribute("data-ax-escape-clears") && e.defaultPrevented;
    else if (e.key === "Tab" || e.code === "Tab" || e.keyCode === 9) clear = !!list;
    else if (e.key === ";" || e.key === ",") clear = list === "keys" && e.defaultPrevented;
    if (clear) setTyped(el, "");
  });

  // Resolves once no key has arrived for `ms` (or after `max` ms), so keys
  // still queued behind a click or a shortcut land in the form first.
  api.quiet = (ms = 80, max = 600) =>
    new Promise((done) => {
      const start = Date.now();
      const check = () => {
        const now = Date.now();
        if (now - lastKeyAt >= ms || now - start >= max) done();
        else setTimeout(check, 20);
      };
      check();
    });

  // Hand-over while one form replaces another (compose pop-out): `hold()`
  // catches keys still arriving in a field outside the app, and
  // `release(id)` focuses the new form's field and types them there, so
  // they neither vanish with the old form nor land in another field.
  let held = null;
  api.hold = () => {
    if (held) return;
    const t = document.createElement("textarea");
    t.setAttribute("aria-hidden", "true");
    t.tabIndex = -1;
    t.style.cssText = "position:fixed;left:-10000px;top:0;width:1px;height:1px;opacity:0;";
    document.body.appendChild(t);
    t.focus({ preventScroll: true });
    held = t;
    setTimeout(() => {
      if (held === t) {
        held = null;
        t.remove();
      }
    }, 5000);
  };
  api.release = (id) => {
    const t = held;
    held = null;
    const el = document.getElementById(id);
    const text = t ? t.value : "";
    if (t) t.remove();
    if (!el) return;
    el.focus();
    const n = getValue(el).length;
    try {
      el.setSelectionRange(n, n);
    } catch (_) {}
    if (text) document.execCommand("insertText", false, text);
  };
})(globalThis);
