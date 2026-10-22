//! Shared components. Props here are a contract used by every module: add
//! optional props freely, but don't rename or remove existing ones.
//!
//! Keyboard and focus behaviour that has to run in the page (roving focus,
//! focus trap, focus return, tooltips) lives in [`dom`]; components only
//! mark up their structure for it.

pub mod dom;
pub mod keys;
pub mod surface;

mod button;
mod dialog;
mod feedback;
mod form;
mod menu;
mod persona;

pub use button::*;
pub use dialog::*;
pub use feedback::*;
pub use form::*;
pub use menu::*;
pub use persona::*;
pub use surface::{SurfaceGrips, SurfaceSpec, use_surface};

use dioxus::prelude::*;

/// Run `fut` as a task of the component `scope` (an ancestor of the caller,
/// such as a form or module root): it outlives the menu or row that started
/// it, and is dropped with `scope`, so it never wakes up to state that has
/// gone. `alive` is a value owned by `scope`; when it has been dropped the
/// component is gone and nothing runs.
///
/// (`Runtime::spawn(scope, ..)` is not the same: in dioxus-core 0.7 it does
/// not tie the task to the scope, so the task outlives the component and
/// panics when it next touches the component's signals.)
pub fn spawn_in<T: 'static>(scope: ScopeId, alive: CopyValue<T>, fut: impl std::future::Future<Output = ()> + 'static) {
    if alive.try_peek().is_err() {
        return;
    }
    dioxus::core::Runtime::current().in_scope(scope, || {
        spawn(fut);
    });
}

/// Run `f` after `ms` milliseconds as a task of the calling component (the
/// one whose event handler or hook calls this). It is dropped with that
/// component, so `f` may read the component's own signals: it never runs
/// once they have gone. (A task spawned on an ancestor with [`spawn_in`]
/// outlives the caller, and touching the caller's signals there panics:
/// "Dropped(ValueDroppedError)" once the caller has been removed.)
pub fn after(ms: u64, f: impl FnOnce() + 'static) {
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        f();
    });
}

/// Convert a displayed shortcut ("Ctrl+Shift+R", "Ctrl+> or Ctrl+.") to the
/// `aria-keyshortcuts` form ("Control+Shift+R", space-separated alternatives).
pub fn keys_to_aria(display: &str) -> String {
    let key = |k: &str| match k {
        "Ctrl" => "Control".to_string(),
        "Esc" => "Escape".to_string(),
        other => other.to_string(),
    };
    display
        .split(" or ")
        .map(str::trim)
        .filter(|alt| !alt.is_empty() && !alt.contains(" then ") && !alt.contains(", "))
        .map(|alt| {
            // "Ctrl++" style chords end in a literal "+".
            let (body, plus) = alt.strip_suffix("++").map(|b| (b, true)).unwrap_or((alt, false));
            let mut parts: Vec<String> = body.split('+').map(key).collect();
            if plus {
                parts.push("+".into());
            }
            parts.join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aria_keyshortcuts_format() {
        assert_eq!(keys_to_aria("Ctrl+Shift+R"), "Control+Shift+R");
        assert_eq!(keys_to_aria("Ctrl+> or Ctrl+."), "Control+> Control+.");
        assert_eq!(keys_to_aria("Esc"), "Escape");
        assert_eq!(keys_to_aria("G then I"), "");
        assert_eq!(keys_to_aria("Delete"), "Delete");
    }

    /// A task run in a component's scope (a compose form's Send or pop-out)
    /// goes with that component: it is dropped when the component unmounts,
    /// so it can never wake up on signals that are gone (that was a crash).
    #[test]
    fn spawn_in_ties_the_task_to_the_scope() {
        use std::cell::Cell;
        thread_local! {
            static SHOW: Cell<bool> = const { Cell::new(true) };
            static DROPPED: Cell<u32> = const { Cell::new(0) };
        }
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                DROPPED.with(|d| d.set(d.get() + 1));
            }
        }
        #[allow(non_snake_case)]
        fn Owner() -> Element {
            let alive = use_hook(|| CopyValue::new(()));
            let scope = dioxus::core::current_scope_id();
            use_hook(move || {
                let guard = Guard;
                spawn_in(scope, alive, async move {
                    let _guard = guard;
                    std::future::pending::<()>().await;
                });
            });
            rsx! { div {} }
        }
        #[allow(non_snake_case)]
        fn Root() -> Element {
            rsx! {
                if SHOW.with(|s| s.get()) { Owner {} }
            }
        }
        let mut dom = VirtualDom::new(Root);
        dom.rebuild_in_place();
        assert_eq!(DROPPED.with(|d| d.get()), 0, "running while the owner is mounted");
        SHOW.with(|s| s.set(false));
        dom.mark_dirty(ScopeId::APP);
        dom.render_immediate_to_vec();
        assert_eq!(DROPPED.with(|d| d.get()), 1, "dropped with its owner");
    }

    /// `after` belongs to the component that calls it. To Do's "Add a task"
    /// (focus check) and the task title and note (autosave) read their own
    /// signals a moment later; run as the module's tasks, they outlived a
    /// switch to another list or task and ended the app (`Dropped`).
    #[test]
    fn after_goes_with_its_component() {
        use std::cell::Cell;
        thread_local! {
            static SHOW: Cell<bool> = const { Cell::new(true) };
            static RAN: Cell<u32> = const { Cell::new(0) };
        }
        #[allow(non_snake_case)]
        #[component]
        fn Bar(n: u32) -> Element {
            let own = use_signal(move || n);
            use_hook(move || {
                after(30, move || RAN.with(|r| r.set(r.get() + *own.peek())));
            });
            rsx! { div {} }
        }
        #[allow(non_snake_case)]
        fn Root() -> Element {
            rsx! {
                Bar { n: 1 }
                if SHOW.with(|s| s.get()) { Bar { n: 10 } }
            }
        }
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let _in_rt = rt.enter();
        let mut dom = VirtualDom::new(Root);
        dom.rebuild_in_place();
        // The second bar goes before its delay is up (another list opened).
        SHOW.with(|s| s.set(false));
        dom.mark_dirty(ScopeId::APP);
        dom.render_immediate_to_vec();
        rt.block_on(async {
            let _ = tokio::time::timeout(std::time::Duration::from_millis(300), async {
                loop {
                    dom.wait_for_work().await;
                    dom.render_immediate_to_vec();
                }
            })
            .await;
        });
        assert_eq!(RAN.with(|r| r.get()), 1, "the mounted bar's check ran; the removed one's never did");
    }
}
