//! Small bridge to the page: the shell installs `dom.js` once, and
//! components call these helpers (from event handlers or effects) to move
//! focus, announce text to screen readers and so on.
//!
//! Markup conventions the runtime understands:
//! * `data-ax-roving="horizontal|vertical|both|grid"` on a composite widget
//!   and `data-ax-item` on its items: arrow keys, Home/End (and
//!   `data-ax-typeahead`, `data-ax-wrap`, `data-ax-cols`) move focus, and the
//!   widget keeps a single tab stop.
//! * `aria-modal="true"`: Tab is trapped in the topmost modal; Enter presses
//!   its `data-ax-default` button unless focus is on a control that handles
//!   Enter itself (buttons, links, text areas, menus, lists...).
//! * `title` / `data-ax-tip` (+ `data-ax-tip-pos="right"`): styled tooltip
//!   after 500 ms; `aria-keyshortcuts` is appended as "(Ctrl+R)".
//! * `data-ax-region`: an F6 stop inside a module (default: each
//!   `.ax-panes > .ax-pane`); `data-ax-default-focus` picks the element F6
//!   lands on the first time.
//! * `data-ax-keytip="N"`: ribbon key tip letters.
//! * `data-ax-surface` with `data-ax-grip` handles (and
//!   `data-ax-surface-move` areas on `data-ax-movable` surfaces): resizable
//!   dialogs and in-app windows, see [`super::surface`].
//!
//! Text fields need nothing: `typed_text.js`, installed first in the
//! runtime, skips the late echo of earlier keys that a controlled `value:`
//! would otherwise write back over newer typing ([`TypedTextSync`], mounted
//! by the shell, settles the doubtful cases). Fields that a key commits and
//! empties say so, and the page empties them as the key goes down:
//! * `data-ax-enter-clears="true"` (To Do's "Add a task"): Enter, when the
//!   handler took it with `prevent_default()` (a failed add doesn't: its
//!   text is selected, so the next entry typed replaces it).
//! * `data-ax-address-list` (recipient and attendee boxes): Enter and Tab;
//!   `"keys"` also `;` and `,` taken with `prevent_default()` and a pasted
//!   list (the app adds it whole), `"split"` also a separator typed at the
//!   end (the app splits typed text).
//!
//! Keys typed straight on go back to a field the app (or Tab) moved focus
//! from: after Enter in `data-ax-enter-keeps-typing` (the search box) and
//! after Tab out of `data-ax-tab-keeps-typing` (the message body). Keys
//! pressed in a text field before a click, but delivered after it, act in
//! that field (chords too: undo, word deletion, paste), never in the one
//! clicked.

use std::sync::atomic::{AtomicU64, Ordering};

use dioxus::prelude::*;

/// The typed-text guard: a text field's value written back from an older
/// render is skipped while the field shows newer typing, so fast typing
/// never loses letters (the render lags the keys over the webview IPC).
/// Part of [`RUNTIME_JS`]; on its own for the tests.
#[cfg(test)]
const TYPED_TEXT_JS: &str = include_str!("typed_text.js");

pub(crate) const RUNTIME_JS: &str = concat!(include_str!("typed_text.js"), "\n", include_str!("dom.js"));

/// Install the runtime. Idempotent; the shell calls it once after mount.
pub fn install() {
    run(RUNTIME_JS);
}

/// The typed-text guard's render barrier. The page sends a number when it
/// skipped a value it could not be sure was a late echo; the next render
/// carries that number on this element, so the page knows the app has
/// rendered everything after the keys it had sent (see `typed_text.js`).
/// Mounted once, by the shell.
#[component]
pub fn TypedTextSync() -> Element {
    let mut tick = use_signal(|| 0u64);
    use_future(move || async move {
        let mut ch = document::eval(SYNC_JS);
        while let Ok(n) = ch.recv::<u64>().await {
            if n > *tick.peek() {
                tick.set(n);
            }
        }
    });
    rsx! {
        span { hidden: true, "aria-hidden": "true", "data-ax-typed-sync": "{tick}" }
    }
}

const SYNC_JS: &str = "const c = () => { if (window.axTypedText && axTypedText.connect) axTypedText.connect((n) => dioxus.send(n)); else setTimeout(c, 20); }; c();";

/// Tags of the late-key fields on the page now (see [`use_late_tag`]), with
/// how many components hold each (an old card going as a new one comes).
static LATE_LIVE: std::sync::Mutex<Vec<(String, usize)>> = std::sync::Mutex::new(Vec::new());

fn late_live_add(tag: &str, by: isize) {
    let Ok(mut live) = LATE_LIVE.lock() else { return };
    match live.iter().position(|(t, _)| t == tag) {
        Some(i) => {
            let n = live[i].1 as isize + by;
            if n <= 0 {
                live.swap_remove(i);
            } else {
                live[i].1 = n as usize;
            }
        }
        None if by > 0 => live.push((tag.to_string(), by as usize)),
        None => {}
    }
}

fn late_is_live(tag: &str) -> bool {
    LATE_LIVE.lock().is_ok_and(|live| live.iter().any(|(t, _)| t == tag))
}

/// Mark the calling component as the editor of the field tagged `tag`
/// (`data-ax-late-keys`), while it is mounted: late keys for that field are
/// then its own input events, and [`use_late_keys`] leaves them alone.
pub fn use_late_tag(tag: String) {
    use_hook(|| late_live_add(&tag, 1));
    // The first render's closure is the one kept, with that render's tag.
    use_drop(move || late_live_add(&tag, -1));
}

/// Keys pressed in a text field before a click that removed it (another
/// task opened in To Do while its title was being typed), delivered by a
/// busy page after the click: the page puts them into the field it still
/// holds and sends the field's value here, when the field carries
/// `data-ax-late-keys="{prefix}:{rest}"` and no component edits that field
/// any more ([`use_late_tag`]). `on_value` gets `(rest, value)`; call this
/// in a component that outlives the fields (the module's root).
pub fn use_late_keys(prefix: &'static str, on_value: Callback<(String, String)>) {
    use_future(move || async move {
        let mut ch = document::eval(&late_js(prefix));
        while let Ok((tag, value)) = ch.recv::<(String, String)>().await {
            if let Some(rest) = late_rest(prefix, &tag)
                && !late_is_live(&tag)
            {
                on_value.call((rest.to_string(), value));
            }
        }
    });
}

fn late_js(prefix: &str) -> String {
    format!(
        "const c = () => {{ if (window.axTypedText && axTypedText.lateTo) axTypedText.lateTo({}, (m) => dioxus.send(m)); else setTimeout(c, 20); }}; c();",
        js_str(prefix)
    )
}

/// The part of `tag` after `prefix:`.
fn late_rest<'a>(prefix: &str, tag: &'a str) -> Option<&'a str> {
    tag.strip_prefix(prefix)?.strip_prefix(':')
}

/// Run a script without waiting for its result.
pub fn run(js: &str) {
    let _ = document::eval(js);
}

/// Wait until keys still on their way have landed (a busy page delivers a
/// click before keys typed just ahead of it), then say whether the element
/// matching `selector` has focus. Leaving an address box adds what was
/// typed only after this, so its last letters are part of the address.
pub async fn keys_landed(selector: &str) -> bool {
    let js = format!(
        "if (window.axTypedText && axTypedText.quiet) await axTypedText.quiet(); const e = document.querySelector({}); return !!e && document.activeElement === e;",
        js_str(selector)
    );
    document::eval(&js).join::<bool>().await.unwrap_or(false)
}

/// JSON-quote a string for embedding in a script.
pub fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn call(f: &str, args: &str) {
    run(&format!("window.axShell && axShell.{f}({args})"));
}

/// Move focus to the first element matching `selector` (keyboard-visible ring).
pub fn focus(selector: &str) {
    call("focusSel", &js_str(selector));
}

/// Focus the first focusable descendant (or `item` match) of `selector`.
pub fn focus_first_in(selector: &str, item: &str) {
    call("focusFirstIn", &format!("{}, {}", js_str(selector), js_str(item)));
}

/// Remember the focused element and focus the first item of popup `id`.
/// Focus returns to the invoker when the popup leaves the page.
pub fn open_popup(id: &str) {
    call("openPopup", &js_str(id));
}

/// Like [`open_popup`] for dialogs: focuses `[data-ax-autofocus]` or the
/// first tabbable element of the body.
pub fn open_dialog(id: &str) {
    open_dialog_with(id, None);
}

/// [`open_dialog`] with an explicit first focus: `initial` is a selector
/// inside the dialog (falls back to the usual choice when nothing matches).
pub fn open_dialog_with(id: &str, initial: Option<&str>) {
    call("openDialog", &format!("{}, {}", js_str(id), initial.map(js_str).unwrap_or_else(|| "null".into())));
}

/// The dialog `id` closed: give focus back to the element that opened it
/// when focus was lost with the dialog.
pub fn close_dialog(id: &str) {
    call("closeDialog", &js_str(id));
}

/// Remember the focused element so it gets focus back later.
pub fn push_focus() {
    call("push", "");
}

/// Keep a popup inside the viewport.
pub fn fit(id: &str) {
    call("fit", &js_str(id));
}

/// Next / previous F6 region.
pub fn cycle_regions(dir: i32) {
    call("cycle", &dir.to_string());
}

/// Focus the F6 region matching `selector` (its remembered or default element).
pub fn enter_region(selector: &str) {
    call("enterRegion", &js_str(selector));
}

/// Speak `text` through the polite (or assertive) live region.
pub fn announce(text: &str, assertive: bool) {
    call("announce", &format!("{}, {assertive}", js_str(text)));
}

/// Scroll to the element and briefly highlight it.
pub fn flash(selector: &str) {
    call("flash", &js_str(selector));
}

/// Chords whose browser default must never run (`always`) or only outside
/// text fields (`outside`).
pub fn set_prevent(always: &[&str], outside: &[&str]) {
    let a = serde_json::to_string(always).unwrap_or_default();
    let o = serde_json::to_string(outside).unwrap_or_default();
    call("setPrevent", &format!("{a}, {o}"));
}

pub fn set_keytips(on: bool) {
    call("setKeytips", if on { "true" } else { "false" });
}

/// A per-instance DOM id such as `ax-menu-12`, stable for the component's life.
pub fn use_unique_id(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let prefix = prefix.to_string();
    use_hook(move || format!("{prefix}-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_strings_are_quoted() {
        assert_eq!(js_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(js_str("#x .y"), "\"#x .y\"");
    }

    /// The guard's decisions, run in gjs (GNOME's JavaScript shell) when it
    /// is installed; the page itself is checked in WebKitGTK.
    #[test]
    fn typed_text_skips_only_late_echoes() {
        assert!(RUNTIME_JS.starts_with(TYPED_TEXT_JS), "installed with the runtime");
        for attr in ["data-ax-enter-clears", "data-ax-escape-clears", "data-ax-address-list", "data-ax-typed-sync", "data-ax-late-keys"] {
            assert!(TYPED_TEXT_JS.contains(attr), "{attr}");
        }
        // What compose's actions call (actions.rs, state.rs) and the barrier connects to.
        for f in ["api.quiet =", "api.hold =", "api.release =", "api.connect =", "api.history =", "api.lateTo ="] {
            assert!(TYPED_TEXT_JS.contains(f), "{f}");
        }
        assert!(SYNC_JS.contains("axTypedText.connect"));
        assert!(late_js("todo").contains(r#"axTypedText.lateTo("todo","#));
        // The To Do detail fields carry their tags, and their cards hold them.
        let detail = include_str!("../tasks/detail.rs");
        assert_eq!(detail.matches(r#""data-ax-late-keys": LateField::"#).count(), 3, "title, step and note");
        assert_eq!(detail.matches("use_late_tag(LateField::").count(), 3);
        // The address boxes keep their markers (Enter/Tab/separators empty them in the page).
        assert!(include_str!("../mail/compose/recipients.rs").contains(r#""data-ax-address-list": "split""#));
        assert!(include_str!("../calendar/fields.rs").contains(r#""data-ax-address-list": "keys""#));
        assert!(include_str!("../shell/search.rs").contains(r#""data-ax-escape-clears": "true""#));
        assert!(include_str!("../shell/search.rs").contains(r#""data-ax-enter-keeps-typing": "true""#));
        assert!(TYPED_TEXT_JS.contains("data-ax-enter-keeps-typing"));
        assert!(include_str!("../mail/compose/editor.rs").contains(r#""data-ax-tab-keeps-typing": "true""#));
        assert!(TYPED_TEXT_JS.contains("[data-ax-tab-keeps-typing]"));
        let gjs = std::path::Path::new("/usr/bin/gjs");
        if !gjs.exists() {
            eprintln!("gjs not installed: typed-text logic not run");
            return;
        }
        let script = format!(
            r#"{TYPED_TEXT_JS}
const t = globalThis.axTypedText;
const eq = (name, got, want) => {{ if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(name + ": " + JSON.stringify(got)); }};
const st = (base, entries) => ({{ base, log: entries.map(([v, e]) => ({{ v, e }})), app: undefined }});
const vals = (s) => s.log.map((x) => x.v);
let s;
// Echoes behind the keys, in order, while the batch may not have seen them: skipped.
s = st("", [["a", 0], ["ab", 0], ["abc", 0]]);
eq("first echo", [t.appWrite(s, "a", "abc", 0, true), vals(s)], [true, ["ab", "abc"]]);
eq("second echo", [t.appWrite(s, "ab", "abc", 0, true), vals(s)], [true, ["abc"]]);
eq("caught up", [t.appWrite(s, "abc", "abc", 0, true), vals(s)], [false, []]);
// Renders coalesce: one echo stands for every key up to it.
s = st("", [["a", 0], ["ab", 0], ["abc", 0], ["abcd", 0]]);
eq("coalesced", [t.appWrite(s, "abc", "abcd", 0, true), vals(s)], [true, ["abcd"]]);
// The value from before the first key (an autosave render made before it arrived).
s = st("", [["g", 5]]);
eq("value before typing", t.appWrite(s, "", "g", 5, true), true);
// Keys an earlier batch was applied before have been seen: the newest of them is the floor.
s = st("", [["a", 3], ["ab", 3], ["abc", 4]]);
eq("floor echo", [t.appWrite(s, "ab", "abc", 4, true), s.base, vals(s)], [true, "ab", ["abc"]]);
s = st("", [["a", 3], ["ab", 3], ["abc", 4]]);
eq("older than the floor is the app's own", [t.appWrite(s, "a", "abc", 4, true), vals(s)], [false, []]);
// The user emptied the field earlier, typed "pri" and picked a suggestion: the app's clear goes through.
s = st("", [["x", 0], ["", 1], ["p", 2], ["pr", 3], ["pri", 4]]);
eq("clear after an earlier empty", [t.appWrite(s, "", "pri", 7, true), vals(s)], [false, []]);
// Every key seen: whatever the app writes is its own.
s = st("", [["bob@x.example;", 2]]);
eq("all seen", [t.appWrite(s, "", "bob@x.example;", 3, true), vals(s)], [false, []]);
// A value the user never typed (a chip made, a value set elsewhere) goes through and ends the run.
s = st("", [["b", 0], ["bo", 0], ["bob;", 0]]);
eq("outside write", [t.appWrite(s, "Bob", "bob;", 0, true), vals(s)], [false, []]);
// The field no longer shows the newest typed text: something else set it, so writes go through.
s = st("", [["a", 0], ["ab", 0]]);
eq("changed elsewhere", [t.appWrite(s, "a", "zz", 0, true), vals(s)], [false, []]);
// Deleting and retyping repeats values: they are taken oldest first.
s = st("", [["t", 0], ["te", 0], ["teh", 0], ["te", 0], ["t", 0], ["th", 0], ["the", 0]]);
eq("repeat 1", [t.appWrite(s, "te", "the", 0, true), vals(s).length], [true, 5]);
eq("repeat 2", [t.appWrite(s, "t", "the", 0, true), vals(s)], [true, ["th", "the"]]);
eq("repeat 3", [t.appWrite(s, "the", "the", 0, true), vals(s)], [false, []]);
eq("empty log", t.appWrite(st("", []), "x", "y", 0, true), false);
// Doubtful: the app set the value from before the keys. Skipped, then the barrier settles it.
s = st("", [["d", 0], ["dan@x.example; eve@y.example", 0]]);
eq("doubtful skip", t.appWrite(s, "", "dan@x.example; eve@y.example", 0, true), true);
eq("settled to the app's value", t.settle(s, "dan@x.example; eve@y.example"), "");
s = st("", [["a", 0], ["ab", 0]]);
t.appWrite(s, "a", "ab", 0, true);
t.appWrite(s, "ab", "ab", 0, true);
eq("settled, agreeing", t.settle(s, "ab"), null);
s = st("", [["a", 0], ["ab", 0]]);
t.appWrite(s, "", "ab", 0, true);
eq("settle leaves a field changed since", t.settle(s, "zz"), null);
// Liveview: batches prove nothing, so earlier values stay echoes until the barrier.
s = st("", [["x", 0], ["", 1], ["p", 2], ["pri", 4]]);
eq("liveview", t.appWrite(s, "", "pri", 7, false), true);
eq("liveview settled", t.settle(s, "pri"), "");
// Address boxes that split typed text.
eq("ended", ["bob@x.example;", "a@x.example, b@y.example,", "Priya Raman <p@x.example>;"].map(t.entryEnded), [true, true, true]);
eq("not ended", ["\"Bowen,", "Bowen <b;", ";", "a@x.example; ", "Team:", "bob"].map(t.entryEnded), [false, false, false, false, false, false]);
eq("ended entries", ["a@x.example; b@y.example", "a@x.example;", "\"Lee, Eve\" <e@x.example>", "a; b; c", ""].map((x) => t.endedEntries(x)[0]), [1, 1, 0, 2, 0]);
// "keys" boxes: a paste holding an ended entry is added whole; typing (one character more) is not a paste.
eq("pasted list", t.pasteEnds("", "x1@example.org; x2@example.org"), true);
eq("pasted one", t.pasteEnds("", "x1@example.org"), false);
eq("typed separator", t.pasteEnds("a@x.example", "a@x.example;"), false);
eq("pasted after text", t.pasteEnds("dan", "dan@x.example, eve@y.example"), true);
// Keys after Enter in the search box: straight on, by press time.
const after = (enter) => ({{ ts: 1000, keys: 0, last: 0, enter }});
s = after(false);
eq("first key in time", t.straightOn(s, " ", 1400, false), "type");
eq("stream", [t.straightOn(s, "v", 1500, false), t.straightOn(s, "a", 1700, false)], ["type", "type"]);
eq("pause ends it", t.straightOn(s, "r", 2000, false), "end");
s = after(false);
eq("too late after Enter", t.straightOn(s, " ", 1700, false), "end");
// Down 2 s after Enter, then Space: navigation ends it (Space selects the row).
s = after(false);
eq("arrow ends it", t.straightOn(s, "ArrowDown", 3000, false), "end");
s = after(false);
eq("even right after Enter", t.straightOn(s, "ArrowDown", 1100, false), "end");
s = after(false);
eq("Shift waits", [t.straightOn(s, "Shift", 1100, false), t.straightOn(s, "?", 1200, false)], ["wait", "type"]);
s = after(false);
eq("chord ends it", t.straightOn(s, "a", 1100, true), "end");
eq("Enter again is not typing in search", t.straightOn(after(false), "Enter", 1100, false), "end");
// Backspace right after Enter in the search box fixes the query (the results would archive with it).
const search = () => ({{ ts: 1000, keys: 0, last: 0, enter: false, edits: true }});
s = search();
eq("Backspace straight on", [t.straightOn(s, "Backspace", 1100, false), t.straightOn(s, "Backspace", 1250, false), t.straightOn(s, "e", 1400, false)], ["type", "type", "type"]);
eq("Backspace after a pause goes to the results", t.straightOn(search(), "Backspace", 1700, false), "end");
eq("Delete straight on stays in the box", t.straightOn(search(), "Delete", 1100, false), "type");
eq("Delete after a pause goes to the results", t.straightOn(search(), "Delete", 1700, false), "end");
eq("Backspace is not typing in the body", t.straightOn(after(true), "Backspace", 1100, false), "end");
eq("nor is Delete", t.straightOn(after(true), "Delete", 1100, false), "end");
// Tab out of the message body: Space and Enter typed straight on stay in the body.
s = after(true);
eq("body", [t.straightOn(s, " ", 1150, false), t.straightOn(s, "Enter", 1300, false)], ["type", "type"]);
eq("Tab to Send then a pause: Space presses Send", t.straightOn(after(true), " ", 1800, false), "end");
eq("Tab again moves on", t.straightOn(after(true), "Tab", 1100, false), "end");
eq("clock went back", t.straightOn(after(true), " ", 900, false), "end");
// The focus timeline: a key goes to the focus it was pressed in, by press time.
const tl = (...untils) => untils.map((until, i) => ({{ until, el: "f" + i }}));
let line = tl(40742);
eq("before the click", [t.pressedIn(line, 40585).el, t.pressedIn(line, 40730).el], ["f0", "f0"]);
eq("after the click", [t.pressedIn(line, 40742), line.length], [null, 0]);
// No time limit: a title typed for 6 s on a page far behind still owns its keys.
line = tl(16750);
eq("long before", t.pressedIn(line, 1000).el, "f0");
// Two clicks before the keys came: title keys to the title, note keys to the note.
line = tl(44817, 45745);
eq("two clicks", [t.pressedIn(line, 42031).el, t.pressedIn(line, 42500).el, t.pressedIn(line, 45000).el, line.length], ["f0", "f0", "f1", 1]);
eq("after both", [t.pressedIn(line, 45746), line.length], [null, 0]);
line = tl(100, 200);
eq("no press time forgets the clicks", [t.pressedIn(line, null), line.length], [null, 0]);
// What a late key does in the field it was pressed in.
const lk = (k, ctrl = false, alt = false, shift = false) => t.lateKind(k, ctrl, alt, shift);
eq("plain keys", ["a", " ", "Backspace", "Delete", "Enter", "Tab", "ArrowDown", "Escape"].map((k) => lk(k)), ["type", "type", "type", "type", "drop", "drop", "drop", "drop"]);
eq("chords", [lk("z", true), lk("Z", true, false, true), lk("y", true), lk("Backspace", true), lk("Delete", true), lk("v", true), lk("V", true, false, true)], ["undo", "redo", "redo", "word", "word", "paste", "paste"]);
eq("text chords dropped", [lk("x", true), lk("a", true), lk("ArrowLeft", true), lk("End", true)], ["drop", "drop", "drop", "drop"]);
eq("app shortcuts", [lk("s", true), lk("1", true), lk("Enter", true), lk("s", false, true), lk("Y", true, false, true)], ["through", "through", "through", "through", "through"]);
// Press times on the page's clock: until one event shows it (handled within 2 s), only 10 s old.
eq("proves the clock", [t.provesClock(1000, 1500), t.provesClock(1000, 4000), t.provesClock(0, 100), t.provesClock(5000, 1000)], [true, false, false, false]);
eq("clock not known yet", [t.onPageClock(1000, 9000, false), t.onPageClock(1000, 12000, false)], [true, false]);
// A page 11 s behind (a loaded machine) still knows which keys came before a click.
eq("clock known", [t.onPageClock(13510, 23713, true), t.onPageClock(1000, 200000, true), t.onPageClock(30000, 1000, true)], [true, false, false]);
eq("after a 10 s lag, still before the click", t.pressedIn(tl(14811), 13510).el, "f0");
// Late keys go into the field as the user left it at the click (the value sent to the app).
const ed = (v, s, e, k, max = -1) => {{ const n = t.editAt(v, s, e, k, max); return n && [n.v, n.at]; }};
eq("typed at the caret", [ed("plan ra100 r", 12, 12, "e"), ed("ab", 1, 1, "x"), ed("ab", null, null, "c")], [["plan ra100 re", 13], ["axb", 2], ["abc", 3]]);
eq("deletes", [ed("abc", 3, 3, "Backspace"), ed("abc", 0, 0, "Backspace"), ed("abc", 1, 1, "Delete"), ed("abc", 3, 3, "Delete"), ed("abcd", 1, 3, "Backspace")], [["ab", 2], null, ["ac", 1], null, ["ad", 1]]);
eq("a selection is replaced", ed("abcd", 1, 3, "x"), ["axd", 2]);
eq("surrogate pairs", [ed("a\u{{1F600}}", 3, 3, "Backspace"), ed("\u{{1F600}}b", 0, 0, "Delete")], [["a", 1], ["b", 0]]);
eq("full field", [ed("abc", 3, 3, "d", 3), ed("abc", 3, 3, "Backspace", 3)], [null, ["ab", 2]]);
eq("pasted text", ed("Corporate code", 9, 9, " A1"), ["Corporate A1 code", 12]);
// Ctrl+Backspace / Ctrl+Delete: a word and the spaces next to it.
const wd = (v, s, k) => {{ const n = t.editAt(v, s, s, k, -1, true); return n && [n.v, n.at]; }};
eq("word back", [wd("budget wrongword", 16, "Backspace"), wd("budget wrongword ", 17, "Backspace"), wd("one", 3, "Backspace"), wd("x", 0, "Backspace")], [["budget ", 7], ["budget ", 7], ["", 0], null]);
eq("word forward", [wd("code: AXN-24", 0, "Delete"), wd("a  bc d", 1, "Delete"), wd("ab", 2, "Delete")], [[" AXN-24", 0], ["a d", 1], null]);
// Undo history, one per field. Typing "Budget" in a run is one step.
const type = (h, from, text, t0, same = true) => {{
  let v = from;
  for (const [i, c] of [...text].entries()) {{
    h = t.willEdit(h, v, v.length, v.length, "type", t0 + i * 80, same || i > 0);
    v += c;
    h.post = v;
  }}
  return [h, v];
}};
let h, v;
[h, v] = type(undefined, "", "Budget", 1000, false);
eq("one step", h.undo.length, 1);
let back = t.historyStep(h, "undo", v, 6, 6);
eq("undone", [back.v, back.s], ["", 0]);
eq("nothing more to undo", t.historyStep(h, "undo", "", 0, 0), null);
eq("redone", t.historyStep(h, "redo", "", 0, 0).v, "Budget");
// A pause, a delete, or an edit in another field in between starts a new step.
[h, v] = type(h, "Budget", " Q4", 5000);
h = t.willEdit(h, v, v.length, v.length, "delete", 5300, true); v = "Budget Q"; h.post = v;
eq("steps", h.undo.map((x) => x.v), ["", "Budget", "Budget Q4"]);
[h, v] = type(h, v, "3", 5400, false);
eq("after another field", h.undo.length, 4);
eq("new edit ends redo", h.redo.length, 0);
// Changed by something else since (a chip made, the app's value): nothing to undo, never another field's.
eq("changed elsewhere", t.historyStep(h, "undo", "", 0, 0), null);
h = t.willEdit(h, "", 0, 0, "type", 9000, true);
eq("starts over", h.undo.map((x) => x.v), [""]);
eq("no history", t.historyStep(undefined, "undo", "x", 1, 1), null);
eq("kinds", ["insertText", "insertCompositionText", "deleteContentBackward", "deleteByCut", "insertFromPaste", "insertLineBreak"].map(t.editKind), ["type", "type", "delete", "other", "other", "other"]);
print("ok");
"#
        );
        let out = std::process::Command::new(gjs).arg("-c").arg(&script).output().expect("run gjs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success() && stdout.trim() == "ok", "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    }

    /// Late keys for a field are left to the component editing it while it
    /// is mounted (two can hold a tag for a moment: the old card going as
    /// the new one comes).
    #[test]
    fn late_tags_count_their_holders() {
        let tag = "test:title:987654321";
        assert!(!late_is_live(tag));
        late_live_add(tag, 1);
        late_live_add(tag, 1);
        late_live_add(tag, -1);
        assert!(late_is_live(tag));
        late_live_add(tag, -1);
        assert!(!late_is_live(tag));
        late_live_add(tag, -1);
        assert!(!late_is_live(tag), "never below zero");
        assert_eq!(late_rest("todo", "todo:title:5"), Some("title:5"));
        assert_eq!(late_rest("todo", "todos:title:5"), None);
        assert_eq!(late_rest("todo", "todo"), None);
    }

    #[test]
    fn runtime_defines_the_api_the_helpers_call() {
        for f in ["focusSel", "focusFirstIn", "openPopup", "openDialog", "push", "fit", "cycle", "announce", "flash", "setPrevent", "setKeytips", "watchSurface"] {
            assert!(RUNTIME_JS.contains(&format!("ax.{f} =")), "dom.js lacks ax.{f}");
        }
    }
}
