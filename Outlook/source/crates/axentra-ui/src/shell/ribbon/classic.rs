//! Classic ribbon row: labelled groups of large and small buttons (§6.2,
//! §6.4). Groups that don't fit collapse, rightmost first, into a dropdown
//! named after the group.

use std::rc::Rc;

use dioxus::prelude::*;

use super::command::{Look, RibbonButton};
use super::simplified::LayoutSwitch;
use super::{Command, RibbonTab, layout};
use crate::kit::{Icon, Menu};

/// A group's commands as columns: a large command alone, or up to three
/// small ones stacked.
fn columns(group: &[Command]) -> Vec<(bool, Vec<usize>)> {
    let sizes = layout::classic_sizes(group);
    let mut out: Vec<(bool, Vec<usize>)> = Vec::new();
    for (i, large) in sizes.into_iter().enumerate() {
        match out.last_mut() {
            Some((false, col)) if !large && col.len() < 3 => col.push(i),
            _ => out.push((large, vec![i])),
        }
    }
    out
}

/// A column of a shown group: (large, [(command index, key tip)]).
type Column = (bool, Vec<(usize, String)>);
/// A group slot: (group index, collapsed, columns, key tip when collapsed).
type GroupView = (usize, bool, Vec<Column>, String);

#[component]
pub fn ClassicRow(tab: RibbonTab, on_command: EventHandler<String>) -> Element {
    let mut width = use_signal(|| 0.0f64);
    let labels: Vec<String> = (0..tab.groups.len()).map(|i| tab.group_label(i)).collect();
    let collapsed = if width() > 0.0 { layout::fit_classic(&tab.groups, &labels, width()) } else { vec![false; tab.groups.len()] };
    // Key tips for every command shown and every collapsed group.
    let mut tip_items: Vec<(String, Option<String>)> = Vec::new();
    for (g, group) in tab.groups.iter().enumerate() {
        if collapsed[g] {
            tip_items.push((labels[g].clone(), None));
        } else {
            tip_items.extend(group.iter().map(|c| (c.label.clone(), c.keytip.clone())));
        }
    }
    tip_items.push(("Switch layout".into(), Some("ZL".into())));
    let tips = layout::assign_keytips(&tip_items);
    let mut next_tip = 0usize;
    let mut view: Vec<GroupView> = Vec::new();
    for (g, group) in tab.groups.iter().enumerate() {
        if collapsed[g] {
            view.push((g, true, Vec::new(), tips[next_tip].clone()));
            next_tip += 1;
            continue;
        }
        let cols = columns(group)
            .into_iter()
            .map(|(large, idx)| (large, idx.into_iter().map(|i| (i, tips[next_tip + i].clone())).collect()))
            .collect();
        next_tip += group.len();
        view.push((g, false, cols, String::new()));
    }
    let switch_tip = tips.last().cloned().unwrap_or_default();
    rsx! {
        div {
            class: "ax-ribbon-commands ax-ribbon-classic-row",
            role: "toolbar",
            "aria-label": "{tab.label}",
            "aria-orientation": "horizontal",
            "data-ax-roving": "horizontal",
            "data-ax-tab": "{tab.id}",
            onresize: move |e: ResizeEvent| {
                if let Ok(size) = e.get_content_box_size()
                    && (size.width - width()).abs() >= 4.0
                {
                    width.set(size.width);
                }
            },
            div { class: "ax-ribbon-row",
                for (vi, (g, is_collapsed, cols, tip)) in view.into_iter().enumerate() {
                    div { key: "g{g}", class: "ax-ribbon-cslot",
                        if vi > 0 { div { class: "ax-ribbon-sep", role: "separator", "aria-orientation": "vertical" } }
                        if is_collapsed {
                            CollapsedGroup { label: labels[g].clone(), commands: tab.groups[g].clone(), keytip: tip, on_command }
                        } else {
                            div { class: "ax-ribbon-cgroup", role: "group", "aria-label": "{labels[g]}",
                                div { class: "ax-ribbon-cgroup-body",
                                    for (ci, (large, items)) in cols.into_iter().enumerate() {
                                        div { key: "c{ci}", class: if large { "ax-ribbon-ccol ax-large" } else { "ax-ribbon-ccol" },
                                            for (i, t) in items {
                                                RibbonButton {
                                                    key: "{tab.groups[g][i].id}",
                                                    cmd: tab.groups[g][i].clone(),
                                                    look: if large { Look::Large } else { Look::Small },
                                                    keytip: t,
                                                    tab_stop: vi == 0 && ci == 0 && i == 0,
                                                    on_command,
                                                }
                                            }
                                        }
                                    }
                                }
                                div { class: "ax-ribbon-cgroup-label", "aria-hidden": "true", "{labels[g]}" }
                            }
                        }
                    }
                }
            }
            div { class: "ax-ribbon-end",
                LayoutSwitch { simplified: false, keytip: switch_tip, on_command }
            }
        }
    }
}

/// A group squeezed into one dropdown button named after the group.
#[component]
fn CollapsedGroup(label: String, commands: Vec<Command>, keytip: String, on_command: EventHandler<String>) -> Element {
    let mut menu_at = use_signal(|| None::<(f64, f64)>);
    let mut anchor = use_signal(|| None::<Rc<MountedData>>);
    let icon = commands.first().map(|c| c.icon.clone()).unwrap_or_else(|| "more_horizontal".into());
    let items: Vec<_> = commands.iter().map(Command::as_menu_item).collect();
    let name = if label.is_empty() { "More commands".to_string() } else { label.clone() };
    let open = move || {
        spawn(async move {
            let rect = match anchor() {
                Some(m) => m.get_client_rect().await.ok(),
                None => None,
            };
            menu_at.set(Some(rect.map(|r| (r.min_x(), r.max_y() + 4.0)).unwrap_or((300.0, 140.0))));
        });
    };
    rsx! {
        div { class: "ax-ribbon-cgroup ax-collapsed", role: "group", "aria-label": "{name}",
            button {
                r#type: "button",
                class: "ax-cmd ax-cmd-large",
                title: "{name}",
                "aria-haspopup": "menu",
                "aria-expanded": if menu_at().is_some() { "true" } else { "false" },
                "data-ax-item": "true",
                "data-ax-keytip": "{keytip}",
                tabindex: "-1",
                onmounted: move |e| anchor.set(Some(e.data())),
                onclick: move |_| open(),
                onkeydown: move |e: KeyboardEvent| {
                    if e.key() == Key::ArrowDown {
                        e.prevent_default();
                        e.stop_propagation();
                        open();
                    }
                },
                Icon { name: icon, size: 32 }
                span { class: "ax-cmd-label", "{name}" }
                Icon { name: "chevron_down", size: 12, class: "ax-cmd-caret" }
            }
            div { class: "ax-ribbon-cgroup-label", "aria-hidden": "true", "{label}" }
        }
        if let Some((x, y)) = menu_at() {
            Menu { items, x, y, label: "{name}", on_select: move |id| on_command.call(id), on_close: move |_| menu_at.set(None) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_commands_stack_three_per_column() {
        let group: Vec<Command> = ["A", "B", "C", "D", "E", "F"].iter().map(|l| Command::new(*l, *l, "mail")).collect();
        let cols = columns(&group);
        assert_eq!(cols, vec![(true, vec![0]), (true, vec![1]), (false, vec![2, 3, 4]), (false, vec![5])]);
    }
}
