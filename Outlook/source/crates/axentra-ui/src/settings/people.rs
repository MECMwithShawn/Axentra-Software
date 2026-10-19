//! Settings > People > Contacts (docs/spec/shell.md §7.6).

use dioxus::prelude::*;

use super::controls::{Section, use_settings};
use crate::kit::{RadioGroup, RadioOption};

#[component]
pub fn ContactsPage() -> Element {
    let ctx = use_settings();
    let s = ctx.get();
    rsx! {
        Section { title: "Sort contacts by",
            RadioGroup {
                label: "Sort contacts by",
                value: if s.contacts_sort_by_last { "last" } else { "first" },
                options: vec![RadioOption::new("first", "First name"), RadioOption::new("last", "Last name")],
                onchange: move |v: String| ctx.edit(|s| s.contacts_sort_by_last = v == "last"),
            }
        }
        Section { title: "Display contacts as",
            RadioGroup {
                label: "Display contacts as",
                value: if s.contacts_last_first { "last" } else { "first" },
                options: vec![RadioOption::new("first", "First Last"), RadioOption::new("last", "Last, First")],
                onchange: move |v: String| ctx.edit(|s| s.contacts_last_first = v == "last"),
            }
        }
    }
}
