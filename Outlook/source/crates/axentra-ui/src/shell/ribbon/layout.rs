//! Pure ribbon layout: estimated command widths, the Simplified overflow
//! algorithm (docs/spec/shell.md §6.11), Classic group sizing and key tips.
//!
//! Widths are estimated from the label text (Segoe UI / Selawik at 14px),
//! which keeps the layout deterministic and avoids measuring every command.

use super::custom::LabelMode;
use super::{Command, CommandSize};

/// Approximate advance of `text` at 14px in the UI font (slightly generous,
/// so estimates err towards overflowing early rather than clipping).
pub fn text_width(text: &str) -> f64 {
    1.04 * text
        .chars()
        .map(|c| match c {
            'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' => 3.6,
            'f' | 't' | 'r' | ' ' | '(' | ')' | '-' | '/' => 4.8,
            'm' | 'w' => 10.8,
            'M' | 'W' => 12.2,
            c if c.is_ascii_uppercase() => 8.6,
            c if c.is_ascii_digit() => 7.4,
            _ => 7.2,
        })
        .sum::<f64>()
}

const GAP: f64 = 2.0;
/// Separator line plus its margins and the gap after it.
const SEPARATOR: f64 = 11.0;
/// The layout-switch button at the far right.
pub const SWITCH_W: f64 = 34.0;
/// The "More options" overflow button.
pub const OVERFLOW_W: f64 = 34.0;

/// Width of a Simplified command, with or without its label.
pub fn command_width(cmd: &Command, labelled: bool) -> f64 {
    let label = if labelled && !cmd.label.is_empty() { 6.0 + text_width(&cmd.label) } else { 0.0 };
    let has_menu = !cmd.menu.is_empty();
    if cmd.split {
        // main part (padding 8 / 6) + 24px chevron part
        (8.0 + 20.0 + label + 6.0).max(32.0) + 24.0
    } else if has_menu {
        8.0 + 20.0 + label + 6.0 + 12.0 + 8.0
    } else if label > 0.0 {
        8.0 + 20.0 + label + 8.0
    } else {
        32.0
    }
}

/// Where each command of a tab goes in the Simplified layout.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Fit {
    /// (group, index) of commands on the row, with "label hidden" flags.
    pub visible: Vec<(usize, usize, bool)>,
    /// (group, index) of commands in the More options menu, in ribbon order.
    pub overflow: Vec<(usize, usize)>,
}

struct Item {
    g: usize,
    c: usize,
    rank: u32,
    full: f64,
    icon: f64,
    labelled: bool,
    hidden: bool,
}

fn row_width(items: &[Item], groups: usize) -> f64 {
    let mut w = 0.0;
    let mut n = 0;
    let mut used_groups = 0usize;
    for g in 0..groups {
        let mut any = false;
        for it in items.iter().filter(|i| i.g == g && !i.hidden) {
            w += if it.labelled { it.full } else { it.icon };
            n += 1;
            any = true;
        }
        if any {
            used_groups += 1;
        }
    }
    w + GAP * (n.max(1) - 1) as f64 + SEPARATOR * used_groups.saturating_sub(1) as f64
}

/// Fit commands into `avail` px: drop labels from the lowest-priority
/// commands first, then move whole commands into More options.
pub fn fit_simplified(groups: &[Vec<Command>], avail: f64) -> Fit {
    fit_simplified_with(groups, avail, LabelMode::AsSpacePermits, false)
}

/// [`fit_simplified`] with the Customize label mode: "always" never drops a
/// label (commands overflow instead), "never" shows icons only.
/// `more_always` reserves room for More options even when nothing
/// overflows (customisable tabs keep it for Customize and removed buttons).
pub fn fit_simplified_with(groups: &[Vec<Command>], avail: f64, labels: LabelMode, more_always: bool) -> Fit {
    let mut items: Vec<Item> = Vec::new();
    let mut flat = 0u32;
    for (g, group) in groups.iter().enumerate() {
        for (c, cmd) in group.iter().enumerate() {
            items.push(Item {
                g,
                c,
                rank: cmd.priority.map(|p| p as u32).unwrap_or(100 + flat),
                full: command_width(cmd, true),
                icon: command_width(cmd, false),
                labelled: labels != LabelMode::Never && !cmd.icon_only && !cmd.label.is_empty(),
                hidden: false,
            });
            flat += 1;
        }
    }
    let room = avail - SWITCH_W - GAP - if more_always { OVERFLOW_W + GAP } else { 0.0 };
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse((items[i].rank, i)));
    // 1. Drop labels, lowest priority first.
    if labels == LabelMode::AsSpacePermits {
        for &i in &order {
            if row_width(&items, groups.len()) <= room {
                break;
            }
            items[i].labelled = false;
        }
    }
    // 2. Move whole commands to the overflow menu.
    if row_width(&items, groups.len()) > room {
        let room = if more_always { room } else { room - OVERFLOW_W - GAP };
        for &i in &order {
            if row_width(&items, groups.len()) <= room {
                break;
            }
            items[i].hidden = true;
        }
    }
    Fit {
        visible: items.iter().filter(|i| !i.hidden).map(|i| (i.g, i.c, !i.labelled)).collect(),
        overflow: items.iter().filter(|i| i.hidden).map(|i| (i.g, i.c)).collect(),
    }
}

/// Large (icon over label) or small (icon beside label) in Classic.
pub fn classic_sizes(group: &[Command]) -> Vec<bool> {
    let hinted = group.iter().any(|c| c.size != CommandSize::Auto);
    group
        .iter()
        .enumerate()
        .map(|(i, c)| match c.size {
            CommandSize::Large => true,
            CommandSize::Small => false,
            CommandSize::Auto if hinted => false,
            CommandSize::Auto => group.len() <= 3 || i < 2,
        })
        .collect()
}

/// Estimated width of a Classic group (or of its collapsed dropdown).
pub fn classic_group_width(group: &[Command], label: &str, collapsed: bool) -> f64 {
    let label_w = text_width(label) * 12.0 / 14.0 + 16.0;
    if collapsed {
        return label_w.max(64.0) + SEPARATOR;
    }
    let sizes = classic_sizes(group);
    let mut w = 0.0;
    let mut column: Vec<f64> = Vec::new();
    let flush = |column: &mut Vec<f64>, w: &mut f64| {
        if let Some(m) = column.iter().cloned().reduce(f64::max) {
            *w += m + GAP;
        }
        column.clear();
    };
    for (cmd, large) in group.iter().zip(sizes) {
        if large {
            flush(&mut column, &mut w);
            let longest = cmd.label.split_whitespace().map(|word| text_width(word) * 12.0 / 14.0).fold(0.0, f64::max);
            w += (longest + 16.0).clamp(52.0, 76.0) + GAP;
        } else {
            let extra = if cmd.split { 20.0 } else if cmd.menu.is_empty() { 0.0 } else { 16.0 };
            column.push(4.0 + 16.0 + 6.0 + text_width(&cmd.label) + 8.0 + extra);
            if column.len() == 3 {
                flush(&mut column, &mut w);
            }
        }
    }
    flush(&mut column, &mut w);
    w.max(label_w) + 8.0 + SEPARATOR
}

/// Which Classic groups collapse into dropdowns to fit `avail` (rightmost first).
pub fn fit_classic(groups: &[Vec<Command>], labels: &[String], avail: f64) -> Vec<bool> {
    let mut collapsed = vec![false; groups.len()];
    let total = |collapsed: &[bool]| -> f64 {
        groups.iter().enumerate().map(|(i, g)| classic_group_width(g, &labels[i], collapsed[i])).sum::<f64>() + SWITCH_W
    };
    for i in (0..groups.len()).rev() {
        if total(&collapsed) <= avail {
            break;
        }
        collapsed[i] = true;
    }
    collapsed
}

/// Key tip letters for labels; explicit tips are kept, the rest get the
/// first letter when it's unique, else two letters, so no tip is a prefix
/// of another.
pub fn assign_keytips(items: &[(String, Option<String>)]) -> Vec<String> {
    let mut out: Vec<Option<String>> = items.iter().map(|(_, k)| k.as_ref().map(|k| k.to_uppercase())).collect();
    let first = |s: &str| s.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_ascii_uppercase());
    let taken_single: Vec<char> = out.iter().flatten().filter(|k| k.chars().count() == 1).filter_map(|k| k.chars().next()).collect();
    let mut counts = std::collections::HashMap::new();
    for (i, (label, _)) in items.iter().enumerate() {
        if out[i].is_none()
            && let Some(c) = first(label)
        {
            *counts.entry(c).or_insert(0) += 1;
        }
    }
    let mut used: Vec<String> = out.iter().flatten().cloned().collect();
    for (i, (label, _)) in items.iter().enumerate() {
        if out[i].is_some() {
            continue;
        }
        let Some(c) = first(label) else { continue };
        let single = c.to_string();
        if counts.get(&c) == Some(&1) && !used.contains(&single) && !used.iter().any(|u| u.starts_with(c)) {
            used.push(single.clone());
            out[i] = Some(single);
            continue;
        }
        // Two letters: the first letter plus a later letter of the label.
        let prefix = if taken_single.contains(&c) || used.contains(&single) { 'Z' } else { c };
        let pick = label
            .chars()
            .skip(1)
            .filter(|ch| ch.is_alphanumeric())
            .map(|ch| format!("{prefix}{}", ch.to_ascii_uppercase()))
            .find(|t| !used.contains(t))
            .unwrap_or_else(|| (1..).map(|n| format!("{prefix}{n}")).find(|t| !used.contains(t)).unwrap_or_default());
        used.push(pick.clone());
        out[i] = Some(pick);
    }
    out.into_iter().map(|k| k.unwrap_or_default()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmds(labels: &[&str]) -> Vec<Command> {
        labels.iter().map(|l| Command::new(l.to_lowercase(), *l, "mail")).collect()
    }

    #[test]
    fn everything_fits_when_wide() {
        let groups = vec![cmds(&["New mail", "Delete", "Archive"]), cmds(&["Reply", "Reply all", "Forward"])];
        let fit = fit_simplified(&groups, 2000.0);
        assert_eq!(fit.visible.len(), 6);
        assert!(fit.overflow.is_empty());
        assert!(fit.visible.iter().all(|(_, _, icon_only)| !icon_only));
    }

    #[test]
    fn labels_go_first_then_low_priority_commands() {
        let mut groups = vec![cmds(&["New mail", "Delete", "Archive"]), cmds(&["Reply", "Reply all", "Forward", "Snooze", "Print"])];
        groups[0][0] = groups[0][0].clone().priority(1);
        let wide = fit_simplified(&groups, 2000.0);
        let full = row_width_of(&groups, &wide);
        // A little narrower: some labels drop, nothing overflows.
        let fit = fit_simplified(&groups, full + SWITCH_W - 40.0);
        assert!(fit.overflow.is_empty());
        assert!(fit.visible.iter().any(|v| v.2), "a label was dropped");
        assert!(!fit.visible[0].2, "the top-priority command keeps its label");
        // Very narrow: the last commands overflow, in ribbon order.
        let tight = fit_simplified(&groups, 200.0);
        assert!(!tight.overflow.is_empty());
        assert_eq!(tight.visible[0], (0, 0, true), "New mail stays visible");
        assert!(tight.overflow.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(*tight.overflow.last().unwrap(), (1, 4), "Print goes first");
    }

    fn row_width_of(groups: &[Vec<Command>], fit: &Fit) -> f64 {
        fit.visible.iter().map(|&(g, c, icon)| command_width(&groups[g][c], !icon)).sum::<f64>() + 2.0 * 7.0 + 9.0
    }

    #[test]
    fn label_modes() {
        let groups = vec![cmds(&["New mail", "Delete", "Archive"]), cmds(&["Reply", "Reply all", "Forward"])];
        let never = fit_simplified_with(&groups, 2000.0, LabelMode::Never, false);
        assert!(never.visible.iter().all(|v| v.2), "hide labels always");
        let full = row_width_of(&groups, &fit_simplified(&groups, 2000.0));
        let always = fit_simplified_with(&groups, full + SWITCH_W - 40.0, LabelMode::Always, false);
        assert!(always.visible.iter().all(|v| !v.2), "labels stay");
        assert!(!always.overflow.is_empty(), "a command overflows instead");
        let auto = fit_simplified_with(&groups, full + SWITCH_W - 40.0, LabelMode::AsSpacePermits, false);
        assert!(auto.overflow.is_empty());
    }

    #[test]
    fn classic_sizes_follow_hints_or_position() {
        assert_eq!(classic_sizes(&cmds(&["A", "B"])), vec![true, true]);
        assert_eq!(classic_sizes(&cmds(&["A", "B", "C", "D"])), vec![true, true, false, false]);
        let mut g = cmds(&["A", "B", "C"]);
        g[1] = g[1].clone().large();
        assert_eq!(classic_sizes(&g), vec![false, true, false]);
    }

    #[test]
    fn classic_collapses_from_the_right() {
        let groups = vec![cmds(&["New mail"]), cmds(&["Delete", "Archive", "Report", "Sweep"]), cmds(&["Reply", "Reply all", "Forward"])];
        let labels = vec!["New".to_string(), "Delete".into(), "Respond".into()];
        assert_eq!(fit_classic(&groups, &labels, 2000.0), vec![false, false, false]);
        assert_eq!(fit_classic(&groups, &labels, 260.0), vec![false, true, true]);
    }

    #[test]
    fn keytips_are_unique_and_prefix_free() {
        let items: Vec<(String, Option<String>)> =
            ["New mail", "Delete", "Archive", "Reply", "Reply all", "Read / Unread", "Forward"].iter().map(|l| (l.to_string(), None)).collect();
        let tips = assign_keytips(&items);
        assert_eq!(tips[0], "N");
        assert_eq!(tips[1], "D");
        for (i, a) in tips.iter().enumerate() {
            for (j, b) in tips.iter().enumerate() {
                if i != j {
                    assert!(!b.starts_with(a.as_str()), "{a} is a prefix of {b}");
                }
            }
        }
        let explicit = assign_keytips(&[("Home".into(), Some("h".into())), ("Help".into(), None)]);
        assert_eq!(explicit[0], "H");
        assert!(explicit[1].len() == 2 && explicit[1] != "H");
    }
}
