//! The Customize dialog of the Simplified ribbon (docs/spec/shell.md §6.10):
//! "Buttons" (tick to add, untick to remove), a "Customize" preview with
//! reorder (drag, or the arrow buttons), rename / delete / add groups, the
//! label mode, and Reset all changes / Cancel / Save.

use axentra_core::AccountId;
use dioxus::prelude::*;

use super::RibbonTab;
use super::custom::{self, LabelMode, TabLayout};
use crate::kit::{Button, Checkbox, ConfirmDialog, Dialog, Icon, IconButton, Select, Variant, dom, keys};
use crate::state::use_app;

/// Waiting on a confirmation.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Confirm {
    DeleteGroup(usize),
    Reset,
}

#[component]
pub fn CustomizeDialog(tab: RibbonTab, account: AccountId, on_close: EventHandler<()>, on_saved: EventHandler<()>) -> Element {
    let app = use_app();
    let mut saved = use_signal({
        let tab = tab.clone();
        move || custom::load(account, &tab.id).resolved(&tab)
    });
    let mut draft = use_signal(move || saved.peek().clone());
    let mut renaming = use_signal(|| None::<usize>);
    let mut name_text = use_signal(String::new);
    let mut confirm = use_signal(|| None::<Confirm>);
    let mut dragging = use_signal(|| None::<String>);
    let tab_label = tab.label.clone();
    let layout = draft();
    let dirty = layout != *saved.read();
    let all: Vec<_> = {
        let mut seen = std::collections::HashSet::new();
        tab.groups.iter().flatten().filter(|c| seen.insert(c.id.clone())).cloned().collect()
    };
    let lookup = {
        let all = all.clone();
        move |id: &str| all.iter().find(|c| c.id == id).cloned()
    };

    let save = {
        let tab = tab.clone();
        move |_| {
            // An untouched layout is stored as "default", so later changes
            // to the module's ribbon show up as they are.
            let mut l = draft();
            if l == TabLayout::from_tab(&tab) {
                l = TabLayout::default();
            }
            if app.check(custom::save(account, &tab.id, &l)).is_some() {
                dom::announce("Ribbon saved", false);
                on_saved.call(());
                on_close.call(());
            }
        }
    };
    let mut commit_rename = move |g: usize| {
        let name = name_text().trim().to_string();
        if !name.is_empty() {
            draft.write().groups[g].name = name;
        }
        renaming.set(None);
    };

    rsx! {
        Dialog {
            title: "Customize the {tab_label} tab",
            width: 860,
            resize_key: "customize-ribbon", min_width: 640, min_height: 420,
            height: 640,
            class: "ax-customize",
            modal_scrim: true,
            on_close: move |_| on_close.call(()),
            actions: rsx! {
                Button {
                    label: "Reset all changes",
                    variant: Variant::Subtle,
                    class: "ax-customize-reset",
                    title: "Put the {tab_label} tab back the way it was",
                    onclick: move |_| confirm.set(Some(Confirm::Reset)),
                }
                Button { label: "Cancel", onclick: move |_| on_close.call(()) }
                Button {
                    label: "Save",
                    variant: Variant::Primary,
                    disabled: !dirty,
                    title: if dirty { "Save changes" } else { "No changes to save" },
                    onclick: save,
                }
            },
            div { class: "ax-customize-body",
                section { class: "ax-customize-buttons", "aria-labelledby": "ax-customize-buttons-title",
                    h3 { id: "ax-customize-buttons-title", class: "ax-customize-heading", "Buttons" }
                    p { class: "ax-set-desc", "Select the buttons to show on the ribbon. Cleared buttons stay under More options." }
                    div { class: "ax-customize-checklist",
                        for c in all.iter().cloned() {
                            div { key: "{c.id}", class: "ax-customize-check",
                                Checkbox {
                                    checked: layout.contains(&c.id),
                                    label: c.label.clone(),
                                    onchange: {
                                        let id = c.id.clone();
                                        let tab = tab.clone();
                                        move |on: bool| {
                                            let mut d = draft.write();
                                            if on { d.add(&tab, &id) } else { d.remove(&id) }
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
                section { class: "ax-customize-preview", "aria-labelledby": "ax-customize-preview-title",
                    div { class: "ax-row ax-customize-preview-head",
                        h3 { id: "ax-customize-preview-title", class: "ax-customize-heading ax-grow", "Customize" }
                        Button {
                            label: "Add group",
                            icon: "add",
                            variant: Variant::Subtle,
                            small: true,
                            onclick: move |_| {
                                let g = draft.write().add_group();
                                name_text.set(draft.read().groups[g].name.clone());
                                renaming.set(Some(g));
                            },
                        }
                    }
                    div { class: "ax-customize-groups",
                        for (g, group) in layout.groups.iter().cloned().enumerate() {
                            div {
                                key: "{g}-{group.name}",
                                class: "ax-customize-group",
                                role: "group",
                                "aria-label": "{group.name}",
                                "data-ax-drop": "true",
                                ondrop: move |e| {
                                    e.prevent_default();
                                    if let Some(id) = dragging() {
                                        draft.write().place(&id, g, None);
                                    }
                                    dragging.set(None);
                                },
                                div { class: "ax-customize-group-head",
                                    if renaming() == Some(g) {
                                        input {
                                            class: "ax-input ax-customize-rename",
                                            r#type: "text",
                                            "aria-label": "Group name",
                                            value: "{name_text}",
                                            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                                            oninput: move |e| name_text.set(e.value()),
                                            onblur: move |_| commit_rename(g),
                                            onkeydown: move |e: KeyboardEvent| {
                                                keys::stop_typing_keys(&e);
                                                match e.key() {
                                                    Key::Enter => commit_rename(g),
                                                    Key::Escape => {
                                                        e.stop_propagation();
                                                        renaming.set(None);
                                                    }
                                                    _ => {}
                                                }
                                            },
                                        }
                                    } else {
                                        span { class: "ax-customize-group-name ax-ellipsis", "{group.name}" }
                                        IconButton {
                                            icon: "edit",
                                            title: "Rename {group.name}",
                                            small: true,
                                            onclick: {
                                                let name = group.name.clone();
                                                move |_| {
                                                    name_text.set(name.clone());
                                                    renaming.set(Some(g));
                                                }
                                            },
                                        }
                                        IconButton {
                                            icon: "delete",
                                            title: "Delete {group.name}",
                                            small: true,
                                            class: "ax-customize-group-delete",
                                            onclick: move |_| confirm.set(Some(Confirm::DeleteGroup(g))),
                                        }
                                    }
                                }
                                div { class: "ax-customize-chips", role: "list",
                                    if group.commands.is_empty() {
                                        span { class: "ax-customize-empty", "Drag buttons here" }
                                    }
                                    for (i, id) in group.commands.iter().cloned().enumerate() {
                                        if let Some(cmd) = lookup(&id) {
                                            div {
                                                key: "{id}",
                                                class: "ax-customize-chip",
                                                class: if dragging().as_deref() == Some(id.as_str()) { "ax-dragging" },
                                                role: "listitem",
                                                draggable: "true",
                                                "data-ax-drop": "true",
                                                ondragstart: {
                                                    let id = id.clone();
                                                    move |_| dragging.set(Some(id.clone()))
                                                },
                                                ondragend: move |_| dragging.set(None),
                                                ondrop: {
                                                    let id = id.clone();
                                                    move |e: DragEvent| {
                                                        e.prevent_default();
                                                        e.stop_propagation();
                                                        if let Some(moving) = dragging() {
                                                            draft.write().place(&moving, g, Some(&id));
                                                        }
                                                        dragging.set(None);
                                                    }
                                                },
                                                Icon { name: cmd.icon.clone(), size: 16 }
                                                span { class: "ax-ellipsis", "{cmd.label}" }
                                                IconButton {
                                                    icon: "arrow_left",
                                                    title: "Move {cmd.label} left",
                                                    small: true,
                                                    disabled: g == 0 && i == 0,
                                                    onclick: {
                                                        let id = id.clone();
                                                        move |_| draft.write().shift(&id, -1)
                                                    },
                                                }
                                                IconButton {
                                                    icon: "arrow_right",
                                                    title: "Move {cmd.label} right",
                                                    small: true,
                                                    disabled: g + 1 == layout.groups.len() && i + 1 == group.commands.len(),
                                                    onclick: {
                                                        let id = id.clone();
                                                        move |_| draft.write().shift(&id, 1)
                                                    },
                                                }
                                                IconButton {
                                                    icon: "dismiss",
                                                    title: "Remove {cmd.label}",
                                                    small: true,
                                                    onclick: {
                                                        let id = id.clone();
                                                        move |_| draft.write().remove(&id)
                                                    },
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "ax-customize-labels",
                        Select {
                            label: "Labels",
                            value: layout.labels.id(),
                            options: LabelMode::ALL.iter().map(|m| (m.id().to_string(), m.label().to_string())).collect::<Vec<_>>(),
                            onchange: move |v: String| draft.write().labels = LabelMode::from_id(&v),
                        }
                    }
                }
            }
        }
        match confirm() {
            Some(Confirm::DeleteGroup(g)) => rsx! {
                ConfirmDialog {
                    title: "Delete this group?",
                    message: "Its buttons move to More options. You can't get the group back after you save.",
                    confirm_label: "Delete",
                    danger: true,
                    on_cancel: move |_| confirm.set(None),
                    on_confirm: move |_| {
                        draft.write().delete_group(g);
                        confirm.set(None);
                    },
                }
            },
            Some(Confirm::Reset) => rsx! {
                ConfirmDialog {
                    title: "Reset all changes?",
                    message: "This puts the {tab_label} tab back the way it was. You can't undo this.",
                    confirm_label: "Reset",
                    on_cancel: move |_| confirm.set(None),
                    on_confirm: {
                        let tab = tab.clone();
                        move |_| {
                            confirm.set(None);
                            if app.check(custom::save(account, &tab.id, &TabLayout::default())).is_some() {
                                draft.set(TabLayout::from_tab(&tab));
                                saved.set(TabLayout::from_tab(&tab));
                                dom::announce("Ribbon reset", false);
                                on_saved.call(());
                            }
                        }
                    },
                }
            },
            None => rsx! {},
        }
    }
}
