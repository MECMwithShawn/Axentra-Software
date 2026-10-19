//! `ComposePane`: one compose form, inline in the reading pane or in its
//! own window, with loading, autosave and the ribbon wiring.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axentra_core::{ComposeKind, Draft, MessageId};
use dioxus::prelude::*;

use super::actions;
use super::attachments::{self, AttachmentWell, FileInputs};
use super::commands;
use super::dialogs::ComposeDialogs;
use super::editor::BodyEditor;
use super::flyouts::ComposeFlyouts;
use super::footer::{ActionBar, Banners};
use super::header::{CompactHeader, FromRow, SubjectRow};
use super::mention::MentionList;
use super::recipients::RecipientRow;
use super::ribbon::ribbon_tabs;
use super::send_queue;
use super::state::{Field, Form, RIBBON, RibbonState};
use crate::kit::{Button, EmptyState, Icon, IconButton};
use crate::shell::ribbon::Ribbon;
use crate::state::{AppState, ComposeRequest, store, use_app};

static NEXT_UID: AtomicU64 = AtomicU64::new(1);

/// Autosave: 2 s after the last edit, and at least every 30 s while there
/// are unsaved changes (mail spec 6.7).
const SAVE_AFTER_IDLE: Duration = Duration::from_secs(2);
const SAVE_AT_LEAST_EVERY: Duration = Duration::from_secs(30);

/// Same compose session, wherever it is shown.
fn same_session(a: &ComposeRequest, b: &ComposeRequest) -> bool {
    a.kind == b.kind && a.source == b.source && a.draft == b.draft && a.to == b.to && a.subject == b.subject
}

/// The compose form for `request`, in the reading pane or (with
/// `request.popout`) in a window of its own. A new request replaces the
/// form (the old one saves itself); pop out / pop in keep it.
#[component]
pub fn ComposePane(request: ComposeRequest) -> Element {
    let app = use_app();
    let saved_id = use_hook(|| CopyValue::new(None::<MessageId>));
    let mut session = use_hook(|| CopyValue::new((0u64, request.clone())));
    let (serial, prev) = session.peek().clone();
    if !same_session(&prev, &request) {
        // The form itself points the request at its draft after the first
        // save; that is still the same form.
        let ours = request.draft.is_some() && request.draft == *saved_id.peek();
        session.set((if ours { serial } else { serial + 1 }, request.clone()));
    }
    let serial = session.peek().0;
    use_effect(move || send_queue::ensure_background(app));
    rsx! {
        ComposeForm { key: "{serial}", request, saved_id }
    }
}

/// Open the request's draft, or start a new one pre-filled for it.
fn load(req: &ComposeRequest) -> Result<Draft, String> {
    let s = store();
    if let Some(id) = req.draft {
        let mut d = s.draft(id).map_err(|e| e.to_string())?.ok_or_else(|| "This draft no longer exists.".to_string())?;
        if d.send_pending_until.is_some() {
            // Opening a message that waits out "Undo send" takes it back.
            if let Ok(Some(back)) = s.unqueue_send(id) {
                d = back;
            }
        }
        return Ok(d);
    }
    let mut d = s.new_draft(req.kind, None, req.source).map_err(|e| e.to_string())?;
    for a in &req.to {
        if !d.to.iter().any(|x| x.address.eq_ignore_ascii_case(&a.address)) {
            d.to.push(a.clone());
        }
    }
    if let Some(subject) = &req.subject {
        d.subject = subject.clone();
    }
    Ok(d)
}

#[derive(Clone, Copy, PartialEq)]
enum FirstFocus {
    To,
    Subject,
    Body,
}

#[component]
fn ComposeForm(request: ComposeRequest, saved_id: CopyValue<Option<MessageId>>) -> Element {
    let app = use_app();
    let loaded: Result<Draft, String> = use_hook(|| load(&request));
    let reopened = request.draft.is_some();
    let form = use_hook(|| {
        let draft = loaded.clone().unwrap_or_default();
        let mut sid = saved_id;
        sid.set(draft.id);
        // Fresh replies start with the one-line header; new messages,
        // forwards and reopened drafts show every field.
        let expanded = reopened || matches!(draft.kind, ComposeKind::New | ComposeKind::Forward);
        Form::new(NEXT_UID.fetch_add(1, Ordering::Relaxed), draft, saved_id, expanded)
    });
    use_context_provider(|| form);
    let first_focus = use_hook(|| {
        let d = form.draft.peek();
        let nobody = d.to.is_empty() && d.cc.is_empty() && d.bcc.is_empty();
        match d.kind {
            ComposeKind::Reply | ComposeKind::ReplyAll => FirstFocus::Body,
            _ if nobody => FirstFocus::To,
            _ if reopened => FirstFocus::Body,
            _ if d.subject.trim().is_empty() => FirstFocus::Subject,
            _ => FirstFocus::Body,
        }
    });

    // Initial focus (the body focuses itself once its content is loaded).
    use_effect(move || match first_focus {
        FirstFocus::To => form.focus(Field::To.key()),
        FirstFocus::Subject => form.focus("subject"),
        FirstFocus::Body => {}
    });

    // Autosave.
    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if *form.closed.peek() {
                break;
            }
            if !form.is_dirty() {
                continue;
            }
            let idle = form.last_edit.peek().elapsed() >= SAVE_AFTER_IDLE;
            let overdue = form.last_save.peek().elapsed() >= SAVE_AT_LEAST_EVERY;
            if (idle || overdue) && (form.draft.peek().id.is_some() || actions::has_content(form)) {
                actions::save_now(app, form);
            }
        }
    });

    // Ribbon commands picked in the Mail ribbon.
    let mut seen_cmd = use_hook(|| CopyValue::new(app.compose_cmd.peek().as_ref().map(|c| c.0).unwrap_or(0)));
    use_effect(move || {
        let cmd = app.compose_cmd.read().clone();
        if let Some((serial, id)) = cmd
            && serial > *seen_cmd.peek()
        {
            seen_cmd.set(serial);
            commands::run(app, form, &id);
        }
    });

    // "Recent files" for Attach file, refreshed as mail changes.
    let recent = use_memo(move || {
        let _ = app.mail_rev();
        store().recent_attachments(*form.saved_id.peek(), attachments::RECENT_FILES).unwrap_or_default()
    });

    // What the ribbon shows as pressed.
    use_effect(move || {
        let d = form.draft.read();
        let accounts = {
            let _ = app.accounts_rev();
            store().accounts().map(|a| a.len()).unwrap_or(1)
        };
        let st = RibbonState {
            fmt: form.fmt.read().clone(),
            importance: d.importance,
            show_from: (form.show_from)() || accounts > 1,
            from_locked: accounts > 1,
            show_bcc: (form.show_bcc)(),
            read_receipt: d.request_read_receipt,
            delivery_receipt: d.request_delivery_receipt,
            plain_text: d.plain_text,
            signature: !app.settings.read().signature_html.trim().is_empty(),
            color: form.color.read().clone(),
            highlight: form.highlight.read().clone(),
            scheduled: d.scheduled_send_at.is_some(),
            recent: recent(),
        };
        if *RIBBON.peek() != st {
            *RIBBON.write() = st;
        }
    });

    // Leaving without closing (another module, another request): keep the work.
    use_drop(move || save_on_drop(app, form));

    if let Err(msg) = loaded {
        return rsx! {
            div { class: "ax-compose ax-compose-full",
                EmptyState { icon: "error_circle", title: "Couldn't open the draft", subtitle: "{msg}",
                    Button {
                        label: "Close",
                        onclick: move |_| {
                            let mut c = app.compose;
                            c.set(None);
                        },
                    }
                }
            }
        };
    }
    let focus_body = first_focus == FirstFocus::Body;
    if request.popout {
        rsx! {
            div { class: "ax-compose-placeholder",
                EmptyState {
                    icon: "window_new",
                    title: "This message is open in a separate window",
                    subtitle: "Finish it there, or bring it back here.",
                    Button {
                        label: "Bring it back here",
                        onclick: move |_| { form.spawn(actions::set_popout(app, form, false)); },
                    }
                }
            }
            PopoutWindow { focus_body }
        }
    } else {
        rsx! { FormBody { popout: false, focus_body } }
    }
}

/// Save from a drop path, reading signals defensively (they may already
/// be gone while the scope is torn down).
fn save_on_drop(app: AppState, form: Form) {
    let Ok(closed) = form.closed.try_peek() else { return };
    if *closed {
        return;
    }
    drop(closed);
    let (Ok(dirty), Ok(saved)) = (form.dirty.try_peek().map(|v| *v), form.saved.try_peek().map(|v| *v)) else { return };
    if dirty == saved {
        return;
    }
    let (Ok(draft), Ok(body)) = (form.draft.try_peek().map(|d| d.clone()), form.body.try_peek().map(|b| b.clone())) else { return };
    let mut d = draft;
    d.body_html = body.html;
    let has_content = !d.to.is_empty() || !d.cc.is_empty() || !d.bcc.is_empty() || !d.subject.trim().is_empty() || !d.attachments.is_empty() || !body.text.trim().is_empty();
    if d.id.is_none() && !has_content {
        return;
    }
    let first = d.id.is_none();
    match store().save_draft(&mut d) {
        Ok(id) => {
            let mut sid = form.saved_id;
            if let Ok(mut s) = sid.try_write() {
                *s = Some(id);
            }
            if first {
                let mut compose = app.compose;
                if let Ok(mut c) = compose.try_write()
                    && let Some(r) = c.as_mut()
                {
                    r.draft = Some(id);
                }
            }
            app.bump_mail();
        }
        Err(e) => {
            app.error(format!("Couldn't save your draft: {e}"));
        }
    }
}

/// The form's own window (pop-out): title bar, compose ribbon, then the
/// form with the action bar at the top.
#[component]
fn PopoutWindow(focus_body: bool) -> Element {
    let app = use_app();
    let form = use_context::<Form>();
    let subject = form.draft.read().subject.clone();
    let title = if subject.trim().is_empty() { "(No subject)".to_string() } else { subject };
    rsx! {
        div { class: "ax-compose-window", role: "main", "aria-label": "New message",
            div { class: "ax-compose-titlebar",
                Icon { name: "mail", size: 16 }
                span { class: "ax-compose-titlebar-text ax-ellipsis", "{title}" }
                IconButton {
                    icon: "arrow_minimize",
                    title: "Move to the reading pane",
                    onclick: move |_| { form.spawn(actions::set_popout(app, form, false)); },
                }
                IconButton {
                    icon: "dismiss",
                    title: "Close (draft is saved)",
                    onclick: move |_| { form.spawn(actions::close(app, form)); },
                }
            }
            div { class: "ax-compose-window-ribbon",
                Ribbon { tabs: ribbon_tabs(), on_command: move |id: String| commands::run(app, form, &id) }
            }
            FormBody { popout: true, focus_body }
        }
    }
}

/// The fields, body and actions.
#[component]
fn FormBody(popout: bool, focus_body: bool) -> Element {
    let app = use_app();
    let form = use_context::<Form>();
    let d = form.draft.read();
    let kind = d.kind;
    drop(d);
    // The window always shows every field; only inline replies start compact.
    let expanded = (form.expanded)() || popout;
    let accounts = {
        let _ = app.accounts_rev();
        store().accounts().map(|a| a.len()).unwrap_or(1)
    };
    let show_from = (form.show_from)() || accounts > 1;
    let inline_reply = kind != ComposeKind::New && !popout;
    let popout_btn = (!popout).then(|| {
        rsx! {
            IconButton {
                icon: "open",
                title: "Open in new window",
                onclick: move |_| { form.spawn(actions::set_popout(app, form, true)); },
            }
        }
    });
    rsx! {
        div {
            id: form.dom("root"),
            class: "ax-compose",
            class: if popout { "ax-compose-popped" } else if inline_reply { "ax-compose-inline" } else { "ax-compose-full" },
            role: "region",
            "aria-label": if kind == ComposeKind::New { "New message" } else { "Compose" },
            onkeydown: move |e| commands::on_form_key(app, form, &e),
            if popout {
                ActionBar { top: true }
            }
            div { class: "ax-compose-scroll",
                div { class: "ax-compose-fields",
                    if !expanded {
                        CompactHeader { trailing: popout_btn }
                    } else {
                        if show_from { FromRow {} }
                        RecipientRow { field: Field::To, trailing: popout_btn }
                        if (form.show_cc)() { RecipientRow { field: Field::Cc } }
                        if (form.show_bcc)() { RecipientRow { field: Field::Bcc } }
                        SubjectRow {}
                    }
                }
                Banners {}
                BodyEditor { focus_start: focus_body }
                // After the body in tab order (keyboard spec 5.3); CSS shows
                // it above the body, under the subject, like Outlook.
                AttachmentWell {}
            }
            if !popout {
                ActionBar {}
            }
            div { class: "ax-compose-drop", "aria-hidden": "true",
                Icon { name: "attach", size: 32 }
                span { "Drop files here to attach them" }
            }
            FileInputs {}
            ComposeDialogs {}
            ComposeFlyouts {}
            MentionList {}
            div { class: "ax-visually-hidden", role: "status", "aria-live": "polite", "{form.live}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popout_does_not_start_a_new_session() {
        let a = ComposeRequest::new_message();
        let b = ComposeRequest { popout: true, ..a.clone() };
        assert!(same_session(&a, &b));
        let c = ComposeRequest { draft: Some(3), ..a.clone() };
        assert!(!same_session(&a, &c));
    }
}
