//! The keymap: one table (docs/spec/keyboard.md §3–§9) used by the global
//! key handler, the keyboard-shortcuts dialog, tooltips and the webview's
//! default-action filter.
//!
//! The shell runs only the rows with a [`GlobalCmd`]; module rows document
//! what each module implements in its own container handler.

use axentra_core::KeyboardMode;

/// Dialog sections, in default order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    General,
    Mail,
    Reading,
    Compose,
    Formatting,
    Calendar,
    People,
    Tasks,
    Search,
}

impl Section {
    pub const ALL: [Section; 9] = [
        Section::General,
        Section::Mail,
        Section::Reading,
        Section::Compose,
        Section::Formatting,
        Section::Calendar,
        Section::People,
        Section::Tasks,
        Section::Search,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Mail => "Mail",
            Section::Reading => "Reading",
            Section::Compose => "Compose",
            Section::Formatting => "Formatting",
            Section::Calendar => "Calendar",
            Section::People => "People",
            Section::Tasks => "To Do",
            Section::Search => "Search",
        }
    }
}

/// Commands the shell itself runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobalCmd {
    GoMail,
    GoCalendar,
    GoPeople,
    GoTasks,
    NextRegion,
    PrevRegion,
    Shortcuts,
    Search,
    Undo,
    NewMessage,
    Sync,
    Settings,
    Help,
}

impl GlobalCmd {
    /// Navigation keys stay on with "Turn off keyboard shortcuts" (§1.2).
    fn always_on(self) -> bool {
        matches!(self, GlobalCmd::NextRegion | GlobalCmd::PrevRegion)
    }
}

pub struct Shortcut {
    pub section: Section,
    pub action: &'static str,
    /// Keys in the default (Outlook) set, display form: "Ctrl+R",
    /// "Ctrl+> or Ctrl+.", "G then I", "Alt, H".
    pub keys: &'static str,
    /// Keys in the web-style sets when they differ ("" = same).
    pub web: &'static str,
    pub cmd: Option<GlobalCmd>,
    /// Accepted but not listed (e.g. Outlook on the web's module keys).
    pub aliases: &'static [&'static str],
}

const fn row(section: Section, action: &'static str, keys: &'static str, web: &'static str) -> Shortcut {
    Shortcut { section, action, keys, web, cmd: None, aliases: &[] }
}

const fn cmd(section: Section, action: &'static str, keys: &'static str, web: &'static str, c: GlobalCmd, aliases: &'static [&'static str]) -> Shortcut {
    Shortcut { section, action, keys, web, cmd: Some(c), aliases }
}

use GlobalCmd as G;
use Section::*;

/// Module keys follow docs/spec/shell.md §4.1/§10: Ctrl+1 Mail, Ctrl+2
/// Calendar, Ctrl+3 reserved, Ctrl+4 People, Ctrl+5 To Do; the web's
/// Ctrl+Shift+digit keys are silent aliases.
pub const SHORTCUTS: &[Shortcut] = &[
    // General
    cmd(General, "Go to Mail", "Ctrl+1", "Ctrl+Shift+1", G::GoMail, &["Ctrl+Shift+1"]),
    cmd(General, "Go to Calendar", "Ctrl+2", "Ctrl+Shift+2", G::GoCalendar, &["Ctrl+Shift+2"]),
    cmd(General, "Go to People", "Ctrl+4", "Ctrl+Shift+4", G::GoPeople, &["Ctrl+Shift+4"]),
    cmd(General, "Go to To Do", "Ctrl+5", "", G::GoTasks, &[]),
    cmd(General, "Move to the next region", "F6 or Ctrl+F6", "", G::NextRegion, &[]),
    cmd(General, "Move to the previous region", "Shift+F6 or Ctrl+Shift+F6", "", G::PrevRegion, &["Ctrl+Shift+Tab"]),
    cmd(General, "Show keyboard shortcuts", "?", "Shift+?", G::Shortcuts, &[]),
    cmd(General, "Go to Search", "Ctrl+E or Alt+Q or F3", "Alt+Q or /", G::Search, &[]),
    cmd(General, "Undo the last action", "Ctrl+Z", "", G::Undo, &[]),
    row(General, "New item in the current module", "Ctrl+N", "N"),
    cmd(General, "New message from anywhere", "Ctrl+Shift+M", "", G::NewMessage, &[]),
    cmd(General, "Sync now", "F9", "", G::Sync, &[]),
    cmd(General, "Open Settings", "Ctrl+,", "", G::Settings, &[]),
    cmd(General, "Open Help", "F1", "", G::Help, &[]),
    row(General, "Show key tips on the ribbon", "Alt", "Alt+."),
    row(General, "Go to the Home tab / View tab", "Alt, H / Alt, V", "Alt+H / Alt+V"),
    row(General, "Open a split button's menu", "Alt+Down", ""),
    row(General, "Open the context menu", "Shift+F10 or Menu", ""),
    row(General, "Print", "Ctrl+P", ""),
    row(General, "Close a menu, dialog or pane", "Esc", ""),
    // Mail
    row(Mail, "Go to the folder pane", "Ctrl+Y", ""),
    row(Mail, "Delete", "Delete", ""),
    row(Mail, "Permanently delete", "Shift+Delete", ""),
    row(Mail, "Mark as read", "Ctrl+Q", "Ctrl+Q or Q"),
    row(Mail, "Mark as unread", "Ctrl+U", "Ctrl+U or U"),
    row(Mail, "Flag or mark complete", "Insert", ""),
    row(Mail, "Archive", "Backspace", "E"),
    row(Mail, "Move to folder", "Ctrl+Shift+V", "V"),
    row(Mail, "Copy to folder", "Ctrl+Shift+Y", ""),
    row(Mail, "Reply", "Ctrl+R", "Ctrl+R or R"),
    row(Mail, "Reply all", "Ctrl+Shift+R", "Ctrl+Shift+R or Shift+R"),
    row(Mail, "Forward", "Ctrl+F", "Ctrl+Shift+F or Shift+F"),
    row(Mail, "Forward as attachment", "Ctrl+Alt+F", ""),
    row(Mail, "Open in a new window", "Shift+Enter", ""),
    row(Mail, "Next message", "Ctrl+> or Ctrl+.", "Ctrl+."),
    row(Mail, "Previous message", "Ctrl+<", "Ctrl+,"),
    row(Mail, "Go to Inbox", "Ctrl+Shift+I", "G then I"),
    row(Mail, "Select all", "Ctrl+A", ""),
    row(Mail, "Extend the selection", "Shift+Up or Shift+Down", ""),
    row(Mail, "Toggle selection", "Ctrl+Space", ""),
    row(Mail, "Expand or collapse a conversation", "Right / Left", ""),
    row(Mail, "Clear the selection", "Esc", ""),
    // Reading
    row(Reading, "Top or bottom of the message", "Home / End", "Ctrl+Home / Ctrl+End"),
    row(Reading, "Page up or down", "PageUp / PageDown", ""),
    row(Reading, "Move between sections", "Tab / Shift+Tab", ""),
    row(Reading, "Close the message", "Esc", ""),
    // Compose
    row(Compose, "New message", "Ctrl+N", "N"),
    row(Compose, "Send", "Ctrl+Enter or Alt+S", ""),
    row(Compose, "Save draft", "Ctrl+S", ""),
    row(Compose, "Close (the draft is kept)", "Esc", ""),
    row(Compose, "Insert a link", "Ctrl+K", ""),
    row(Compose, "Check names (in To, Cc, Bcc)", "Ctrl+K", ""),
    row(Compose, "Attach a file", "Alt, N, A, F", ""),
    // Formatting
    row(Formatting, "Bold / Italic / Underline", "Ctrl+B / Ctrl+I / Ctrl+U", ""),
    row(Formatting, "Increase font size", "Ctrl+Shift+> or Ctrl+]", ""),
    row(Formatting, "Decrease font size", "Ctrl+Shift+< or Ctrl+[", ""),
    row(Formatting, "Center / left / right align", "Ctrl+E / Ctrl+L / Ctrl+R", ""),
    row(Formatting, "Justify", "Ctrl+Shift+J", ""),
    row(Formatting, "Bulleted list", "Ctrl+Shift+L", ""),
    row(Formatting, "Increase / decrease indent", "Ctrl+T / Ctrl+Shift+T", ""),
    row(Formatting, "Clear formatting", "Ctrl+Space", ""),
    row(Formatting, "Paste as plain text", "Ctrl+Shift+V", ""),
    row(Formatting, "Undo / Redo", "Ctrl+Z / Ctrl+Y", ""),
    // Calendar
    row(Calendar, "New event", "Ctrl+N", "N"),
    row(Calendar, "Open the selected event", "Enter", ""),
    row(Calendar, "Delete the selected event", "Delete", ""),
    row(Calendar, "Day view", "Ctrl+Alt+1", "Shift+Alt+1"),
    row(Calendar, "Work week view", "Ctrl+Alt+2", "Shift+Alt+2"),
    row(Calendar, "Week view", "Ctrl+Alt+3 or Alt+-", "Shift+Alt+3"),
    row(Calendar, "Month view", "Ctrl+Alt+4 or Alt+=", "Shift+Alt+4"),
    row(Calendar, "Three-day view", "Alt+3", ""),
    row(Calendar, "Go to today", "Alt+Shift+Y or Home", "Alt+Shift+Y"),
    row(Calendar, "Next / previous period", "Ctrl+Alt+Right / Ctrl+Alt+Left", "Shift+Right / Shift+Left"),
    row(Calendar, "Next / previous week", "Alt+Down / Alt+Up", ""),
    row(Calendar, "Next / previous month", "Alt+PageDown / Alt+PageUp", ""),
    row(Calendar, "Go to a date", "Ctrl+G", ""),
    // People
    row(People, "New contact", "Ctrl+N", "Shift+N"),
    row(People, "New contact list", "Ctrl+L", "Shift+L"),
    row(People, "Edit the contact or list", "Shift+E", ""),
    row(People, "Save the contact or list", "Ctrl+S", ""),
    row(People, "Delete the contact or list", "Delete or Ctrl+D", "Delete"),
    row(People, "Search contacts", "Ctrl+E", "Alt+Q"),
    row(People, "Select all", "Ctrl+A", ""),
    row(People, "Discard edits or clear the selection", "Esc", ""),
    // To Do
    row(Tasks, "New task", "Ctrl+N", "N"),
    row(Tasks, "New list", "Ctrl+L", ""),
    row(Tasks, "Add to My Day", "Ctrl+T", ""),
    row(Tasks, "Mark complete", "Ctrl+D", ""),
    row(Tasks, "Search tasks", "Ctrl+E or Ctrl+F", ""),
    row(Tasks, "Open task details", "Enter", ""),
    row(Tasks, "Delete the task", "Delete", ""),
    // Search
    row(Search, "Browse suggestions", "Down / Up", ""),
    row(Search, "Run the search", "Enter", ""),
    row(Search, "Close suggestions, then exit search", "Esc, Esc", ""),
    row(Search, "Search the current folder", "Ctrl+Alt+K", ""),
    row(Search, "Search all mail", "Ctrl+Alt+A", ""),
    row(Search, "Include subfolders", "Ctrl+Alt+Z", ""),
];

/// Gmail and Yahoo Mail overlays (keyboard spec §11.2, §11.3), shown in the
/// dialog when that set is active: (key, action).
pub const GMAIL_KEYS: &[(&str, &str)] = &[
    ("C", "New message"),
    ("/", "Search"),
    ("K / J", "Previous / next conversation"),
    ("O or Enter", "Open"),
    ("U", "Back to the list"),
    ("E", "Archive"),
    ("#", "Delete"),
    ("R / A / F", "Reply / Reply all / Forward"),
    ("Shift+I / Shift+U", "Mark read / unread"),
    ("S", "Flag or unflag"),
    ("!", "Report junk"),
    ("V", "Move to folder"),
    ("L", "Categorize"),
    ("Z", "Undo"),
    ("X", "Select conversation"),
    ("G then I", "Go to Inbox"),
    ("?", "Keyboard shortcuts"),
];

pub const YAHOO_KEYS: &[(&str, &str)] = &[
    ("N", "New message"),
    ("S or /", "Search"),
    ("M", "Go to Inbox"),
    (";", "Settings"),
    ("K / Shift+K", "Mark read / unread"),
    ("L / Shift+L", "Flag / unflag"),
    ("D", "Move to folder"),
    ("E", "Archive"),
    ("Delete", "Delete"),
    ("R / A / F", "Reply / Reply all / Forward"),
    ("P", "Print"),
    ("Shift+?", "Keyboard shortcuts"),
];

/// Label of a shortcut set in Settings and the dialog.
pub fn mode_label(mode: KeyboardMode) -> &'static str {
    match mode {
        KeyboardMode::Outlook => "Outlook",
        KeyboardMode::Classic => "Classic Outlook",
        KeyboardMode::Gmail => "Gmail",
        KeyboardMode::Yahoo => "Yahoo Mail",
        KeyboardMode::Off => "Keyboard shortcuts turned off",
    }
}

fn web_style(mode: KeyboardMode) -> bool {
    matches!(mode, KeyboardMode::Gmail | KeyboardMode::Yahoo)
}

/// The keys a row shows for `mode`.
pub fn display_keys(s: &Shortcut, mode: KeyboardMode) -> &'static str {
    if web_style(mode) && !s.web.is_empty() { s.web } else { s.keys }
}

/// "Shift+?" and "?" are the same key press on most layouts; drop Shift
/// before a symbol so both forms compare equal.
pub fn normalize(chord: &str) -> String {
    let (mods, key) = match chord.rfind('+') {
        Some(i) if i + 1 < chord.len() => chord.split_at(i + 1),
        _ if chord.ends_with("++") => chord.split_at(chord.len() - 1),
        _ => ("", chord),
    };
    let symbol = key.chars().count() == 1 && !key.chars().all(|c| c.is_alphanumeric());
    let mods = if symbol { mods.replace("Shift+", "") } else { mods.to_string() };
    let key = match key {
        "Up" => "ArrowUp",
        "Down" => "ArrowDown",
        "Left" => "ArrowLeft",
        "Right" => "ArrowRight",
        "Esc" => "Escape",
        k => k,
    };
    format!("{mods}{key}")
}

/// Single chords of a display string ("Ctrl+E or Alt+Q" → both; sequences
/// such as "G then I" are skipped).
fn chords(display: &'static str) -> impl Iterator<Item = &'static str> {
    display.split(" or ").map(str::trim).filter(|c| !c.is_empty() && !c.contains(" then ") && !c.contains(", ") && !c.contains(" / "))
}

/// The global command for a key press, if the shell handles it in `mode`.
pub fn command_for(chord: &str, mode: KeyboardMode) -> Option<GlobalCmd> {
    let pressed = normalize(chord);
    SHORTCUTS.iter().find_map(|s| {
        let c = s.cmd?;
        if mode == KeyboardMode::Off && !c.always_on() {
            return None;
        }
        let web = if web_style(mode) { chords(s.web) } else { chords("") };
        let hit = chords(s.keys).chain(web).chain(s.aliases.iter().copied()).any(|k| normalize(k) == pressed);
        hit.then_some(c)
    })
}

/// The `aria-keyshortcuts` value for a command in `mode` ("" if unbound).
pub fn aria_for(c: GlobalCmd, mode: KeyboardMode) -> String {
    if mode == KeyboardMode::Off && !c.always_on() {
        return String::new();
    }
    SHORTCUTS
        .iter()
        .find(|s| s.cmd == Some(c))
        .map(|s| crate::kit::keys_to_aria(display_keys(s, mode)))
        .unwrap_or_default()
}

/// Chords whose webview default must never run, and ones blocked only
/// outside text fields (see `kit::dom::set_prevent`).
pub fn prevent_lists(mode: KeyboardMode) -> (Vec<&'static str>, Vec<&'static str>) {
    // Browser actions that would navigate away, open browser UI or steal keys
    // the app uses (reload, new window, find, print, tab switching...).
    let mut always = vec![
        "F1", "F3", "F6", "Shift+F6", "Ctrl+F6", "Ctrl+Shift+F6", "F7", "F10", "Ctrl+,", "Ctrl+E", "Alt+Q",
        "Ctrl+N", "Ctrl+Shift+N", "Ctrl+R", "Ctrl+Shift+R", "Ctrl+F", "Ctrl+Shift+F", "Ctrl+P", "Ctrl+S", "Ctrl+D",
        "Ctrl+L", "Ctrl+K", "Ctrl+G", "Ctrl+Shift+G", "Ctrl+Q", "Ctrl+T", "Ctrl+Shift+T", "Ctrl+Shift+M",
        "Ctrl+Shift+I", "Ctrl+Shift+J", "Ctrl+Shift+O", "Ctrl+H", "Ctrl+J", "Ctrl+O", "Ctrl+W", "Ctrl+Enter",
        "Alt+S", "Ctrl+=", "Ctrl+-", "Ctrl+0", "Ctrl+Alt+1", "Ctrl+Alt+2", "Ctrl+Alt+3", "Ctrl+Alt+4",
        "Alt+ArrowLeft", "Alt+ArrowRight", "Alt+Home",
    ];
    // Module keys and quick steps (browser tab switching otherwise).
    always.extend(["Ctrl+1", "Ctrl+2", "Ctrl+3", "Ctrl+4", "Ctrl+5", "Ctrl+6", "Ctrl+7", "Ctrl+8", "Ctrl+9"]);
    always.extend(["Ctrl+Shift+1", "Ctrl+Shift+2", "Ctrl+Shift+3", "Ctrl+Shift+4", "Ctrl+Shift+5", "Ctrl+Shift+6", "Ctrl+Shift+7", "Ctrl+Shift+8", "Ctrl+Shift+9"]);
    let mut outside = vec![
        "Ctrl+U", "Ctrl+Y", "Ctrl+A", "Ctrl+Z", "Ctrl+B", "Ctrl+I", "Backspace", "F5", "Shift+F5", "F9", "/", "'",
        "Shift+Delete", "Ctrl+Shift+V", "Ctrl+Shift+Y", "Ctrl+Space",
    ];
    if mode == KeyboardMode::Off {
        // Leave the webview's own keys alone, except region cycling.
        always.retain(|c| matches!(*c, "F6" | "Shift+F6" | "Ctrl+F6" | "Ctrl+Shift+F6" | "F10"));
        outside.clear();
    }
    (always, outside)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_keys_follow_the_spec() {
        let m = KeyboardMode::Outlook;
        assert_eq!(command_for("Ctrl+1", m), Some(GlobalCmd::GoMail));
        assert_eq!(command_for("Ctrl+2", m), Some(GlobalCmd::GoCalendar));
        assert_eq!(command_for("Ctrl+3", m), None, "Ctrl+3 stays reserved");
        assert_eq!(command_for("Ctrl+4", m), Some(GlobalCmd::GoPeople));
        assert_eq!(command_for("Ctrl+5", m), Some(GlobalCmd::GoTasks));
        assert_eq!(command_for("Ctrl+Shift+1", m), Some(GlobalCmd::GoMail));
        assert_eq!(command_for("Ctrl+Shift+4", m), Some(GlobalCmd::GoPeople));
        assert_eq!(command_for("Ctrl+Shift+3", m), None, "the web's Ctrl+Shift+3 stays unbound too");
    }

    #[test]
    fn question_mark_matches_with_or_without_shift() {
        assert_eq!(command_for("Shift+?", KeyboardMode::Outlook), Some(GlobalCmd::Shortcuts));
        assert_eq!(command_for("?", KeyboardMode::Outlook), Some(GlobalCmd::Shortcuts));
        assert_eq!(normalize("Ctrl+Shift+>"), "Ctrl+>");
        assert_eq!(normalize("Ctrl+Shift+R"), "Ctrl+Shift+R");
        assert_eq!(normalize("Shift+F6"), "Shift+F6");
        assert_eq!(normalize("Alt+Down"), "Alt+ArrowDown");
    }

    #[test]
    fn regions_work_with_shortcuts_off() {
        let off = KeyboardMode::Off;
        assert_eq!(command_for("F6", off), Some(GlobalCmd::NextRegion));
        assert_eq!(command_for("Shift+F6", off), Some(GlobalCmd::PrevRegion));
        assert_eq!(command_for("Ctrl+1", off), None);
        assert_eq!(command_for("Shift+?", off), None);
        assert_eq!(aria_for(GlobalCmd::GoMail, off), "");
        assert_eq!(aria_for(GlobalCmd::GoMail, KeyboardMode::Outlook), "Control+1");
    }

    #[test]
    fn web_style_sets_add_slash_search() {
        assert_eq!(command_for("/", KeyboardMode::Outlook), None);
        assert_eq!(command_for("/", KeyboardMode::Gmail), Some(GlobalCmd::Search));
        assert_eq!(command_for("Alt+Q", KeyboardMode::Outlook), Some(GlobalCmd::Search));
        assert_eq!(command_for("Ctrl+E", KeyboardMode::Outlook), Some(GlobalCmd::Search));
    }

    #[test]
    fn every_global_command_has_one_row() {
        use GlobalCmd::*;
        for c in [GoMail, GoCalendar, GoPeople, GoTasks, NextRegion, PrevRegion, Shortcuts, Search, Undo, NewMessage, Sync, Settings, Help] {
            assert_eq!(SHORTCUTS.iter().filter(|s| s.cmd == Some(c)).count(), 1, "{c:?}");
        }
    }

    #[test]
    fn prevent_lists_keep_text_editing_in_fields() {
        let (always, outside) = prevent_lists(KeyboardMode::Outlook);
        assert!(always.contains(&"Ctrl+R") && always.contains(&"F6") && always.contains(&"Ctrl+9"));
        assert!(!always.contains(&"Ctrl+Z") && outside.contains(&"Ctrl+Z"));
        let (always_off, outside_off) = prevent_lists(KeyboardMode::Off);
        assert!(always_off.contains(&"F6") && !always_off.contains(&"Ctrl+R") && outside_off.is_empty());
    }
}
