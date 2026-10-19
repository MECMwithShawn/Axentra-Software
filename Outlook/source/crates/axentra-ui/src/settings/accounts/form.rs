//! The manual IMAP / SMTP form (§2.2 step 3e).

use std::collections::HashMap;

use axentra_sync::config::{is_valid_email, preset};
use axentra_sync::{Detection, ImapSetup, Provider, Secret, Security, ServerConfig};
use dioxus::prelude::*;

use super::widgets::{Expander, Field};
use crate::kit::{Checkbox, Select};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImapForm {
    pub provider: Provider,
    pub display_name: String,
    pub email: String,
    pub password: String,
    pub imap_host: String,
    pub imap_port: String,
    pub imap_security: Security,
    /// The user typed a port: don't change it when security changes.
    pub imap_port_custom: bool,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_security: Security,
    pub smtp_port_custom: bool,
    pub same_sign_in: bool,
    pub advanced: bool,
    pub imap_username: String,
    pub smtp_username: String,
    pub smtp_password: String,
}

/// Field keys used for inline errors.
pub type Errors = HashMap<&'static str, String>;

impl ImapForm {
    pub fn new(email: &str, display_name: &str, detection: Option<&Detection>) -> Self {
        let (provider, imap, smtp) = match detection {
            Some(d) => (d.provider, d.imap.clone(), d.smtp.clone()),
            None => {
                let domain = axentra_sync::config::domain_of(email).unwrap_or_default();
                let (i, s) = axentra_sync::config::guess_servers(&domain);
                (Provider::Imap, i, s)
            }
        };
        let mut f = ImapForm {
            provider,
            display_name: display_name.to_string(),
            email: email.trim().to_string(),
            imap_host: imap.host.clone(),
            imap_port: imap.port.to_string(),
            imap_security: imap.security,
            smtp_host: smtp.host.clone(),
            smtp_port: smtp.port.to_string(),
            smtp_security: smtp.security,
            same_sign_in: true,
            imap_username: imap.username.clone(),
            smtp_username: smtp.username.clone(),
            ..Default::default()
        };
        if let Some(p) = preset(provider)
            && p.imap_local_part
        {
            f.imap_username = email.split('@').next().unwrap_or_default().to_string();
        }
        f.advanced = !f.imap_username.is_empty() || !f.smtp_username.is_empty();
        f
    }

    /// A provider picked from the "Other account" list.
    pub fn for_provider(provider: Provider, email: &str, display_name: &str) -> Self {
        let d = preset(provider).map(|p| Detection { provider, imap: p.imap, smtp: p.smtp, source: "preset" });
        let mut f = Self::new(email, display_name, d.as_ref());
        if d.is_none() {
            f.provider = Provider::Imap;
        }
        f
    }

    pub fn app_password_url(&self) -> Option<&'static str> {
        preset(self.provider).and_then(|p| p.app_password_url)
    }

    pub fn validate(&self) -> Errors {
        let mut e = Errors::new();
        if !is_valid_email(&self.email) {
            e.insert("email", "Enter an email address in the format someone@example.com.".into());
        }
        if self.password.is_empty() {
            e.insert("password", "Enter your password.".into());
        }
        let port_ok = |p: &str| p.trim().parse::<u16>().is_ok_and(|n| n > 0);
        if self.imap_host.trim().is_empty() {
            e.insert("imap_host", "Enter the incoming mail server.".into());
        }
        if !port_ok(&self.imap_port) {
            e.insert("imap_port", "Enter a port from 1 to 65535.".into());
        }
        if self.smtp_host.trim().is_empty() {
            e.insert("smtp_host", "Enter the outgoing mail server.".into());
        }
        if !port_ok(&self.smtp_port) {
            e.insert("smtp_port", "Enter a port from 1 to 65535.".into());
        }
        if !self.same_sign_in && self.smtp_password.is_empty() {
            e.insert("smtp_password", "Enter the outgoing server password.".into());
        }
        e
    }

    pub fn setup(&self) -> ImapSetup {
        ImapSetup {
            display_name: self.display_name.trim().to_string(),
            email: self.email.trim().to_string(),
            password: Secret::new(self.password.clone()),
            provider: self.provider,
            imap: ServerConfig {
                host: self.imap_host.trim().to_string(),
                port: self.imap_port.trim().parse().unwrap_or(0),
                security: self.imap_security,
                username: self.imap_username.trim().to_string(),
            },
            smtp: ServerConfig {
                host: self.smtp_host.trim().to_string(),
                port: self.smtp_port.trim().parse().unwrap_or(0),
                security: self.smtp_security,
                username: self.smtp_username.trim().to_string(),
            },
            smtp_same_auth: self.same_sign_in,
            smtp_password: (!self.same_sign_in).then(|| Secret::new(self.smtp_password.clone())),
        }
    }
}

fn security_options() -> Vec<(String, String)> {
    vec![("tls".into(), Security::Tls.label().into()), ("starttls".into(), Security::StartTls.label().into())]
}

fn security_key(s: Security) -> &'static str {
    match s {
        Security::Tls => "tls",
        Security::StartTls => "starttls",
    }
}

fn parse_security(v: &str) -> Security {
    if v == "starttls" { Security::StartTls } else { Security::Tls }
}

#[component]
pub fn ImapFormView(form: Signal<ImapForm>, errors: Signal<Errors>, onenter: EventHandler<()>) -> Element {
    let f = form.read().clone();
    let err = errors.read().clone();
    let e = |k: &str| err.get(k).cloned();
    let pw_label = if f.app_password_url().is_some() { "App password" } else { "Password" };
    let mut set = move |mutate: &dyn Fn(&mut ImapForm), key: &'static str| {
        form.with_mut(|f| mutate(f));
        if errors.read().contains_key(key) {
            errors.write().remove(key);
        }
    };
    rsx! {
        div { class: "ax-acct-form",
            Field {
                label: "Display name",
                value: f.display_name.clone(),
                placeholder: "Name that people see",
                autocomplete: "name",
                oninput: move |v: String| set(&|f| f.display_name = v.clone(), "display_name"),
                onenter,
            }
            Field {
                label: "Email address",
                value: f.email.clone(),
                placeholder: "someone@example.com",
                input_type: "email",
                autocomplete: "email",
                error: e("email"),
                oninput: move |v: String| set(&|f| f.email = v.clone(), "email"),
                onenter,
            }
            Field {
                label: pw_label,
                value: f.password.clone(),
                input_type: "password",
                autocomplete: "current-password",
                autofocus: true,
                error: e("password"),
                hint: "Use an app password if your provider requires one",
                oninput: move |v: String| set(&|f| f.password = v.clone(), "password"),
                onenter,
            }
            div { class: "ax-acct-server-row",
                Field {
                    label: "Incoming mail (IMAP) server",
                    value: f.imap_host.clone(),
                    placeholder: "imap.example.com",
                    class: "ax-acct-host",
                    error: e("imap_host"),
                    oninput: move |v: String| set(&|f| f.imap_host = v.clone(), "imap_host"),
                    onenter,
                }
                Field {
                    label: "Port",
                    value: f.imap_port.clone(),
                    class: "ax-acct-port",
                    input_mode: "numeric",
                    error: e("imap_port"),
                    oninput: move |v: String| set(&|f| { f.imap_port = v.clone(); f.imap_port_custom = true; }, "imap_port"),
                    onenter,
                }
                Select {
                    label: "Security",
                    class: "ax-acct-security",
                    value: security_key(f.imap_security),
                    options: security_options(),
                    onchange: move |v: String| form.with_mut(|f| {
                        f.imap_security = parse_security(&v);
                        if !f.imap_port_custom {
                            f.imap_port = f.imap_security.default_imap_port().to_string();
                        }
                    }),
                }
            }
            div { class: "ax-acct-server-row",
                Field {
                    label: "Outgoing mail (SMTP) server",
                    value: f.smtp_host.clone(),
                    placeholder: "smtp.example.com",
                    class: "ax-acct-host",
                    error: e("smtp_host"),
                    oninput: move |v: String| set(&|f| f.smtp_host = v.clone(), "smtp_host"),
                    onenter,
                }
                Field {
                    label: "Port",
                    value: f.smtp_port.clone(),
                    class: "ax-acct-port",
                    input_mode: "numeric",
                    error: e("smtp_port"),
                    oninput: move |v: String| set(&|f| { f.smtp_port = v.clone(); f.smtp_port_custom = true; }, "smtp_port"),
                    onenter,
                }
                Select {
                    label: "Security",
                    class: "ax-acct-security",
                    value: security_key(f.smtp_security),
                    options: security_options(),
                    onchange: move |v: String| form.with_mut(|f| {
                        f.smtp_security = parse_security(&v);
                        if !f.smtp_port_custom {
                            f.smtp_port = f.smtp_security.default_smtp_port().to_string();
                        }
                    }),
                }
            }
            Checkbox {
                checked: f.same_sign_in,
                label: "Outgoing server uses the same sign-in",
                onchange: move |v: bool| form.with_mut(|f| {
                    f.same_sign_in = v;
                    if !v {
                        f.advanced = true;
                    }
                }),
            }
            Expander { label: "Advanced", open: f.advanced, ontoggle: move |_| form.with_mut(|f| f.advanced = !f.advanced),
                Field {
                    label: "IMAP username",
                    value: f.imap_username.clone(),
                    placeholder: "Same as email address",
                    oninput: move |v: String| set(&|f| f.imap_username = v.clone(), "imap_username"),
                    onenter,
                }
                Field {
                    label: "SMTP username",
                    value: f.smtp_username.clone(),
                    placeholder: "Same as IMAP username",
                    oninput: move |v: String| set(&|f| f.smtp_username = v.clone(), "smtp_username"),
                    onenter,
                }
                if !f.same_sign_in {
                    Field {
                        label: "SMTP password",
                        value: f.smtp_password.clone(),
                        input_type: "password",
                        error: e("smtp_password"),
                        oninput: move |v: String| set(&|f| f.smtp_password = v.clone(), "smtp_password"),
                        onenter,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_validation_and_setup() {
        let f = ImapForm::for_provider(Provider::ICloud, "emily@icloud.com", "Emily");
        assert_eq!((f.imap_host.as_str(), f.imap_port.as_str()), ("imap.mail.me.com", "993"));
        assert_eq!(f.imap_username, "emily", "iCloud signs in with the name part");
        assert_eq!(f.smtp_security, Security::StartTls);
        assert!(f.app_password_url().is_some());
        let errs = f.validate();
        assert!(errs.contains_key("password") && !errs.contains_key("email"));
        let mut g = ImapForm::new("ana@example.org", "", None);
        assert_eq!(g.imap_host, "imap.example.org");
        g.password = "pw".into();
        g.imap_port = "99999".into();
        assert!(g.validate().contains_key("imap_port"));
        g.imap_port = "993".into();
        assert!(g.validate().is_empty());
        let s = g.setup();
        assert_eq!((s.imap.port, s.smtp.port, s.smtp_same_auth), (993, 465, true));
    }
}
