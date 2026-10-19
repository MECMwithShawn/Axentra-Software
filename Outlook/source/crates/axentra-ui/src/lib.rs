//! Axentra Outlook user interface (Dioxus). Rendered by `axentra-desktop`
//! (native window) and `axentra-preview` (liveview in a browser).

pub mod calendar;
pub mod fmt;
pub mod icons;
pub mod kit;
pub mod mail;
pub mod people;
pub mod settings;
pub mod shell;
pub mod state;
pub mod tasks;

use dioxus::prelude::*;

pub use state::init;

/// Product name shown in the header and window title. Microsoft owns the
/// "Outlook" trademark; change this before distributing the app.
pub const APP_NAME: &str = "Axentra Outlook";

/// All stylesheets, in cascade order.
pub const CSS: &str = concat!(
    include_str!("../styles/tokens.css"),
    include_str!("../styles/base.css"),
    include_str!("../styles/kit.css"),
    include_str!("../styles/shell.css"),
    include_str!("../styles/mail.css"),
    include_str!("../styles/compose.css"),
    include_str!("../styles/calendar.css"),
    include_str!("../styles/people.css"),
    include_str!("../styles/tasks.css"),
    include_str!("../styles/settings.css"),
    include_str!("../styles/accounts.css"),
);

/// Root component.
#[allow(non_snake_case)]
pub fn App() -> Element {
    state::provide_app_state();
    let persona = use_hook(kit::persona_css);
    rsx! {
        document::Title { "{APP_NAME}" }
        style { {CSS} }
        style { {persona} }
        shell::Shell {}
    }
}
