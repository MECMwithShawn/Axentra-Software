//! Compose / reply / forward: the form shown in the reading pane (or popped
//! out), its ribbon tabs, drafts and sending (mail spec section 6).
//!
//! * [`ComposePane`] renders the form for a `ComposeRequest`; the Mail
//!   module shows it while `app.compose` is set.
//! * [`ribbon_tabs`] gives the Message / Insert / Format text / Options tabs
//!   for the Mail ribbon. Their command ids start with `compose.`; forward
//!   picks with `app.compose_command(id)`.
//! * [`ComposeBackground`] (mount once near the root) sends scheduled
//!   drafts on time and finishes "Undo send" delays a quit interrupted.
//! * [`forward_as_attachment`] starts a forward with the message attached
//!   as an `.eml` file (for the message context menu).

mod actions;
pub mod address;
mod attachments;
mod commands;
mod dialogs;
mod draw;
mod editor;
mod emoji;
mod flyouts;
mod footer;
mod format;
mod header;
mod mention;
mod pane;
mod recipients;
mod ribbon;
mod schedule;
pub mod send_queue;
mod state;

pub use actions::forward_as_attachment;
pub use pane::ComposePane;
pub use ribbon::ribbon_tabs;
pub use send_queue::ComposeBackground;
