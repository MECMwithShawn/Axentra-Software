//! The Add account dialog (§2.2): email → detection → Microsoft / Google
//! sign-in in the browser (or with a code), or the IMAP form → verifying →
//! done. Also used to sign in again to an account that needs it.

use axentra_core::{AccountId, AccountKind};
use axentra_sync::config::{AuthMethod, is_valid_email};
use axentra_sync::{AccountConfig, DeviceCodeInfo, Detection, Provider, Secret, SignInKind, SignInRequest, SignInTask, SignedIn, SyncError, SyncResult};
use chrono::Utc;
use dioxus::prelude::*;
use tokio::task::AbortHandle;

use super::engine;
use super::form::{Errors, ImapForm, ImapFormView};
use super::widgets::{AcctDialog, Busy, DialogButton, Field, LinkButton, MessageBar, copy_to_clipboard};
use crate::kit::{Icon, keys};
use crate::state::{store, use_app};

#[derive(Clone, Debug, PartialEq)]
enum Step {
    /// 1. Enter address.
    Email,
    /// "Other account": pick a provider.
    Providers,
    /// 2. Looking up your account.
    Detecting,
    /// 3a.
    Microsoft,
    /// 3b.
    Waiting { url: String },
    /// 3c.
    Device(DeviceCodeInfo),
    /// 3d.
    Google,
    /// 3e.
    Manual,
    /// Sign in again with a password.
    Password,
    /// 4. "Connecting to {host}…".
    Verifying(String),
    /// Signed in as somebody else (§2.4).
    Mismatch { flow: u64, actual: String },
    /// 5.
    Done { email: String, reauth: bool },
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Banner {
    intent: &'static str,
    text: String,
    detail: Option<String>,
    /// Offer "Retry" (offline) or "Get a new code" (device code expired).
    retry: bool,
}

fn banner(e: &SyncError) -> Banner {
    match e {
        SyncError::Tls { message, detail } => Banner { intent: "error", text: message.clone(), detail: Some(detail.clone()), retry: false },
        SyncError::Offline(m) => Banner { intent: "error", text: m.clone(), detail: None, retry: true },
        SyncError::User(m) if m == axentra_sync::error::copy::CODE_EXPIRED => Banner { intent: "error", text: m.clone(), detail: None, retry: true },
        SyncError::User(m) if m == axentra_sync::error::copy::ALREADY_PRESENT => Banner { intent: "info", text: m.clone(), detail: None, retry: false },
        other => Banner { intent: "error", text: other.to_string(), detail: None, retry: false },
    }
}

fn is_missing_client_id(e: &SyncError) -> bool {
    matches!(e, SyncError::User(m) if m == axentra_sync::error::copy::NO_CLIENT_ID)
}

fn google_configured() -> bool {
    axentra_sync::oauth::google_client().is_some()
}

/// What to show first when signing in again to an existing account.
fn reauth_start(id: AccountId) -> (Step, String) {
    let Some(a) = store().account(id).ok().flatten() else { return (Step::Email, String::new()) };
    let cfg = AccountConfig::of(&a);
    let step = match cfg.auth {
        AuthMethod::Password => Step::Password,
        AuthMethod::Xoauth2Google => Step::Google,
        _ => Step::Microsoft,
    };
    (step, a.email)
}

#[component]
pub fn AddAccountDialog(on_close: EventHandler<()>, #[props(default)] reauth: Option<AccountId>) -> Element {
    let app = use_app();
    let (first_step, first_email) = use_hook(|| reauth.map(reauth_start).unwrap_or((Step::Email, String::new())));
    let mut step = use_signal(|| first_step.clone());
    let mut email = use_signal(|| first_email.clone());
    let mut email_error = use_signal(|| None::<String>);
    let mut tenant = use_signal(|| "common".to_string());
    // Prefilled from AXENTRA_MS_CLIENT_ID or the last successful sign-in, so
    // Advanced can stay closed; it opens when sign-in finds no ID.
    let mut client_id = use_signal(|| axentra_sync::oauth::ms_client_id(None).unwrap_or_default());
    let remembered_id = use_hook(axentra_sync::oauth::client_id::remembered);
    let mut ms_advanced = use_signal(|| false);
    let mut banner_sig = use_signal(|| None::<Banner>);
    let mut form = use_signal(ImapForm::default);
    let mut errors = use_signal(Errors::new);
    let mut testing = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut abort = use_signal(|| None::<AbortHandle>);
    let mut password = use_signal(String::new);
    let mut password_error = use_signal(|| None::<String>);
    let mut now = use_signal(Utc::now);
    // Where "Back" / errors return to from the sign-in steps.
    let mut sign_in_step = use_signal(|| if first_step == Step::Google { Step::Google } else { Step::Microsoft });

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if matches!(*step.peek(), Step::Device(_)) {
                now.set(Utc::now());
            }
        }
    });
    use_drop(move || {
        if let Some(a) = abort.peek().as_ref() {
            a.abort();
        }
    });

    let mut cancel = move || {
        if let Some(a) = abort.take() {
            a.abort();
        }
        on_close.call(());
    };

    let sign_in_kind = move || -> SignInKind {
        if let Some(id) = reauth {
            return SignInKind::Reauth(id);
        }
        if *sign_in_step.peek() == Step::Google {
            SignInKind::Google
        } else {
            let id = client_id.peek().trim().to_string();
            SignInKind::Microsoft { tenant: tenant.peek().clone(), client_id: (!id.is_empty()).then_some(id) }
        }
    };

    // Account creation after a successful sign-in.
    let mut finish = move |flow: u64, who: String| {
        step.set(Step::Verifying(who.clone()));
        spawn(async move {
            match engine().finish_sign_in(flow).await {
                Ok(_) => {
                    app.bump_accounts();
                    step.set(Step::Done { email: who, reauth: reauth.is_some() });
                }
                Err(e) => {
                    banner_sig.set(Some(banner(&e)));
                    step.set(sign_in_step());
                }
            }
        });
    };

    let mut handle_result = move |r: SyncResult<SignedIn>| {
        abort.set(None);
        match r {
            Ok(s) if s.matches => finish(s.flow, s.email),
            Ok(s) => step.set(Step::Mismatch { flow: s.flow, actual: s.email }),
            Err(SyncError::Cancelled) => {}
            Err(e) => {
                banner_sig.set(Some(banner(&e)));
                step.set(sign_in_step());
            }
        }
    };

    let mut start_browser = move |select_account: bool| {
        banner_sig.set(None);
        busy.set(true);
        let req = SignInRequest { email: email.peek().trim().to_string(), kind: sign_in_kind(), select_account };
        spawn(async move {
            match engine().start_browser_sign_in(req).await {
                Err(e) => {
                    busy.set(false);
                    if is_missing_client_id(&e) {
                        ms_advanced.set(true);
                    }
                    banner_sig.set(Some(banner(&e)));
                    step.set(sign_in_step());
                }
                Ok((url, task)) => {
                    let SignInTask { abort: a, handle: _ } = &task;
                    abort.set(Some(a.clone()));
                    busy.set(false);
                    step.set(Step::Waiting { url });
                    let r = task.wait().await;
                    handle_result(r);
                }
            }
        });
    };

    let mut start_device = move || {
        banner_sig.set(None);
        busy.set(true);
        let req = SignInRequest { email: email.peek().trim().to_string(), kind: sign_in_kind(), select_account: false };
        spawn(async move {
            match engine().start_device_sign_in(req).await {
                Err(e) => {
                    busy.set(false);
                    if is_missing_client_id(&e) {
                        ms_advanced.set(true);
                    }
                    banner_sig.set(Some(banner(&e)));
                    step.set(Step::Microsoft);
                }
                Ok((info, task)) => {
                    abort.set(Some(task.abort.clone()));
                    busy.set(false);
                    now.set(Utc::now());
                    step.set(Step::Device(info));
                    let r = task.wait().await;
                    handle_result(r);
                }
            }
        });
    };

    let mut to_manual = move |d: Option<Detection>, provider: Option<Provider>| {
        let e = email.peek().trim().to_string();
        let f = match (provider, d) {
            (Some(p), _) => ImapForm::for_provider(p, &e, ""),
            (None, d) => ImapForm::new(&e, "", d.as_ref()),
        };
        form.set(f);
        errors.set(Errors::new());
        banner_sig.set(None);
        step.set(Step::Manual);
    };

    let mut to_microsoft = move |t: &str| {
        tenant.set(t.to_string());
        sign_in_step.set(Step::Microsoft);
        banner_sig.set(None);
        step.set(Step::Microsoft);
    };

    let mut to_google = move || {
        if google_configured() {
            sign_in_step.set(Step::Google);
            banner_sig.set(None);
            step.set(Step::Google);
        } else {
            to_manual(None, Some(Provider::Google));
        }
    };

    let mut continue_email = move || {
        let e = email.peek().trim().to_string();
        if !is_valid_email(&e) {
            email_error.set(Some("Enter an email address in the format someone@example.com.".into()));
            return;
        }
        let existing = store().accounts().unwrap_or_default().into_iter().any(|a| a.kind != AccountKind::Local && a.email.eq_ignore_ascii_case(&e));
        if existing {
            email_error.set(Some(axentra_sync::error::copy::ALREADY_PRESENT.into()));
            return;
        }
        step.set(Step::Detecting);
        spawn(async move {
            let detection = engine().detect(e).await.ok();
            if *step.peek() != Step::Detecting {
                return;
            }
            match detection.as_ref().map(|d| d.provider) {
                Some(Provider::MicrosoftWork) => to_microsoft("organizations"),
                Some(Provider::MicrosoftPersonal) => to_microsoft("consumers"),
                Some(Provider::Google) => to_google(),
                _ => to_manual(detection, None),
            }
        });
    };

    let mut sign_in_imap = move || {
        let f = form.peek().clone();
        let errs = f.validate();
        if !errs.is_empty() {
            errors.set(errs);
            return;
        }
        banner_sig.set(None);
        step.set(Step::Verifying(f.imap_host.trim().to_string()));
        spawn(async move {
            match engine().add_imap(f.setup()).await {
                Ok(_) => {
                    app.bump_accounts();
                    step.set(Step::Done { email: f.email.trim().to_string(), reauth: false });
                }
                Err(e) => {
                    step.set(Step::Manual);
                    match &e {
                        SyncError::Auth(m) => {
                            errors.write().insert("password", m.clone());
                        }
                        _ => banner_sig.set(Some(banner(&e))),
                    }
                }
            }
        });
    };

    let mut test_connection = move || {
        let f = form.peek().clone();
        let errs = f.validate();
        if !errs.is_empty() {
            errors.set(errs);
            return;
        }
        testing.set(true);
        banner_sig.set(None);
        spawn(async move {
            let r = engine().test_imap(f.setup()).await;
            testing.set(false);
            match r {
                Ok(_) => banner_sig.set(Some(Banner {
                    intent: "success",
                    text: format!("Connected to {} and {}.", f.imap_host.trim(), f.smtp_host.trim()),
                    ..Default::default()
                })),
                Err(SyncError::Auth(m)) => {
                    errors.write().insert("password", m);
                }
                Err(e) => banner_sig.set(Some(banner(&e))),
            }
        });
    };

    let mut sign_in_password = move || {
        let pw = password.peek().clone();
        if pw.is_empty() {
            password_error.set(Some("Enter your password.".into()));
            return;
        }
        let Some(id) = reauth else { return };
        let who = email.peek().clone();
        step.set(Step::Verifying(who.clone()));
        spawn(async move {
            match engine().update_password(id, Secret::new(pw)).await {
                Ok(()) => {
                    app.bump_accounts();
                    step.set(Step::Done { email: who, reauth: true });
                }
                Err(SyncError::Auth(m)) => {
                    password_error.set(Some(m));
                    step.set(Step::Password);
                }
                Err(e) => {
                    banner_sig.set(Some(banner(&e)));
                    step.set(Step::Password);
                }
            }
        });
    };

    let current = step();
    let title = match &current {
        Step::Microsoft => "Sign in with Microsoft",
        Step::Waiting { .. } => "Finish signing in",
        Step::Device(_) => "Sign in on another device",
        Step::Google => "Sign in with Google",
        Step::Manual => "Set up your account",
        Step::Password => "Sign in again",
        Step::Done { reauth: true, .. } => "You're signed in",
        Step::Done { .. } => "Account added",
        _ => "Add an account",
    };
    let bar = banner_sig();
    let bar_view = bar.clone().map(|b| {
        let retry_label = if matches!(current, Step::Device(_) | Step::Microsoft) && b.retry && b.text == axentra_sync::error::copy::CODE_EXPIRED {
            "Get a new code"
        } else {
            "Retry"
        };
        let actions = if b.retry {
                    rsx! {
                        button {
                            r#type: "button",
                            class: "ax-btn ax-small",
                            onclick: move |_| {
                                banner_sig.set(None);
                                match step() {
                                    Step::Manual => sign_in_imap(),
                                    Step::Device(_) => start_device(),
                                    Step::Microsoft if retry_label == "Get a new code" => start_device(),
                                    Step::Microsoft | Step::Google => start_browser(false),
                                    Step::Password => sign_in_password(),
                                    _ => {}
                                }
                            },
                            "{retry_label}"
                        }
                    }
                } else {
                    rsx! {}
                };
        rsx! {
            MessageBar {
                intent: b.intent,
                text: b.text.clone(),
                detail: b.detail.clone(),
                on_dismiss: move |_| banner_sig.set(None),
                actions,
            }
        }
    });

    // One dialog for every step (so it doesn't re-open between steps); each
    // step supplies its body, actions and optional footer link.
    let mut footer_start: Option<Element> = None;
    let (actions, body): (Element, Element) = match current {
        Step::Email => (
            rsx! {
                DialogButton { label: "Cancel", onclick: move |_| cancel() }
                DialogButton {
                    label: "Continue",
                    primary: true,
                    disabled: !is_valid_email(&email()),
                    title: if is_valid_email(&email()) { None } else { Some("Enter an email address to continue".to_string()) },
                    onclick: move |_| continue_email(),
                }
            },
            rsx! {
                div { class: "ax-acct-step",
                    Field {
                        label: "Email address",
                        value: email(),
                        placeholder: "someone@example.com",
                        input_type: "email",
                        autocomplete: "email",
                        autofocus: true,
                        error: email_error(),
                        oninput: move |v: String| {
                            email.set(v);
                            email_error.set(None);
                        },
                        onenter: move |_| continue_email(),
                    }
                    div { class: "ax-acct-links",
                        LinkButton { label: "Work or school account", onclick: move |_| to_microsoft("organizations") }
                        LinkButton { label: "Other account", onclick: move |_| step.set(Step::Providers) }
                    }
                }
            },
        ),
        Step::Providers => (
            rsx! {
                DialogButton { label: "Back", onclick: move |_| step.set(Step::Email) }
            },
            rsx! {
                div { class: "ax-acct-step",
                    p { class: "ax-acct-text", "Choose your email provider." }
                    div { class: "ax-acct-providers", role: "list",
                        ProviderTile { icon: "building", label: "Microsoft 365", sub: "Work or school account", autofocus: true, onclick: move |_| to_microsoft("organizations") }
                        ProviderTile { icon: "person_mail", label: "Outlook.com", sub: "Outlook.com, Hotmail, Live or MSN", onclick: move |_| to_microsoft("consumers") }
                        ProviderTile { icon: "mail", label: "Gmail", sub: if google_configured() { "Sign in with Google" } else { "With an app password" }, onclick: move |_| to_google() }
                        ProviderTile { icon: "globe", label: "iCloud", sub: "With an app-specific password", onclick: move |_| to_manual(None, Some(Provider::ICloud)) }
                        ProviderTile { icon: "mail_inbox", label: "Yahoo", sub: "With an app password", onclick: move |_| to_manual(None, Some(Provider::Yahoo)) }
                        ProviderTile { icon: "settings", label: "Other (IMAP)", sub: "Enter server settings", onclick: move |_| to_manual(None, Some(Provider::Imap)) }
                    }
                }
            },
        ),
        Step::Detecting => (
            rsx! { DialogButton { label: "Cancel", onclick: move |_| cancel(), autofocus: true } },
            rsx! { div { class: "ax-acct-step", Busy { label: "Looking up your account…" } } },
        ),
        Step::Microsoft | Step::Google => {
            let google = current == Step::Google;
            (
                rsx! {
                    DialogButton { label: "Cancel", onclick: move |_| cancel() }
                    DialogButton { label: "Open browser", primary: true, autofocus: true, disabled: busy(), onclick: move |_| start_browser(false) }
                },
                rsx! {
                    div { class: "ax-acct-step",
                        {bar_view}
                        p { class: "ax-acct-text", "We'll open your web browser so you can sign in securely." }
                        if !email().is_empty() {
                            div { class: "ax-acct-identity", Icon { name: "person_circle", size: 20 } span { "{email}" } }
                        }
                        if !google {
                            div { class: "ax-acct-links",
                                LinkButton { label: "Use a code instead", disabled: busy(), onclick: move |_| start_device() }
                            }
                        }
                        if !google && reauth.is_none() {
                            super::widgets::Expander { label: "Advanced", open: ms_advanced(), ontoggle: move |_| ms_advanced.toggle(),
                                Field {
                                    label: "Application (client) ID",
                                    value: client_id(),
                                    placeholder: "00000000-0000-0000-0000-000000000000",
                                    hint: if remembered_id.as_deref().is_some_and(|r| r == client_id().trim()) {
                                        "Used for your last Microsoft sign-in. From your Microsoft Entra app registration (docs/ACCOUNTS.md)."
                                    } else {
                                        "From your Microsoft Entra app registration. See docs/ACCOUNTS.md."
                                    },
                                    oninput: move |v: String| client_id.set(v),
                                    onenter: move |_| start_browser(false),
                                }
                            }
                        }
                    }
                },
            )
        }
        Step::Waiting { url } => (
            rsx! { DialogButton { label: "Cancel", onclick: move |_| cancel(), autofocus: true } },
            rsx! {
                div { class: "ax-acct-step",
                    p { class: "ax-acct-text", "Complete sign-in in your browser, then come back here." }
                    Busy { label: "Waiting for your browser…" }
                    div { class: "ax-acct-links",
                        LinkButton {
                            label: "Copy sign-in link",
                            onclick: move |_| {
                                copy_to_clipboard(&url);
                                app.toast("Sign-in link copied");
                            },
                        }
                    }
                }
            },
        ),
        Step::Device(info) => {
            let left = (info.expires_at - now()).num_seconds().max(0);
            let code = info.user_code.clone();
            let code2 = code.clone();
            let uri = info.verification_uri.clone();
            let uri2 = uri.clone();
            (
                rsx! {
                    DialogButton { label: "Cancel", onclick: move |_| cancel() }
                    DialogButton {
                        label: "Copy code",
                        primary: true,
                        autofocus: true,
                        disabled: left == 0,
                        title: if left == 0 { Some("The code expired. Get a new code.".to_string()) } else { None },
                        onclick: move |_| {
                            copy_to_clipboard(&code);
                            app.toast("Code copied");
                        },
                    }
                },
                rsx! {
                    div { class: "ax-acct-step",
                        {bar_view}
                        p { class: "ax-acct-text", "Go to {uri} and enter this code:" }
                        div {
                            class: "ax-acct-code ax-selectable",
                            tabindex: "0",
                            role: "textbox",
                            "aria-readonly": "true",
                            "aria-label": "Sign-in code {code2}",
                            onkeydown: move |e: KeyboardEvent| {
                                if keys::chord(&e) == "Ctrl+C" {
                                    e.prevent_default();
                                    e.stop_propagation();
                                    copy_to_clipboard(&code2);
                                    app.toast("Code copied");
                                }
                            },
                            "{info.user_code}"
                        }
                        p { class: "ax-acct-countdown", "aria-live": "off",
                            if left > 0 { "Code expires in {left / 60}:{left % 60:02}" } else { "The code expired." }
                        }
                        div { class: "ax-acct-links",
                            if left == 0 {
                                LinkButton { label: "Get a new code", onclick: move |_| start_device() }
                            }
                            LinkButton {
                                label: format!("Open {uri2}"),
                                onclick: move |_| {
                                    if let Err(e) = engine().open_url(&uri2) {
                                        banner_sig.set(Some(banner(&e)));
                                    }
                                },
                            }
                        }
                    }
                },
            )
        }
        Step::Manual => {
            let f = form();
            let app_pw = f.app_password_url();
            let provider_label = f.provider.label();
            footer_start = Some(rsx! {
                if testing() {
                    Busy { label: "Testing…" }
                } else {
                    LinkButton { label: "Test connection", onclick: move |_| test_connection() }
                }
            });
            (
                rsx! {
                    DialogButton { label: "Back", onclick: move |_| { banner_sig.set(None); step.set(Step::Email) } }
                    DialogButton {
                        label: "Sign in",
                        primary: true,
                        disabled: testing(),
                        title: if testing() { Some("Testing the connection…".to_string()) } else { None },
                        onclick: move |_| sign_in_imap(),
                    }
                },
                rsx! {
                    div { class: "ax-acct-step ax-acct-step-form",
                        {bar_view}
                        if let Some(url) = app_pw {
                            MessageBar {
                                intent: "info",
                                text: format!("{provider_label} needs an app password to sign in from other apps."),
                                actions: rsx! {
                                    button {
                                        r#type: "button",
                                        class: "ax-btn ax-small",
                                        onclick: move |_| {
                                            if let Err(e) = engine().open_url(url) {
                                                banner_sig.set(Some(banner(&e)));
                                            }
                                        },
                                        "Create an app password"
                                    }
                                },
                            }
                        }
                        ImapFormView { form, errors, onenter: move |_| sign_in_imap() }
                    }
                },
            )
        }
        Step::Password => (
            rsx! {
                DialogButton { label: "Cancel", onclick: move |_| cancel() }
                DialogButton { label: "Sign in", primary: true, onclick: move |_| sign_in_password() }
            },
            rsx! {
                div { class: "ax-acct-step",
                    {bar_view}
                    p { class: "ax-acct-text", "Enter the password for {email}." }
                    Field {
                        label: "Password",
                        value: password(),
                        input_type: "password",
                        autocomplete: "current-password",
                        autofocus: true,
                        error: password_error(),
                        hint: "Use an app password if your provider requires one",
                        oninput: move |v: String| { password.set(v); password_error.set(None); },
                        onenter: move |_| sign_in_password(),
                    }
                }
            },
        ),
        Step::Verifying(host) => (
            rsx! { DialogButton { label: "Cancel", onclick: move |_| cancel(), autofocus: true } },
            rsx! { div { class: "ax-acct-step", Busy { label: format!("Connecting to {host}…") } } },
        ),
        Step::Mismatch { flow, actual } => {
            let who = actual.clone();
            (
                rsx! {
                    DialogButton { label: "Try again", onclick: move |_| start_browser(true) }
                    DialogButton { label: format!("Add {actual}"), primary: true, autofocus: true, onclick: move |_| finish(flow, who.clone()) }
                },
                rsx! {
                    div { class: "ax-acct-step",
                        MessageBar { intent: "warning", text: format!("You signed in as {actual}. Add {actual} instead?") }
                    }
                },
            )
        }
        Step::Done { email: who, reauth: again } => (
            rsx! { DialogButton { label: "Done", primary: true, autofocus: true, onclick: move |_| on_close.call(()) } },
            rsx! {
                div { class: "ax-acct-step ax-acct-done",
                    Icon { name: "checkmark_circle_filled", size: 24, class: "ax-acct-done-icon" }
                    p { class: "ax-acct-text",
                        if again { "You're signed in again. We're syncing {who} now." } else { "{who} is ready. We're syncing your mail now." }
                    }
                }
            },
        ),
    };
    rsx! {
        AcctDialog { title, on_cancel: move |_| cancel(), actions, footer_start, {body} }
    }
}

#[component]
fn ProviderTile(
    #[props(into)] icon: String,
    #[props(into)] label: String,
    #[props(into)] sub: String,
    onclick: EventHandler<MouseEvent>,
    #[props(default)] autofocus: bool,
) -> Element {
    rsx! {
        button {
            r#type: "button",
            class: "ax-acct-provider",
            role: "listitem",
            onmounted: move |e| async move {
                if autofocus {
                    let _ = e.set_focus(true).await;
                }
            },
            onclick: move |e| onclick.call(e),
            Icon { name: icon.clone(), size: 20, class: "ax-acct-provider-icon" }
            span { class: "ax-acct-provider-text",
                span { class: "ax-acct-provider-label", "{label}" }
                span { class: "ax-acct-provider-sub", "{sub}" }
            }
            Icon { name: "chevron_right", size: 16, class: "ax-acct-provider-chevron" }
        }
    }
}
