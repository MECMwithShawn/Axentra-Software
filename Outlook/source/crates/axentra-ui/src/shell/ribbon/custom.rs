//! Simplified-ribbon customisation (docs/spec/shell.md §6.10): the user's
//! groups and command order for a tab, removed commands (they stay under
//! More options) and the label mode. Stored per account and tab in the
//! store's key/value table; only the Mail Home and View tabs are
//! customisable.

use std::collections::HashSet;

use axentra_core::AccountId;
use serde::{Deserialize, Serialize};

use super::{Command, RibbonTab};
use crate::state::{Module, store};

/// "Show labels as space permits" (default), "Show labels always", "Hide
/// labels always" (§6.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LabelMode {
    #[default]
    AsSpacePermits,
    Always,
    Never,
}

impl LabelMode {
    pub const ALL: [LabelMode; 3] = [LabelMode::AsSpacePermits, LabelMode::Always, LabelMode::Never];

    pub fn label(self) -> &'static str {
        match self {
            LabelMode::AsSpacePermits => "Show labels as space permits",
            LabelMode::Always => "Show labels always",
            LabelMode::Never => "Hide labels always",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            LabelMode::AsSpacePermits => "auto",
            LabelMode::Always => "always",
            LabelMode::Never => "never",
        }
    }

    pub fn from_id(id: &str) -> Self {
        LabelMode::ALL.into_iter().find(|m| m.id() == id).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomGroup {
    pub name: String,
    /// Command ids, in order.
    pub commands: Vec<String>,
}

/// A customised tab. Empty `groups` means the module's default layout.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabLayout {
    pub groups: Vec<CustomGroup>,
    /// Commands the user unticked; they stay reachable under More options.
    pub removed: Vec<String>,
    pub labels: LabelMode,
}

/// Only the Mail ribbon's Home and View tabs can be customised (§6.2).
pub fn customizable(module: Module, tab_id: &str) -> bool {
    module == Module::Mail && matches!(tab_id, "home" | "view")
}

fn default_group_name(tab: &RibbonTab, i: usize) -> String {
    let label = tab.group_label(i);
    if label.is_empty() { format!("Group {}", i + 1) } else { label }
}

impl TabLayout {
    /// The module's own layout as a starting point for editing.
    pub fn from_tab(tab: &RibbonTab) -> Self {
        let mut seen = HashSet::new();
        TabLayout {
            groups: tab
                .groups
                .iter()
                .enumerate()
                .map(|(i, g)| CustomGroup {
                    name: default_group_name(tab, i),
                    commands: g.iter().filter(|c| seen.insert(c.id.clone())).map(|c| c.id.clone()).collect(),
                })
                .collect(),
            removed: Vec::new(),
            labels: LabelMode::default(),
        }
    }

    /// The layout with its groups filled in (from the tab when empty), so an
    /// editor always works on explicit groups.
    pub fn resolved(&self, tab: &RibbonTab) -> Self {
        if self.groups.is_empty() {
            TabLayout { removed: self.removed.clone(), labels: self.labels, ..TabLayout::from_tab(tab) }
        } else {
            self.clone()
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.groups.iter().any(|g| g.commands.iter().any(|c| c == id))
    }

    /// Untick a command: it leaves its group and goes under More options.
    pub fn remove(&mut self, id: &str) {
        for g in &mut self.groups {
            g.commands.retain(|c| c != id);
        }
        if !self.removed.iter().any(|r| r == id) {
            self.removed.push(id.to_string());
        }
    }

    /// Tick a command: it goes to the end of its default group (or the last
    /// group when that one is gone).
    pub fn add(&mut self, tab: &RibbonTab, id: &str) {
        self.removed.retain(|r| r != id);
        if self.contains(id) {
            return;
        }
        let home = tab.groups.iter().position(|g| g.iter().any(|c| c.id == id)).map(|i| default_group_name(tab, i));
        let target = home.and_then(|name| self.groups.iter().position(|g| g.name == name));
        match target.or(self.groups.len().checked_sub(1)) {
            Some(i) => self.groups[i].commands.push(id.to_string()),
            None => self.groups.push(CustomGroup { name: default_group_name(tab, 0), commands: vec![id.to_string()] }),
        }
    }

    /// Move a command one place left (`-1`) or right (`1`), crossing into
    /// the neighbouring group at either end.
    pub fn shift(&mut self, id: &str, dir: i32) {
        let Some((g, i)) = self.position(id) else { return };
        let len = self.groups[g].commands.len();
        if dir < 0 && i > 0 {
            self.groups[g].commands.swap(i, i - 1);
        } else if dir > 0 && i + 1 < len {
            self.groups[g].commands.swap(i, i + 1);
        } else if dir < 0 && g > 0 {
            let c = self.groups[g].commands.remove(i);
            self.groups[g - 1].commands.push(c);
        } else if dir > 0 && g + 1 < self.groups.len() {
            let c = self.groups[g].commands.remove(i);
            self.groups[g + 1].commands.insert(0, c);
        }
    }

    /// Put `id` right before `before` (or at the end of group `group` when
    /// `before` is `None`): drag and drop.
    pub fn place(&mut self, id: &str, group: usize, before: Option<&str>) {
        if group >= self.groups.len() || before == Some(id) {
            return;
        }
        for g in &mut self.groups {
            g.commands.retain(|c| c != id);
        }
        self.removed.retain(|r| r != id);
        let cmds = &mut self.groups[group].commands;
        let at = before.and_then(|b| cmds.iter().position(|c| c == b)).unwrap_or(cmds.len());
        cmds.insert(at, id.to_string());
    }

    pub fn position(&self, id: &str) -> Option<(usize, usize)> {
        self.groups.iter().enumerate().find_map(|(g, grp)| grp.commands.iter().position(|c| c == id).map(|i| (g, i)))
    }

    /// Add an empty group with a unique name; returns its index.
    pub fn add_group(&mut self) -> usize {
        let name = (1..)
            .map(|n| if n == 1 { "New group".to_string() } else { format!("New group {n}") })
            .find(|n| !self.groups.iter().any(|g| &g.name == n))
            .unwrap_or_default();
        self.groups.push(CustomGroup { name, commands: Vec::new() });
        self.groups.len() - 1
    }

    /// Delete a group; its commands go under More options.
    pub fn delete_group(&mut self, g: usize) {
        if g < self.groups.len() {
            let gone = self.groups.remove(g);
            for c in gone.commands {
                if !self.removed.contains(&c) {
                    self.removed.push(c);
                }
            }
        }
    }

    /// The tab as the user arranged it, plus the commands they removed.
    /// Commands the module added since the layout was saved go to the end
    /// of their default group; unknown ids are ignored.
    pub fn apply(&self, tab: &RibbonTab) -> (RibbonTab, Vec<Command>) {
        let all: Vec<(usize, &Command)> = tab.groups.iter().enumerate().flat_map(|(g, cmds)| cmds.iter().map(move |c| (g, c))).collect();
        let find = |id: &str| all.iter().find(|(_, c)| c.id == id).map(|(_, c)| (*c).clone());
        let removed: Vec<Command> = self.removed.iter().filter_map(|id| find(id)).collect();
        if self.groups.is_empty() {
            let gone: HashSet<&str> = self.removed.iter().map(String::as_str).collect();
            let groups = tab.groups.iter().map(|g| g.iter().filter(|c| !gone.contains(c.id.as_str())).cloned().collect()).collect();
            return (RibbonTab { groups, ..tab.clone() }, removed);
        }
        let mut layout = self.clone();
        for (_, c) in &all {
            if !layout.contains(&c.id) && !layout.removed.contains(&c.id) {
                layout.add(tab, &c.id);
            }
        }
        let groups = layout
            .groups
            .iter()
            .map(|g| {
                g.commands
                    .iter()
                    .filter_map(|id| find(id))
                    .map(|mut c| {
                        c.group = Some(g.name.clone());
                        c
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|g| !g.is_empty())
            .collect();
        (RibbonTab { groups, ..tab.clone() }, removed)
    }
}

fn key(account: AccountId, tab_id: &str) -> String {
    format!("shell.ribbon.{account}.{tab_id}")
}

/// The saved layout of a tab for an account (default when none).
pub fn load(account: AccountId, tab_id: &str) -> TabLayout {
    store().kv_get(&key(account, tab_id)).ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
}

pub fn save(account: AccountId, tab_id: &str, layout: &TabLayout) -> axentra_core::Result<()> {
    let json = if *layout == TabLayout::default() { String::new() } else { serde_json::to_string(layout).unwrap_or_default() };
    store().kv_set(&key(account, tab_id), &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab() -> RibbonTab {
        RibbonTab::new(
            "home",
            "Home",
            vec![
                vec![Command::new("new", "New mail", "mail_add").group("New")],
                vec![Command::new("delete", "Delete", "delete").group("Delete"), Command::new("archive", "Archive", "archive")],
                vec![Command::new("reply", "Reply", "arrow_reply"), Command::new("forward", "Forward", "arrow_forward")],
            ],
        )
    }

    fn ids(t: &RibbonTab) -> Vec<Vec<&str>> {
        t.groups.iter().map(|g| g.iter().map(|c| c.id.as_str()).collect()).collect()
    }

    #[test]
    fn default_layout_is_the_module_layout() {
        let t = tab();
        let (shown, removed) = TabLayout::default().apply(&t);
        assert_eq!(shown, t);
        assert!(removed.is_empty());
        let l = TabLayout::from_tab(&t);
        assert_eq!(l.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["New", "Delete", "Group 3"]);
        assert_eq!(l.apply(&t).0.groups.len(), 3);
    }

    #[test]
    fn removed_commands_go_to_more_options_and_come_back_to_their_group() {
        let t = tab();
        let mut l = TabLayout::from_tab(&t);
        l.remove("archive");
        let (shown, removed) = l.apply(&t);
        assert_eq!(ids(&shown), vec![vec!["new"], vec!["delete"], vec!["reply", "forward"]]);
        assert_eq!(removed.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["archive"]);
        l.shift("delete", 1); // only command of its group: crosses into the next group
        assert!(l.groups[1].commands.is_empty());
        assert_eq!(l.groups[2].commands, ["delete", "reply", "forward"]);
        // The (now empty) "Delete" group still exists, so Archive returns there.
        l.add(&t, "archive");
        assert_eq!(l.position("archive"), Some((1, 0)));
        assert!(l.removed.is_empty());
        // Empty groups don't render.
        l.remove("archive");
        assert_eq!(l.apply(&t).0.groups.len(), 2);
    }

    #[test]
    fn reorder_rename_and_delete_groups() {
        let t = tab();
        let mut l = TabLayout::from_tab(&t);
        l.shift("forward", -1);
        assert_eq!(l.groups[2].commands, ["forward", "reply"]);
        l.shift("forward", -1);
        assert_eq!(l.groups[1].commands, ["delete", "archive", "forward"]);
        l.place("reply", 0, Some("new"));
        assert_eq!(l.groups[0].commands, ["reply", "new"]);
        let g = l.add_group();
        assert_eq!(l.groups[g].name, "New group");
        let g2 = l.add_group();
        assert_eq!(l.groups[g2].name, "New group 2");
        l.place("new", g, None);
        l.groups[g].name = "Mine".into();
        let (shown, _) = l.apply(&t);
        assert_eq!(shown.group_label(shown.groups.len() - 1), "Mine", "renamed groups label the classic layout too");
        l.delete_group(1);
        assert!(l.removed.contains(&"delete".to_string()) && l.removed.contains(&"forward".to_string()));
        let (shown, removed) = l.apply(&t);
        assert!(shown.groups.iter().flatten().all(|c| c.id != "delete"));
        assert_eq!(removed.len(), 3);
    }

    #[test]
    fn commands_new_to_the_module_still_show() {
        let mut t = tab();
        let l = TabLayout::from_tab(&t);
        t.groups[2].push(Command::new("meeting", "Meeting", "calendar_ltr"));
        t.groups[0].push(Command::new("template", "Mail from template", "mail_template").group("New"));
        let (shown, _) = l.apply(&t);
        let flat: Vec<&str> = shown.groups.iter().flatten().map(|c| c.id.as_str()).collect();
        assert!(flat.contains(&"meeting") && flat.contains(&"template"));
        assert_eq!(shown.groups[0].last().unwrap().id, "template", "back in its own group");
    }

    #[test]
    fn layouts_round_trip_and_label_modes_parse() {
        let mut l = TabLayout::from_tab(&tab());
        l.labels = LabelMode::Never;
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<TabLayout>(&json).unwrap(), l);
        assert_eq!(serde_json::from_str::<TabLayout>("{}").unwrap(), TabLayout::default());
        assert_eq!(LabelMode::from_id("always"), LabelMode::Always);
        assert_eq!(LabelMode::from_id("??"), LabelMode::AsSpacePermits);
        assert!(customizable(Module::Mail, "home") && !customizable(Module::Calendar, "home") && !customizable(Module::Mail, "help"));
    }
}
