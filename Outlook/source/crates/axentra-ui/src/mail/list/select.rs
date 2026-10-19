//! Selection and keyboard navigation in the message list
//! (docs/spec/mail.md §3.12, keyboard spec §4.3).

use std::rc::Rc;

use axentra_core::ReadingPanePosition;
use dioxus::prelude::*;

use super::super::actions;
use super::super::ctx::{MailCtx, RowKey};
use super::super::model::{Entry, ListModel};
use crate::state::AppState;

/// Rows from `a` to `b` inclusive, in list order.
pub fn range(model: &ListModel, a: &RowKey, b: &RowKey) -> Vec<RowKey> {
    match (model.index_of(a), model.index_of(b)) {
        (Some(i), Some(j)) => model.keys[i.min(j)..=i.max(j)].to_vec(),
        (_, Some(j)) => vec![model.keys[j].clone()],
        _ => Vec::new(),
    }
}

fn hidden_pane(app: AppState) -> bool {
    app.settings.peek().reading_pane == ReadingPanePosition::Hidden
}

/// Open the single selected row, or nothing for zero / several.
fn sync_open(app: AppState, mut ctx: MailCtx, model: &ListModel) {
    let sel = ctx.selection.peek().clone();
    if sel.len() == 1 {
        actions::open_row(app, ctx, model, &sel[0]);
    } else {
        ctx.open.set(None);
    }
}

/// Select exactly one row and open it.
pub fn select_one(app: AppState, mut ctx: MailCtx, model: &ListModel, key: RowKey) {
    ctx.selection.set(vec![key.clone()]);
    ctx.anchor.set(Some(key.clone()));
    ctx.focus.set(Some(key.clone()));
    actions::open_row(app, ctx, model, &key);
    ctx.reveal(key);
}

/// Toggle one row in the selection (checkbox, Ctrl+click, Space).
pub fn toggle(app: AppState, mut ctx: MailCtx, model: &ListModel, key: RowKey) {
    let mut sel = ctx.selection.peek().clone();
    match sel.iter().position(|k| k == &key) {
        Some(i) => {
            sel.remove(i);
        }
        None => sel.push(key.clone()),
    }
    ctx.selection.set(sel);
    ctx.anchor.set(Some(key.clone()));
    ctx.focus.set(Some(key));
    sync_open(app, ctx, model);
}

/// Mouse click on a row.
pub fn click(app: AppState, mut ctx: MailCtx, model: &ListModel, key: RowKey, ctrl: bool, shift: bool) {
    if shift {
        let anchor = ctx.anchor.peek().clone().or_else(|| ctx.focus.peek().clone()).unwrap_or_else(|| key.clone());
        let r = range(model, &anchor, &key);
        ctx.selection.set(r);
        ctx.focus.set(Some(key));
        sync_open(app, ctx, model);
    } else if ctrl || *ctx.select_mode.peek() {
        toggle(app, ctx, model, key);
    } else {
        ctx.selection.set(vec![key.clone()]);
        ctx.anchor.set(Some(key.clone()));
        ctx.focus.set(Some(key.clone()));
        actions::open_row(app, ctx, model, &key);
        // With the reading pane off, opening a message fills the view.
        if hidden_pane(app) && model.message(&key).is_some_and(|m| !m.is_draft) {
            ctx.full_view.set(true);
        }
    }
}

/// Double-click / Enter: open the row and move into it.
pub fn activate(app: AppState, mut ctx: MailCtx, model: &ListModel, key: RowKey) {
    ctx.selection.set(vec![key.clone()]);
    ctx.anchor.set(Some(key.clone()));
    ctx.focus.set(Some(key.clone()));
    actions::open_row(app, ctx, model, &key);
    if model.message(&key).is_some_and(|m| m.is_draft) {
        return;
    }
    if hidden_pane(app) {
        ctx.full_view.set(true);
    }
    ctx.focus_reading += 1;
}

/// The conversation row a child row belongs to.
fn parent_of(model: &ListModel, key: &RowKey) -> Option<RowKey> {
    model.entries.iter().find_map(|e| match e {
        Entry::Child(c) if &c.key == key => Some(RowKey::Conv(c.conversation.clone())),
        _ => None,
    })
}

/// Arrow-key and selection keys inside the list. Returns true when handled.
pub fn list_key(app: AppState, mut ctx: MailCtx, model: Rc<ListModel>, chord: &str, page: usize) -> bool {
    let keys = &model.keys;
    if keys.is_empty() {
        return false;
    }
    let last = keys.len() - 1;
    let cur = ctx
        .focus
        .peek()
        .clone()
        .or_else(|| ctx.selection.peek().last().cloned())
        .and_then(|k| model.index_of(&k));
    let step = |delta: isize| -> usize {
        match cur {
            Some(i) => (i as isize + delta).clamp(0, last as isize) as usize,
            None => {
                if delta < 0 {
                    last
                } else {
                    0
                }
            }
        }
    };
    let m2 = model.clone();
    let extend_to = move |i: usize| {
        let mut ctx = ctx;
        let keys = &m2.keys;
        let anchor = ctx.anchor.peek().clone().or_else(|| cur.map(|c| keys[c].clone())).unwrap_or_else(|| keys[i].clone());
        let r = range(&m2, &anchor, &keys[i]);
        ctx.selection.set(r);
        ctx.focus.set(Some(keys[i].clone()));
        sync_open(app, ctx, &m2);
        ctx.reveal(keys[i].clone());
    };
    match chord {
        "ArrowDown" => select_one(app, ctx, &model, keys[step(1)].clone()),
        "ArrowUp" => select_one(app, ctx, &model, keys[step(-1)].clone()),
        "Home" | "Ctrl+Home" => select_one(app, ctx, &model, keys[0].clone()),
        "End" | "Ctrl+End" => select_one(app, ctx, &model, keys[last].clone()),
        "PageDown" => select_one(app, ctx, &model, keys[step(page as isize)].clone()),
        "PageUp" => select_one(app, ctx, &model, keys[step(-(page as isize))].clone()),
        "Shift+ArrowDown" => extend_to(step(1)),
        "Shift+ArrowUp" => extend_to(step(-1)),
        "Shift+Home" | "Ctrl+Shift+Home" => extend_to(0),
        "Shift+End" | "Ctrl+Shift+End" => extend_to(last),
        "Shift+PageDown" => extend_to(step(page as isize)),
        "Shift+PageUp" => extend_to(step(-(page as isize))),
        "Ctrl+ArrowDown" | "Ctrl+ArrowUp" => {
            let i = step(if chord.ends_with("Down") { 1 } else { -1 });
            ctx.focus.set(Some(keys[i].clone()));
            ctx.reveal(keys[i].clone());
        }
        "Space" | "Ctrl+Space" => {
            if let Some(i) = cur {
                toggle(app, ctx, &model, keys[i].clone());
            }
        }
        "Shift+Space" => {
            if let Some(i) = cur {
                extend_to(i);
            }
        }
        "Ctrl+A" => {
            ctx.selection.set(keys.clone());
            ctx.open.set(None);
        }
        "ArrowRight" => {
            let Some(i) = cur else { return false };
            match &keys[i] {
                RowKey::Conv(c) => {
                    let has_more = model.row(&keys[i]).is_some_and(|r| r.count > 1);
                    if !has_more {
                        return false;
                    }
                    if ctx.expanded.peek().as_deref() == Some(c.as_str()) {
                        if i < last {
                            ctx.focus.set(Some(keys[i + 1].clone()));
                            ctx.reveal(keys[i + 1].clone());
                        }
                    } else {
                        ctx.expanded.set(Some(c.clone()));
                    }
                }
                RowKey::Msg(_) => return false,
            }
        }
        "ArrowLeft" => {
            let Some(i) = cur else { return false };
            match &keys[i] {
                RowKey::Conv(c) if ctx.expanded.peek().as_deref() == Some(c.as_str()) => ctx.expanded.set(None),
                k => match parent_of(&model, k) {
                    Some(p) => {
                        ctx.focus.set(Some(p.clone()));
                        ctx.reveal(p);
                    }
                    None => return false,
                },
            }
        }
        "Enter" => {
            if let Some(i) = cur {
                activate(app, ctx, &model, keys[i].clone());
            }
        }
        "Shift+F10" | "ContextMenu" => {
            if let Some(i) = cur {
                super::row::open_context_menu_at_row(app, ctx, model.clone(), keys[i].clone());
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_follow_list_order() {
        let model = ListModel::from_keys(vec![RowKey::Msg(1), RowKey::Conv("a".into()), RowKey::Msg(3), RowKey::Msg(4)]);
        assert_eq!(range(&model, &RowKey::Msg(4), &RowKey::Conv("a".into())), vec![RowKey::Conv("a".into()), RowKey::Msg(3), RowKey::Msg(4)]);
        assert_eq!(range(&model, &RowKey::Msg(9), &RowKey::Msg(3)), vec![RowKey::Msg(3)]);
        assert!(range(&model, &RowKey::Msg(1), &RowKey::Msg(9)).is_empty());
    }
}
