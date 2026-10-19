//! The compose ribbon tabs: Message, Insert, Format text, Draw, Options
//! (mail spec 6.4). Every command id starts with `compose.`; picks come
//! back through `AppState::compose_command` and are run by
//! [`super::commands::run`].

use axentra_core::Importance;
use dioxus::prelude::*;

use super::attachments::file_icon;
use super::draw::{HIGHLIGHTER_COLORS, PEN_COLORS};
use super::format::{FONTS, HIGHLIGHTS, LINE_SPACINGS, SIZES, STANDARD_COLORS, STYLES, size_label};
use super::state::{RIBBON, RibbonState};
use crate::kit::MenuItem;
use crate::shell::ribbon::{Command, RibbonTab};

const PLAIN_TIP: &str = "Not available in plain text";

/// Ribbon tabs for the open compose form. Reads the form's state so
/// toggles (Bold, High importance, ...) show as pressed.
pub fn ribbon_tabs() -> Vec<RibbonTab> {
    let st = RIBBON.read().clone();
    vec![
        RibbonTab::new("compose_message", "Message", message_tab(&st)),
        RibbonTab::new("compose_insert", "Insert", insert_tab(&st)),
        RibbonTab::new("compose_format", "Format text", format_tab(&st)),
        RibbonTab::new("compose_draw", "Draw", draw_tab(&st)),
        RibbonTab::new("compose_options", "Options", options_tab(&st)),
    ]
}

/// A formatting command: disabled with a reason in plain text.
fn fmt_cmd(st: &RibbonState, id: &str, label: &str, icon: &str, keys: Option<&str>) -> Command {
    let tip = match keys {
        Some(k) => format!("{label} ({k})"),
        None => label.to_string(),
    };
    let c = Command::new(format!("compose.{id}"), label, icon).icon_only();
    if st.plain_text { c.disabled(true).tooltip(format!("{label}: {PLAIN_TIP}")) } else { c.tooltip(tip) }
}

fn font_cmd(st: &RibbonState) -> Command {
    let current = if st.fmt.font.is_empty() { "Aptos".to_string() } else { st.fmt.font.clone() };
    let mut items: Vec<MenuItem> = Vec::new();
    if !FONTS.iter().any(|f| f.eq_ignore_ascii_case(&current)) {
        items.push(MenuItem::new(format!("compose.font:{current}"), current.clone()).checked(true));
        items.push(MenuItem::divider());
    }
    items.extend(FONTS.iter().map(|f| MenuItem::new(format!("compose.font:{f}"), *f).checked(f.eq_ignore_ascii_case(&current))));
    let c = Command::new("compose.font", current.clone(), "text_font").menu(items);
    if st.plain_text { c.disabled(true).tooltip(format!("Font: {PLAIN_TIP}")) } else { c.tooltip(format!("Font: {current}")) }
}

fn size_cmd(st: &RibbonState) -> Command {
    let size = if st.fmt.size > 0.0 { st.fmt.size } else { 12.0 };
    let items = SIZES
        .iter()
        .map(|s| MenuItem::new(format!("compose.size:{s}"), size_label(*s)).checked((s - size).abs() < 0.01))
        .collect();
    let c = Command::new("compose.size", size_label(size), "text_font_size").menu(items);
    if st.plain_text { c.disabled(true).tooltip(format!("Font size: {PLAIN_TIP}")) } else { c.tooltip("Font size") }
}

fn highlight_cmd(st: &RibbonState) -> Command {
    let mut items: Vec<MenuItem> =
        HIGHLIGHTS.iter().map(|(hex, name)| MenuItem::new(format!("compose.highlight:{hex}"), *name).swatch(*hex)).collect();
    items.push(MenuItem::divider());
    items.push(MenuItem::new("compose.highlight:none", "No color").icon("prohibited"));
    let c = Command::new("compose.highlight", "Highlight", "highlight").icon_only().split(items);
    if st.plain_text { c.disabled(true).tooltip(format!("Highlight: {PLAIN_TIP}")) } else { c.tooltip("Highlight") }
}

fn color_cmd(st: &RibbonState) -> Command {
    let mut items = vec![MenuItem::new("compose.color:auto", "Automatic").icon("text_color"), MenuItem::divider(), MenuItem::header("Standard colors")];
    items.extend(STANDARD_COLORS.iter().map(|(hex, name)| MenuItem::new(format!("compose.color:{hex}"), *name).swatch(*hex)));
    items.push(MenuItem::divider());
    items.push(MenuItem::new("compose.palette", "More colors…").icon("color"));
    let c = Command::new("compose.color", "Font color", "text_color").icon_only().split(items);
    if st.plain_text { c.disabled(true).tooltip(format!("Font color: {PLAIN_TIP}")) } else { c.tooltip("Font color") }
}

fn bullets_cmd(st: &RibbonState) -> Command {
    let items = vec![
        MenuItem::new("compose.bullet_style:disc", "Filled circle"),
        MenuItem::new("compose.bullet_style:circle", "Hollow circle"),
        MenuItem::new("compose.bullet_style:square", "Square"),
    ];
    let c = Command::new("compose.bullets", "Bullets", "text_bullet_list_ltr").icon_only().split(items).toggled(st.fmt.ul);
    if st.plain_text { c.disabled(true).tooltip(format!("Bullets: {PLAIN_TIP}")) } else { c.tooltip("Bullets (Ctrl+Shift+L)") }
}

fn numbering_cmd(st: &RibbonState) -> Command {
    let items = vec![
        MenuItem::new("compose.number_style:decimal", "1. 2. 3."),
        MenuItem::new("compose.number_style:lower-alpha", "a. b. c."),
        MenuItem::new("compose.number_style:upper-alpha", "A. B. C."),
        MenuItem::new("compose.number_style:lower-roman", "i. ii. iii."),
        MenuItem::new("compose.number_style:upper-roman", "I. II. III."),
    ];
    let c = Command::new("compose.numbering", "Numbering", "text_number_list_ltr").icon_only().split(items).toggled(st.fmt.ol);
    if st.plain_text { c.disabled(true).tooltip(format!("Numbering: {PLAIN_TIP}")) } else { c.tooltip("Numbering") }
}

/// Attach file: the main part opens the file chooser; the menu lists up
/// to ten recent files, then "Browse this computer".
fn attach_cmd(st: &RibbonState) -> Command {
    let mut items = Vec::new();
    if !st.recent.is_empty() {
        items.push(MenuItem::header("Recent files"));
        for a in &st.recent {
            items.push(
                MenuItem::new(format!("compose.recent:{}", a.path), a.name.clone())
                    .icon(file_icon(&a.name, &a.content_type))
                    .shortcut(crate::fmt::file_size(a.size)),
            );
        }
        items.push(MenuItem::divider());
    }
    items.push(MenuItem::new("compose.attach", "Browse this computer").icon("folder_open"));
    Command::new("compose.attach", "Attach file", "attach").split(items).tooltip("Attach file")
}

fn signature_cmd(st: &RibbonState) -> Command {
    let mut items = Vec::new();
    if st.signature {
        items.push(MenuItem::new("compose.signature:default", "Signature").icon("signature"));
    } else {
        items.push(MenuItem::new("compose.signature:none", "No signatures yet").disabled(true));
    }
    items.push(MenuItem::divider());
    items.push(MenuItem::new("compose.signatures", "Signatures…").icon("settings"));
    Command::new("compose.signature", "Signature", "signature").menu(items).tooltip("Signature")
}

fn importance_cmds(st: &RibbonState) -> Vec<Command> {
    vec![
        Command::new("compose.importance_high", "High importance", if st.importance == Importance::High { "important_filled" } else { "important" })
            .icon_only()
            .toggled(st.importance == Importance::High)
            .tooltip("High importance"),
        Command::new("compose.importance_low", "Low importance", if st.importance == Importance::Low { "arrow_down_filled" } else { "arrow_down" })
            .icon_only()
            .toggled(st.importance == Importance::Low)
            .tooltip("Low importance"),
    ]
}

fn link_cmd(st: &RibbonState) -> Command {
    let c = Command::new("compose.link", "Link", "link");
    if st.plain_text { c.disabled(true).tooltip(format!("Link: {PLAIN_TIP}")) } else { c.tooltip("Insert link (Ctrl+K)") }
}

fn message_tab(st: &RibbonState) -> Vec<Vec<Command>> {
    vec![
        vec![font_cmd(st), size_cmd(st)],
        vec![
            fmt_cmd(st, "bold", "Bold", "text_bold", Some("Ctrl+B")).toggled(st.fmt.bold),
            fmt_cmd(st, "italic", "Italic", "text_italic", Some("Ctrl+I")).toggled(st.fmt.italic),
            fmt_cmd(st, "underline", "Underline", "text_underline", Some("Ctrl+U")).toggled(st.fmt.underline),
            highlight_cmd(st),
            color_cmd(st),
        ],
        vec![bullets_cmd(st), numbering_cmd(st)],
        vec![attach_cmd(st), link_cmd(st), signature_cmd(st)],
        importance_cmds(st),
        vec![Command::new("compose.discard", "Discard", "delete").tooltip("Discard")],
    ]
}

fn insert_tab(st: &RibbonState) -> Vec<Vec<Command>> {
    let pictures = Command::new("compose.picture", "Pictures", "image_add");
    let pictures = if st.plain_text { pictures.disabled(true).tooltip(format!("Pictures: {PLAIN_TIP}")) } else { pictures.tooltip("Insert pictures inline") };
    let table = Command::new("compose.table", "Table", "table_add");
    let table = if st.plain_text { table.disabled(true).tooltip(format!("Table: {PLAIN_TIP}")) } else { table.tooltip("Insert table") };
    let hr = Command::new("compose.hr", "Horizontal line", "line_horizontal_1");
    let hr = if st.plain_text { hr.disabled(true).tooltip(format!("Horizontal line: {PLAIN_TIP}")) } else { hr.tooltip("Insert horizontal line") };
    vec![
        vec![attach_cmd(st), link_cmd(st), pictures, table],
        vec![Command::new("compose.emoji", "Emoji", "emoji").tooltip("Emoji"), signature_cmd(st)],
        vec![hr, Command::new("compose.date", "Date & time", "calendar_clock").tooltip("Insert the current date and time")],
    ]
}

fn format_tab(st: &RibbonState) -> Vec<Vec<Command>> {
    let align = st.fmt.align.as_str();
    let line_items = LINE_SPACINGS.iter().map(|v| MenuItem::new(format!("compose.line_spacing:{v}"), *v)).collect();
    let style_name = STYLES.iter().find(|(_, _, tag)| *tag == st.fmt.block).map(|(_, l, _)| *l).unwrap_or("Normal");
    let style_items = STYLES
        .iter()
        .map(|(id, label, tag)| MenuItem::new(format!("compose.style:{id}"), *label).checked(*tag == st.fmt.block))
        .collect();
    let styles = Command::new("compose.styles", style_name, "text_paragraph").menu(style_items);
    let styles = if st.plain_text { styles.disabled(true).tooltip(format!("Styles: {PLAIN_TIP}")) } else { styles.tooltip("Styles") };
    let line = Command::new("compose.line_spacing", "Line spacing", "text_line_spacing").icon_only().menu(line_items);
    let line = if st.plain_text { line.disabled(true).tooltip(format!("Line spacing: {PLAIN_TIP}")) } else { line.tooltip("Line spacing") };
    let mode = Command::new("compose.mode", if st.plain_text { "Plain text" } else { "HTML" }, "code")
        .menu(vec![
            MenuItem::new("compose.mode:html", "HTML").checked(!st.plain_text),
            MenuItem::new("compose.mode:plain", "Plain text").checked(st.plain_text),
        ])
        .tooltip("Message format");
    let rtl = st.fmt.dir == "rtl";
    let direction = Command::new("compose.direction", "Text direction", if rtl { "text_paragraph_direction_left" } else { "text_paragraph_direction_right" })
        .icon_only()
        .menu(vec![
            MenuItem::new("compose.ltr", "Left-to-right").icon("text_paragraph_direction_right").checked(!rtl),
            MenuItem::new("compose.rtl", "Right-to-left").icon("text_paragraph_direction_left").checked(rtl),
        ]);
    let direction = if st.plain_text { direction.disabled(true).tooltip(format!("Text direction: {PLAIN_TIP}")) } else { direction.tooltip("Text direction") };
    let mut link = link_cmd(st);
    link.icon_only = true;
    vec![
        vec![
            font_cmd(st),
            size_cmd(st),
            fmt_cmd(st, "grow", "Increase font size", "font_increase", Some("Ctrl+]")),
            fmt_cmd(st, "shrink", "Decrease font size", "font_decrease", Some("Ctrl+[")),
        ],
        vec![
            fmt_cmd(st, "bold", "Bold", "text_bold", Some("Ctrl+B")).toggled(st.fmt.bold),
            fmt_cmd(st, "italic", "Italic", "text_italic", Some("Ctrl+I")).toggled(st.fmt.italic),
            fmt_cmd(st, "underline", "Underline", "text_underline", Some("Ctrl+U")).toggled(st.fmt.underline),
            fmt_cmd(st, "strike", "Strikethrough", "text_strikethrough", None).toggled(st.fmt.strike),
            fmt_cmd(st, "sub", "Subscript", "text_subscript", None).toggled(st.fmt.sub),
            fmt_cmd(st, "sup", "Superscript", "text_superscript", None).toggled(st.fmt.sup),
        ],
        vec![highlight_cmd(st), color_cmd(st), fmt_cmd(st, "clear", "Clear formatting", "text_clear_formatting", Some("Ctrl+Space"))],
        vec![
            bullets_cmd(st),
            numbering_cmd(st),
            fmt_cmd(st, "outdent", "Decrease indent", "text_indent_decrease_ltr", Some("Ctrl+Shift+T")),
            fmt_cmd(st, "indent", "Increase indent", "text_indent_increase_ltr", Some("Ctrl+T")),
            fmt_cmd(st, "left", "Align left", "text_align_left", Some("Ctrl+L")).toggled(align == "left"),
            fmt_cmd(st, "center", "Center", "text_align_center", Some("Ctrl+E")).toggled(align == "center"),
            fmt_cmd(st, "right", "Align right", "text_align_right", Some("Ctrl+R")).toggled(align == "right"),
            fmt_cmd(st, "justify", "Justify", "text_align_justify", Some("Ctrl+Shift+J")).toggled(align == "justify"),
            line,
            direction,
        ],
        vec![styles, link, mode],
    ]
}

/// Draw: a drawing canvas, opened with a pen or highlighter colour.
fn draw_tab(st: &RibbonState) -> Vec<Vec<Command>> {
    let ink = |tool: &str, colors: &[(&str, &str)]| -> Vec<MenuItem> {
        colors.iter().map(|(hex, name)| MenuItem::new(format!("compose.draw:{tool}:{hex}"), *name).swatch(*hex)).collect()
    };
    let canvas = Command::new("compose.draw", "Drawing canvas", "draw_image");
    let pen = Command::new("compose.draw:pen:#000000", "Pen", "inking_tool").split(ink("pen", PEN_COLORS));
    let highlighter = Command::new("compose.draw:highlighter:#FFFF00", "Highlighter", "highlight").split(ink("highlighter", HIGHLIGHTER_COLORS));
    let cmds: Vec<Command> = if st.plain_text {
        [canvas, pen, highlighter]
            .into_iter()
            .map(|c| {
                let tip = format!("{}: {PLAIN_TIP}", c.label);
                c.disabled(true).tooltip(tip)
            })
            .collect()
    } else {
        vec![canvas.tooltip("Insert a drawing"), pen.tooltip("Draw with a pen"), highlighter.tooltip("Draw with a highlighter")]
    };
    vec![cmds[..1].to_vec(), cmds[1..].to_vec()]
}

fn options_tab(st: &RibbonState) -> Vec<Vec<Command>> {
    let mut tags = importance_cmds(st);
    for c in &mut tags {
        c.icon_only = false;
    }
    vec![
        vec![
            if st.from_locked {
                Command::new("compose.show_from", "Show From", "person")
                    .toggled(true)
                    .disabled(true)
                    .tooltip("Show From: always shown when you have more than one account")
            } else {
                Command::new("compose.show_from", "Show From", "person").toggled(st.show_from).tooltip("Show From")
            },
            Command::new("compose.show_bcc", "Show Bcc", "people").toggled(st.show_bcc).tooltip("Show Bcc"),
        ],
        tags,
        vec![
            Command::new("compose.delivery_receipt", "Request a Delivery Receipt", "mail_checkmark")
                .toggled(st.delivery_receipt)
                .tooltip("Request a Delivery Receipt"),
            Command::new("compose.read_receipt", "Request a Read Receipt", "mail_read").toggled(st.read_receipt).tooltip("Request a Read Receipt"),
        ],
        vec![
            Command::new("compose.save", "Save draft", "save").tooltip("Save draft (Ctrl+S)"),
            Command::new("compose.schedule", "Schedule send", "send_clock").tooltip("Schedule send"),
        ],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(groups: &[Vec<Command>]) -> Vec<String> {
        groups.iter().flatten().map(|c| c.id.clone()).collect()
    }

    #[test]
    fn tabs_and_command_ids() {
        let st = RibbonState::default();
        let all: Vec<String> = [message_tab(&st), insert_tab(&st), format_tab(&st), draw_tab(&st), options_tab(&st)].iter().flat_map(|g| ids(g)).collect();
        assert!(all.iter().all(|id| id.starts_with("compose.")), "{all:?}");
        for id in ["compose.bold", "compose.attach", "compose.link", "compose.table", "compose.emoji", "compose.schedule", "compose.read_receipt", "compose.draw"] {
            assert!(all.iter().any(|a| a == id), "{id}");
        }
    }

    #[test]
    fn toggles_follow_state() {
        let mut st = RibbonState::default();
        st.fmt.bold = true;
        st.importance = Importance::High;
        let msg = message_tab(&st);
        let bold = msg.iter().flatten().find(|c| c.id == "compose.bold").unwrap();
        assert!(bold.toggled);
        let high = msg.iter().flatten().find(|c| c.id == "compose.importance_high").unwrap();
        assert!(high.toggled);
        st.plain_text = true;
        let fmt = format_tab(&st);
        let italic = fmt.iter().flatten().find(|c| c.id == "compose.italic").unwrap();
        assert!(italic.disabled);
        assert!(italic.tooltip.as_deref().unwrap_or_default().contains("plain text"));
    }
}
