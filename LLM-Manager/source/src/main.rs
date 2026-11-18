mod telemetry;
mod ui;

use std::collections::VecDeque;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use telemetry::Snapshot;
use ui::{Action, View};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const STATE_CONFIRMATION_WINDOW: Duration = Duration::from_secs(40);
const OFFLINE_PANEL_URL: &str = "http://127.0.0.1:8766/";
const OFFLINE_PANEL_LAUNCHER: &str = ".local/share/axentra/offline-panel/launch.sh";
const GPU_HISTORY_LENGTH: usize = ui::HISTORY;
/// A result stays on screen this long; errors stay until the next action.
const RESULT_LIFETIME: Duration = Duration::from_secs(6);

enum Event {
    Sample {
        started: Instant,
        snapshot: Snapshot,
    },
    RequestFinished(Result<(), String>),
    PanelOpenFailed(String),
}

struct Change {
    model: String,
    load: bool,
    accepted_at: Option<Instant>,
}

struct Manager {
    view: View,
    events_tx: Sender<Event>,
    events_rx: Receiver<Event>,
    refresh: SyncSender<()>,
    snapshot: Option<Snapshot>,
    pending: Option<Change>,
    message: Option<(String, bool, Instant)>,
    gpu_utilization: VecDeque<f32>,
}

impl Manager {
    fn new(ctx: &egui::Context, icon: &egui::IconData) -> Self {
        ui::configure(ctx);
        let (events_tx, events_rx) = mpsc::channel();
        let (refresh, refresh_rx) = mpsc::sync_channel(1);
        let sample_tx = events_tx.clone();
        let sampler_ctx = ctx.clone();
        thread::spawn(move || {
            loop {
                let started = Instant::now();
                let snapshot = telemetry::snapshot();
                if sample_tx.send(Event::Sample { started, snapshot }).is_err() {
                    break;
                }
                sampler_ctx.request_repaint();
                if matches!(
                    refresh_rx.recv_timeout(POLL_INTERVAL),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                ) {
                    break;
                }
            }
        });
        Self {
            view: View::new(ctx, icon),
            events_tx,
            events_rx,
            refresh,
            snapshot: None,
            pending: None,
            message: None,
            gpu_utilization: VecDeque::with_capacity(GPU_HISTORY_LENGTH),
        }
    }

    fn receive_events(&mut self) {
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                Event::Sample { started, snapshot } => self.receive_sample(started, snapshot),
                Event::RequestFinished(Ok(())) => {
                    if let Some(change) = &mut self.pending {
                        change.accepted_at = Some(Instant::now());
                    }
                    let _ = self.refresh.try_send(());
                }
                Event::RequestFinished(Err(error)) => {
                    self.pending = None;
                    self.message = Some((error, true, Instant::now()));
                    let _ = self.refresh.try_send(());
                }
                Event::PanelOpenFailed(error) => {
                    self.message = Some((error, true, Instant::now()));
                }
            }
        }
    }

    fn receive_sample(&mut self, started: Instant, snapshot: Snapshot) {
        if let Some(utilization) = snapshot
            .gpus
            .as_ref()
            .ok()
            .and_then(|gpus| gpus.first())
            .and_then(|gpu| gpu.utilization)
        {
            if self.gpu_utilization.len() == GPU_HISTORY_LENGTH {
                self.gpu_utilization.pop_front();
            }
            self.gpu_utilization.push_back(utilization);
        }

        if let Some(change) = &self.pending
            && let Some(accepted_at) = change.accepted_at
            && started >= accepted_at
        {
            let confirmed = snapshot.engine.as_ref().is_ok_and(|engine| {
                engine
                    .residents
                    .iter()
                    .any(|model| model.name == change.model)
                    == change.load
            });
            if confirmed {
                let state = if change.load { "loaded" } else { "unloaded" };
                self.message = Some((format!("{} {state}.", change.model), false, Instant::now()));
                self.pending = None;
            } else if accepted_at.elapsed() >= STATE_CONFIRMATION_WINDOW {
                let verb = if change.load { "load" } else { "unload" };
                self.message = Some((
                    format!("Ollama has yet to confirm the {verb} for {}.", change.model),
                    true,
                    Instant::now(),
                ));
                self.pending = None;
            }
        }
        self.snapshot = Some(snapshot);
    }

    fn handle(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Refresh => {
                let _ = self.refresh.try_send(());
            }
            Action::OpenPanel => {
                let tx = self.events_tx.clone();
                let ctx = ctx.clone();
                thread::spawn(move || {
                    // The Axentra launcher also starts the panel's server when it is down.
                    let launcher = std::env::var_os("HOME")
                        .map(std::path::PathBuf::from)
                        .map(|home| home.join(OFFLINE_PANEL_LAUNCHER))
                        .filter(|path| path.is_file());
                    let mut command = match launcher {
                        Some(path) => Command::new(path),
                        None => {
                            let mut command = Command::new("xdg-open");
                            command.arg(OFFLINE_PANEL_URL);
                            command
                        }
                    };
                    let opened = command
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .is_ok_and(|status| status.success());
                    if !opened {
                        let _ = tx.send(Event::PanelOpenFailed(format!(
                            "Open {OFFLINE_PANEL_URL} in your browser to use Offline Panel."
                        )));
                        ctx.request_repaint();
                    }
                });
            }
            Action::SetResident { model, load } => {
                if self.pending.is_some() {
                    return;
                }
                self.message = None;
                self.pending = Some(Change {
                    model: model.clone(),
                    load,
                    accepted_at: None,
                });
                let tx = self.events_tx.clone();
                let ctx = ctx.clone();
                thread::spawn(move || {
                    let result = telemetry::set_resident(&model, load);
                    let _ = tx.send(Event::RequestFinished(result));
                    ctx.request_repaint();
                });
            }
        }
    }
}

impl eframe::App for Manager {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive_events();
        if let Some((_, error, shown)) = &self.message
            && !*error
        {
            match RESULT_LIFETIME.checked_sub(shown.elapsed()) {
                Some(left) => ui.ctx().request_repaint_after(left),
                None => self.message = None,
            }
        }
        let pending = self
            .pending
            .as_ref()
            .map(|change| (change.model.as_str(), change.load));
        let message = self
            .message
            .as_ref()
            .map(|(text, error, _)| (text.as_str(), *error));
        let action = self.view.show(
            ui,
            self.snapshot.as_ref(),
            pending,
            message,
            self.gpu_utilization.make_contiguous(),
        );
        if let Some(action) = action {
            self.handle(action, ui.ctx());
        }
    }
}

fn main() -> eframe::Result {
    // One decode serves the native window icon and the header texture.
    let icon = Arc::new(
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/llm-manager-256.png"))
            .expect("the bundled LLM Manager icon should be a valid PNG"),
    );
    eframe::run_native(
        "LLM Manager",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                // Matches axentra-llm-manager.desktop so the taskbar uses its launcher icon.
                .with_app_id("axentra-llm-manager")
                // The app draws its own title strip and window controls.
                .with_decorations(false)
                .with_icon(Arc::clone(&icon))
                .with_inner_size([1030.0, 830.0])
                .with_min_inner_size([760.0, 550.0]),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |creation| Ok(Box::new(Manager::new(&creation.egui_ctx, &icon)))),
    )
}
