//! Mail shortcuts for the default "Outlook" key set (keyboard spec §4).
//! List navigation lives in `list::select`; folder tree keys in `folders`.

use axentra_core::*;
use dioxus::prelude::*;

use super::actions;
use super::ctx::{MailCtx, MailDialog};
use super::list::select;
use super::model::ListModel;
use crate::kit::{dom, keys};
use crate::state::{AppState, store};

/// Command chords of the "Outlook" set (none of these work with shortcuts off).
fn command_for(chord: &str) -> Option<&'static str> {
    Some(match chord {
        "Ctrl+N" | "Ctrl+Shift+M" => "new_mail",
        "Ctrl+R" => "reply",
        "Ctrl+Shift+R" => "reply_all",
        "Ctrl+F" => "forward",
        "Ctrl+Alt+F" => "forward_attachment",
        "Backspace" => "archive",
        "Ctrl+Q" => "read",
        "Ctrl+U" => "unread",
        "Insert" => "flag_insert",
        "Ctrl+Shift+V" => "move_picker",
        "Ctrl+Shift+Y" => "copy_picker",
        "Ctrl+P" => "print",
        "Ctrl+Z" => "undo",
        _ => return None,
    })
}

/// Commands that make sense with nothing selected.
fn needs_selection(cmd: &str) -> bool {
    !matches!(cmd, "new_mail" | "undo")
}

/// Move the selection to the next / previous item (Ctrl+. / Ctrl+,).
fn step_message(app: AppState, ctx: MailCtx, model: &ListModel, delta: isize) {
    if model.keys.is_empty() {
        return;
    }
    let cur = ctx
        .focus
        .peek()
        .clone()
        .or_else(|| ctx.selection.peek().last().cloned())
        .and_then(|k| model.index_of(&k));
    let i = match cur {
        Some(i) => (i as isize + delta).clamp(0, model.keys.len() as isize - 1) as usize,
        None => 0,
    };
    select::select_one(app, ctx, model, model.keys[i].clone());
}

/// Handle a key at the module level. Returns true when it was used.
pub fn module_key(app: AppState, mut ctx: MailCtx, model: &ListModel, e: &KeyboardEvent) -> bool {
    if e.is_composing() || ctx.dialog.peek().is_some() || ctx.menu.peek().is_some() {
        return false;
    }
    let chord = keys::chord(e);
    let repeat = e.is_auto_repeating();
    let mode = app.settings.peek().keyboard_mode;
    let targets = actions::targets(ctx, model);

    // Delete, Shift+Delete and Escape stay on even with shortcuts turned off.
    match chord.as_str() {
        "Delete" => {
            if targets.is_empty() {
                return false;
            }
            actions::run(app, ctx, model, "delete", targets);
            return true;
        }
        "Shift+Delete" => {
            if targets.is_empty() || repeat {
                return !targets.is_empty();
            }
            ctx.dialog.set(Some(MailDialog::PermanentDelete(targets)));
            return true;
        }
        "Escape" => {
            if ctx.popout.peek().is_some() {
                ctx.popout.set(None);
                ctx.request_list_focus();
                return true;
            }
            if *ctx.full_view.peek() {
                ctx.full_view.set(false);
                ctx.request_list_focus();
                return true;
            }
            let sel = ctx.selection.peek().clone();
            if sel.len() > 1 {
                let keep = ctx.focus.peek().clone().filter(|k| sel.contains(k)).unwrap_or_else(|| sel[sel.len() - 1].clone());
                select::select_one(app, ctx, model, keep);
                return true;
            }
            if *ctx.select_mode.peek() {
                ctx.select_mode.set(false);
                return true;
            }
            if app.search.peek().active {
                actions::exit_search(app);
                return true;
            }
            return false;
        }
        _ => {}
    }
    // F6 cycles the regions (keyboard spec §12.1); not a shortcut, always on.
    // Mail's panes are regions of the shell's cycle (data-ax-region), so the
    // order is the app's: header, app bar, ribbon, folders, list, reading
    // pane, then an open side pane.
    match chord.as_str() {
        "F6" | "Ctrl+F6" => {
            dom::cycle_regions(1);
            return true;
        }
        "Shift+F6" | "Ctrl+Shift+F6" => {
            dom::cycle_regions(-1);
            return true;
        }
        _ => {}
    }
    if mode == KeyboardMode::Off {
        return false;
    }
    match chord.as_str() {
        "Shift+Enter" => {
            if !repeat
                && let Some(m) = actions::primary_message(ctx, model).or_else(|| targets.first().and_then(|id| store().message(*id).ok().flatten()))
            {
                actions::open_window(app, ctx, &m);
            }
            return true;
        }
        "Ctrl+." | "Ctrl+>" | "Ctrl+Shift+>" | "Ctrl+Shift+." => {
            step_message(app, ctx, model, 1);
            return true;
        }
        "Ctrl+," | "Ctrl+<" | "Ctrl+Shift+<" | "Ctrl+Shift+," => {
            step_message(app, ctx, model, -1);
            return true;
        }
        "Ctrl+Y" => {
            ctx.request_tree_focus();
            return true;
        }
        "Ctrl+Shift+I" => {
            let account = model.folder.as_ref().map(|f| f.account_id).or_else(|| store().primary_account().ok().flatten().map(|a| a.id));
            if let Some(inbox) = account.and_then(|a| store().folder_by_role(a, FolderRole::Inbox).ok().flatten()) {
                actions::go_to_folder(app, ctx, inbox.id);
            }
            return true;
        }
        "?" | "Shift+?" | "Shift+/" => {
            ctx.dialog.set(Some(MailDialog::Shortcuts));
            return true;
        }
        _ => {}
    }
    let Some(cmd) = command_for(&chord) else { return false };
    if repeat {
        // Held keys don't repeat commands; the key is still ours.
        return true;
    }
    if needs_selection(cmd) && targets.is_empty() {
        return true;
    }
    if cmd == "archive" && model.folder.as_ref().is_some_and(|f| f.role == FolderRole::Archive) {
        return true;
    }
    actions::run(app, ctx, model, cmd, targets);
    true
}

/// Browser defaults to suppress inside the Mail module (the liveview
/// preview can't prevent them from Rust; the desktop webview can, but this
/// is harmless there). Also sets up the drag ghost and the right-click
/// guard for the list and folder pane.
pub const HELPERS_JS: &str = r#"
if (!window.__axMailHelpers) {
  window.__axMailHelpers = true;
  const editable = (t) => t && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName));
  const chords = new Set(['r','f','p','u','q','n','y','e','a','z','.',',','>','<']);
  document.addEventListener('keydown', (e) => {
    const t = e.target;
    if (!t || !t.closest || !t.closest('.ax-mail') || editable(t)) return;
    const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    const nav = ['ArrowUp','ArrowDown','ArrowLeft','ArrowRight','Home','End','PageUp','PageDown',' ','Backspace'];
    if ((e.ctrlKey || e.metaKey) && chords.has(k)) { e.preventDefault(); return; }
    if (nav.includes(e.key) && t.closest('.ax-mail-scroll, .ax-mail-tree')) { e.preventDefault(); return; }
    if (e.key === 'Backspace' || e.key === 'F6' || e.key === 'F10' && e.shiftKey || e.key === 'ContextMenu') e.preventDefault();
  }, true);
  document.addEventListener('contextmenu', (e) => {
    const t = e.target;
    if (t && t.closest && t.closest('.ax-mail-scroll, .ax-mail-tree, .ax-menu, .ax-menu-backdrop')) e.preventDefault();
  }, true);
  document.addEventListener('dragstart', (e) => {
    const row = e.target && e.target.closest && e.target.closest('[data-drag-count]');
    if (!row || !e.dataTransfer) return;
    const n = parseInt(row.getAttribute('data-drag-count') || '1', 10);
    e.dataTransfer.effectAllowed = 'copyMove';
    try { e.dataTransfer.setData('text/plain', row.getAttribute('aria-label') || ''); } catch (_) {}
    // Message ids for drops outside Mail (My Day's To Do tab makes tasks).
    const list = row.closest('[data-selected-message-ids]');
    const ids = row.getAttribute('aria-selected') === 'true' && list && list.dataset.selectedMessageIds
      ? list.dataset.selectedMessageIds
      : row.dataset.messageIds || '';
    if (ids) try { e.dataTransfer.setData('application/x-axentra-message-ids', '[' + ids + ']'); } catch (_) {}
    const ghost = document.createElement('div');
    ghost.className = 'ax-mail-drag-ghost';
    ghost.innerHTML = '<span class="ax-icon" style="width:16px;height:16px">' + (window.__axMailIcon || '') + '</span><span></span>';
    ghost.lastChild.textContent = n === 1 ? '1 item' : n + ' items';
    (document.querySelector('.ax-app') || document.body).appendChild(ghost);
    e.dataTransfer.setDragImage(ghost, 14, 14);
    setTimeout(() => ghost.remove(), 0);
  }, true);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outlook_set_maps_documented_chords() {
        assert_eq!(command_for("Ctrl+R"), Some("reply"));
        assert_eq!(command_for("Ctrl+Shift+R"), Some("reply_all"));
        assert_eq!(command_for("Ctrl+F"), Some("forward"));
        assert_eq!(command_for("Ctrl+Q"), Some("read"));
        assert_eq!(command_for("Ctrl+U"), Some("unread"));
        assert_eq!(command_for("Insert"), Some("flag_insert"));
        assert_eq!(command_for("Backspace"), Some("archive"));
        assert_eq!(command_for("R"), None);
        assert!(!needs_selection("new_mail"));
        assert!(needs_selection("delete"));
    }
}
