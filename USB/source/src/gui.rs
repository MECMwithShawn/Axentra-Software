use axentra_usb::{
    device::{self, Device},
    image::{self, Inspection},
    operation::{Action, Request},
};
use eframe::egui::{self, Align, Color32, Layout, RichText, Stroke, Vec2};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

const BG: Color32 = Color32::from_rgb(11, 16, 24);
const PANEL: Color32 = Color32::from_rgb(20, 28, 40);
const FIELD: Color32 = Color32::from_rgb(28, 39, 55);
const LINE: Color32 = Color32::from_rgb(41, 55, 73);
const TEXT: Color32 = Color32::from_rgb(236, 242, 250);
const MUTED: Color32 = Color32::from_rgb(146, 164, 186);
const GREEN: Color32 = Color32::from_rgb(163, 237, 183);
const AMBER: Color32 = Color32::from_rgb(242, 198, 123);
const RED: Color32 = Color32::from_rgb(242, 149, 156);
fn gb(n: u64) -> String {
    format!("{:.2} GB", n as f64 / 1e9)
}
fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1., LINE))
        .corner_radius(12)
        .inner_margin(20)
}
fn btn(label: &str) -> egui::Button<'_> {
    egui::Button::new(label)
        .min_size(Vec2::new(0., 38.))
        .corner_radius(7)
}
fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(BG).strong())
        .min_size(Vec2::new(0., 38.))
        .corner_radius(7)
        .fill(GREEN)
}
fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(GREEN).size(11.).strong());
}
fn muted(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).color(MUTED).size(13.));
}

enum Event {
    Devices(Result<Vec<Device>, String>),
    Image(Box<Result<Inspection, String>>),
    Progress(String, u64, u64),
    Worker(Result<(), String>),
}
struct App {
    logo: egui::TextureHandle,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    devices: Vec<Device>,
    selected: Option<usize>,
    image: Option<Inspection>,
    source_name: String,
    mode: Action,
    expected_hash: String,
    busy: bool,
    scanning: bool,
    cancel: Arc<AtomicBool>,
    phase: String,
    progress: f32,
    error: bool,
    report: Option<Value>,
    write_verified: bool,
    job: Option<tempfile::TempDir>,
    event_offset: usize,
    worker_finished: bool,
    confirm: bool,
    confirm_text: String,
    help: bool,
    details: bool,
    verify_prompt: bool,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        let mut style = (*cc.egui_ctx.global_style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = PANEL;
        style.visuals.extreme_bg_color = BG;
        style.visuals.faint_bg_color = FIELD;
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.widgets.inactive.bg_fill = FIELD;
        style.visuals.widgets.inactive.weak_bg_fill = FIELD;
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1., LINE);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 63, 78);
        style.visuals.selection.bg_fill = Color32::from_rgb(48, 81, 63);
        style.spacing.item_spacing = Vec2::new(10., 9.);
        style.spacing.button_padding = Vec2::new(16., 9.);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.));
        cc.egui_ctx.set_global_style(style);
        let (tx, rx) = mpsc::channel();
        let icon =
            eframe::icon_data::from_png_bytes(include_bytes!("../assets/axentra-usb-128.png"))
                .expect("bundled Axentra USB logo");
        let logo = cc.egui_ctx.load_texture(
            "axentra-usb-logo",
            egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        let mut a = Self {
            logo,
            tx,
            rx,
            devices: vec![],
            selected: None,
            image: None,
            source_name: "Select an ISO or disk image".into(),
            mode: Action::Compatible,
            expected_hash: String::new(),
            busy: false,
            scanning: false,
            cancel: Arc::new(AtomicBool::new(false)),
            phase: "Ready when you are.".into(),
            progress: 0.,
            error: false,
            report: None,
            write_verified: false,
            job: None,
            event_offset: 0,
            worker_finished: false,
            confirm: false,
            confirm_text: String::new(),
            help: false,
            details: false,
            verify_prompt: false,
        };
        a.refresh(cc.egui_ctx.clone());
        if let Some(p) = initial {
            a.analyze(p, cc.egui_ctx.clone());
        }
        a
    }
    fn refresh(&mut self, ctx: egui::Context) {
        if self.busy || self.scanning {
            return;
        }
        self.scanning = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::Devices(device::list().map_err(|e| format!("{e:#}"))));
            ctx.request_repaint();
        });
    }
    fn analyze(&mut self, path: PathBuf, ctx: egui::Context) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.image = None;
        self.report = None;
        self.write_verified = false;
        self.error = false;
        self.job = None;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.source_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let tx = self.tx.clone();
        let cancel = self.cancel.clone();
        std::thread::spawn(move || {
            let mut last = std::time::Instant::now();
            let result = image::inspect(
                &path,
                &mut |s, n, total| {
                    if last.elapsed().as_millis() > 100 || n == total {
                        last = std::time::Instant::now();
                        let _ = tx.send(Event::Progress(s.into(), n, total));
                        ctx.request_repaint();
                    }
                },
                &|| cancel.load(Ordering::Relaxed),
            );
            let _ = tx.send(Event::Image(Box::new(result.map_err(|e| format!("{e:#}")))));
            ctx.request_repaint();
        });
    }
    fn selected_device(&self) -> Option<&Device> {
        self.selected.and_then(|i| self.devices.get(i))
    }
    fn hash_ok(&self) -> bool {
        self.expected_hash.trim().is_empty()
            || self
                .image
                .as_ref()
                .is_some_and(|i| i.sha256.eq_ignore_ascii_case(self.expected_hash.trim()))
    }
    fn ready(&self) -> bool {
        !self.busy
            && self.hash_ok()
            && self.image.is_some()
            && self.selected_device().is_some_and(|d| {
                d.blocked().is_none() && self.image.as_ref().unwrap().size <= d.size
            })
    }
    fn can_write(&self) -> bool {
        if !self.ready() {
            return false;
        }
        let image = self.image.as_ref().unwrap();
        let d = self.selected_device().unwrap();
        if self.mode == Action::Compatible {
            image.compatible
                && d.size >= image.content_bytes + 1024 * 1024 * 1024 + 256 * 1024 * 1024
        } else {
            d.size >= image.size.div_ceil(d.sector) * d.sector
        }
    }
    fn fail(&mut self, message: String) {
        self.phase = message;
        self.error = true;
        self.busy = false;
    }
    fn start_job(&mut self, action: Action, ctx: egui::Context) {
        if !self.ready() {
            return;
        }
        let result = (|| -> anyhow::Result<_> {
            let job = tempfile::Builder::new().prefix("axentra-job-").tempdir()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(job.path(), std::fs::Permissions::from_mode(0o700))?;
            }
            let image = self.image.as_ref().unwrap();
            let r = Request {
                action,
                device: self.selected_device().unwrap().clone(),
                image_path: image.path.clone(),
                image_sha256: image.sha256.clone(),
                confirmation: self.confirm_text.clone(),
            };
            let path = job.path().join("request.json");
            std::fs::write(&path, serde_json::to_vec(&r)?)?;
            Ok((job, path))
        })();
        match result {
            Ok((job, path)) => {
                self.job = Some(job);
                self.busy = true;
                self.error = false;
                self.progress = 0.;
                self.event_offset = 0;
                self.worker_finished = false;
                self.report = None;
                self.write_verified = false;
                self.phase = "Waiting for device access…".into();
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(Event::Worker(
                        device::run_elevated(&path).map_err(|e| format!("{e:#}")),
                    ));
                    ctx.request_repaint();
                });
            }
            Err(e) => self.fail(format!("{e:#}")),
        }
    }
    fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(job) = &self.job {
            let _ = std::fs::write(job.path().join("cancel"), b"stop");
        }
        self.phase =
            "Stopping at the next safe boundary. Interrupted media must be recreated.".into();
    }
    fn read_events(&mut self) {
        let Some(job) = &self.job else {
            return;
        };
        let Ok(data) = std::fs::read(job.path().join("events.jsonl")) else {
            return;
        };
        if data.len() < self.event_offset {
            return;
        }
        let mut offset = self.event_offset;
        while let Some(end) = data[offset..].iter().position(|b| *b == b'\n') {
            let line = &data[offset..offset + end];
            offset += end + 1;
            if let Ok(v) = serde_json::from_slice::<Value>(line) {
                match v["type"].as_str().unwrap_or("") {
                    "progress" => {
                        self.phase = v["phase"].as_str().unwrap_or("").into();
                        let total = v["total"].as_u64().unwrap_or(0);
                        self.progress = if total > 0 {
                            v["done"].as_u64().unwrap_or(0) as f32 / total as f32
                        } else {
                            0.
                        };
                    }
                    "complete" => {
                        self.worker_finished = true;
                        self.write_verified = true;
                        self.progress = 1.;
                        self.report = Some(v["report"].clone());
                        self.phase="Verification passed. Eject safely, then test booting in the firmware menu.".into();
                    }
                    "error" => {
                        self.worker_finished = true;
                        self.error = true;
                        self.phase = v["message"]
                            .as_str()
                            .unwrap_or("Operation incomplete")
                            .into();
                        self.report = Some(
                            json!({"operation":"incomplete","message":self.phase,"image":self.image,"device":self.selected_device()}),
                        );
                    }
                    _ => {}
                }
            }
        }
        self.event_offset = offset;
    }
    fn receive(&mut self) {
        self.read_events();
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Devices(result) => {
                    self.scanning = false;
                    match result {
                        Ok(v) => {
                            self.devices = v
                                .into_iter()
                                .filter(|d| d.bus.eq_ignore_ascii_case("usb") && !d.system)
                                .collect();
                            self.selected = None;
                        }
                        Err(e) => {
                            self.error = true;
                            self.phase = format!("Device scan unavailable: {e}");
                        }
                    }
                }
                Event::Image(result) => {
                    self.busy = false;
                    match *result {
                        Ok(i) => {
                            self.image = Some(i);
                            self.phase = "Image inspected. Select the USB you want to use.".into();
                            self.progress = 0.;
                        }
                        Err(e) => self.fail(e),
                    }
                }
                Event::Progress(p, n, t) => {
                    self.phase = p;
                    self.progress = if t > 0 { n as f32 / t as f32 } else { 0. };
                }
                Event::Worker(result) => {
                    self.read_events();
                    self.busy = false;
                    if !self.worker_finished {
                        self.fail(result.err().unwrap_or_else(|| {
                            "Device worker stopped without a completion report.".into()
                        }));
                    }
                }
            }
        }
    }
    fn export(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name("axentra-usb-report.json")
            .add_filter("JSON report", &["json"])
            .save_file()
        {
            let data=self.report.clone().unwrap_or_else(||json!({"implementation":"Rust","image":self.image,"device":self.selected_device(),"write":"Not performed","firmware_boot":"Not performed"}));
            match serde_json::to_vec_pretty(&data)
                .map_err(anyhow::Error::from)
                .and_then(|b| std::fs::write(path, b).map_err(anyhow::Error::from))
            {
                Ok(()) => self.phase = "Report saved locally.".into(),
                Err(e) => self.fail(e.to_string()),
            }
        }
    }
    fn source_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card().show(ui, |ui| {
            ui.horizontal(|ui| {
                caption(ui, "01   SOURCE IMAGE");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(!self.busy, btn("Choose image")).clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Boot images", &["iso", "img"])
                            .pick_file()
                    {
                        self.analyze(path, ctx.clone());
                    }
                });
            });
            ui.add_space(8.);
            ui.label(RichText::new(&self.source_name).size(19.).strong());
            muted(
                ui,
                if let Some(i) = &self.image {
                    format!("{}  ·  {}", gb(i.size), i.title)
                } else {
                    "Uncompressed .iso and .img files".into()
                },
            );
            ui.add_space(10.);
            muted(ui, "Publisher SHA-256 · optional authenticity check");
            ui.add_enabled(
                !self.busy,
                egui::TextEdit::singleline(&mut self.expected_hash)
                    .hint_text("Paste the publisher's checksum")
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
            if self.image.is_some() && !self.hash_ok() {
                ui.colored_label(RED, "Checksum mismatch. Creation is blocked.");
            }
        });
    }
    fn target_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card().show(ui, |ui| {
            ui.horizontal(|ui| {
                caption(ui, "02   TARGET USB");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !self.busy && !self.scanning,
                            btn(if self.scanning {
                                "Scanning…"
                            } else {
                                "Refresh"
                            }),
                        )
                        .clicked()
                    {
                        self.refresh(ctx.clone());
                    }
                });
            });
            ui.add_space(7.);
            let selected = self
                .selected_device()
                .map(|d| format!("{}  ·  {}", d.model, gb(d.size)))
                .unwrap_or_else(|| {
                    if self.devices.is_empty() {
                        "No USB drives found".into()
                    } else {
                        "Select a USB drive".into()
                    }
                });
            ui.add_enabled_ui(!self.busy, |ui| {
                egui::ComboBox::from_id_salt("usb")
                    .width(ui.available_width())
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (index, d) in self.devices.iter().enumerate() {
                            let response = ui.selectable_value(
                                &mut self.selected,
                                Some(index),
                                format!("{}  ·  {}  ·  {}", d.model, gb(d.size), d.serial),
                            );
                            if response.changed() {
                                self.write_verified = false;
                                self.report = None;
                            }
                        }
                    });
            });
            if let Some(d) = self.selected_device() {
                muted(ui, format!("{}  ·  Serial {}", d.path, d.serial));
                if let Some(reason) = d.blocked() {
                    ui.colored_label(AMBER, reason);
                } else {
                    muted(ui, "Identity is checked again before any write.");
                }
            } else {
                muted(
                    ui,
                    "Internal and system disks are excluded. Choose your target explicitly.",
                );
            }
        });
    }
    fn method_card(&mut self, ui: &mut egui::Ui) {
        card().show(ui, |ui| {
            caption(ui, "03   WRITE METHOD");
            ui.add_space(10.);
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.radio_value(
                    &mut self.mode,
                    Action::Compatible,
                    RichText::new("UEFI compatibility").strong().size(16.),
                );
                ui.indent("compatible-note", |ui| {
                    muted(ui, "GPT · 1 GiB FAT32 boot + NTFS live data");
                });
                ui.add_space(9.);
                ui.radio_value(
                    &mut self.mode,
                    Action::Raw,
                    RichText::new("Original image").strong().size(16.),
                );
                ui.indent("raw-note", |ui| {
                    muted(ui, "Preserve the image layout · verify every byte");
                });
            });
            ui.add_space(10.);
            if self.mode == Action::Compatible {
                if self.image.as_ref().is_some_and(|i| i.compatible) {
                    ui.colored_label(
                        GREEN,
                        "Zorin profile inspected. Hardware boot test still required.",
                    );
                } else {
                    ui.colored_label(
                        AMBER,
                        "A supported, inspected Zorin image is required for this profile.",
                    );
                }
            } else {
                muted(
                    ui,
                    "Windows may offer to format a raw Linux USB. Cancel that prompt.",
                );
            }
        });
    }
    fn readiness(&self, ui: &mut egui::Ui) {
        card().show(ui,|ui|{
            caption(ui,"BOOT READINESS");ui.add_space(12.);
            ui.label(RichText::new(if self.image.is_some(){"Image inspection complete."}else{"Know before you boot."}).size(23.).strong());
            muted(ui,"A completed write is only one part of a bootable drive.");ui.add_space(18.);
            let states=[
                ("Image inspected",self.image.is_some(),if self.image.is_some(){"Image read and fingerprinted"}else{"Waiting for image"}),
                ("EFI bootloader",self.image.as_ref().is_some_and(|i|i.efi_x64),"x64 removable-media loader"),
                ("Live filesystem",self.image.as_ref().is_some_and(|i|i.compatible),"Casper UUID + NTFS startup support"),
                ("USB write verified",self.write_verified,if self.write_verified{"Read-back verification passed"}else{"Not performed"}),
                ("Firmware boot test",false,"Not performed · test on the target PC")];
            for (label,passed,detail) in states {
                ui.horizontal_top(|ui|{ui.label(RichText::new(if passed{"✓"}else{"○"}).size(23.).color(if passed{GREEN}else{MUTED}));ui.vertical(|ui|{ui.label(RichText::new(label).size(16.).strong());muted(ui,detail);});});ui.add_space(13.);
            }
            ui.separator();ui.add_space(8.);
            muted(ui,if self.image.as_ref().is_some_and(|i|i.largest_size>u32::MAX as u64){"Large live filesystem detected. The compatibility profile keeps boot files on FAT32 and the complete live filesystem on NTFS."}else{"Already flashed a drive? Select its original image and USB, then choose Verify existing USB."});
        });
    }
    fn popups(&mut self, ctx: &egui::Context) {
        if self.confirm {
            let d = self.selected_device().cloned();
            if let Some(d) = d {
                egui::Modal::new("erase-confirm".into()).show(ctx, |ui| {
                    ui.set_width(480.);
                    ui.heading("Erase and create this USB?");
                    ui.colored_label(AMBER, "All existing data on this USB will be erased.");
                    ui.separator();
                    ui.label(RichText::new(format!("{} · {}", d.model, gb(d.size))).strong());
                    ui.monospace(&d.path);
                    ui.monospace(format!("Serial: {}", d.serial));
                    ui.label(format!("Image: {}", self.source_name));
                    ui.label(if self.mode == Action::Compatible {
                        "GPT / FAT32 + NTFS"
                    } else {
                        "Original-image write"
                    });
                    ui.add_space(10.);
                    ui.label(format!("Type {} to continue", d.token()));
                    ui.text_edit_singleline(&mut self.confirm_text);
                    ui.horizontal(|ui| {
                        if ui.add(btn("Cancel")).clicked() {
                            self.confirm = false;
                        }
                        if ui
                            .add_enabled(
                                self.confirm_text == d.token(),
                                primary("Erase and create"),
                            )
                            .clicked()
                        {
                            self.confirm = false;
                            self.start_job(self.mode.clone(), ctx.clone());
                        }
                    });
                });
            }
        }
        if self.verify_prompt {
            egui::Modal::new("verify-confirm".into()).show(ctx,|ui|{
            ui.set_width(450.);ui.heading("Verify an original-image flash");ui.label("Reads the USB and compares every image byte. No disk changes.");
            ui.label("A USB created with compatibility extraction has a different layout and will not match the raw ISO. That mode verifies its files during creation.");
            ui.horizontal(|ui| {
                if ui.add(btn("Cancel")).clicked() { self.verify_prompt = false; }
                else if ui.add(btn("Start verification")).clicked() {
                    self.verify_prompt = false;
                    self.start_job(Action::Verify, ctx.clone());
                }
            });
        });
        }
        egui::Window::new("Inspection details")
            .open(&mut self.details)
            .default_size([730., 550.])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if let Some(i) = &self.image {
                        ui.heading(&i.title);
                        for c in &i.checks {
                            ui.add_space(10.);
                            ui.colored_label(
                                if c.status == "pass" {
                                    GREEN
                                } else if c.status == "warning" {
                                    AMBER
                                } else {
                                    MUTED
                                },
                                format!("{}  ·  {}", c.status.to_uppercase(), c.label),
                            );
                            ui.label(&c.detail);
                        }
                        ui.add_space(16.);
                        ui.label("SHA-256");
                        ui.monospace(&i.sha256);
                        ui.label("Casper UUID");
                        ui.monospace(&i.casper_uuid);
                        ui.label(format!(
                            "{} files · {} live filesystem bytes",
                            i.file_count, i.largest_size
                        ));
                    } else {
                        ui.label("Choose an image to inspect its boot chain.");
                    }
                });
            });
        egui::Window::new("Boot help").open(&mut self.help).default_size([720.,560.]).show(ctx,|ui|{egui::ScrollArea::vertical().show(ui,|ui|{
            ui.heading("USB missing from the boot menu");ui.label("Fully shut down, use a direct USB port, and open the boot menu. On Alienware, press F12 during startup. A matching partition table alone is not complete verification.");
            ui.add_space(15.);ui.heading("Unable to find a live filesystem");ui.label("Firmware loading an EFI program does not establish that Linux can read its live files. This profile inspects the actual Casper scripts, NTFS tooling, FUSE support and live-media UUID, then copies and verifies every file.");
            ui.add_space(15.);ui.heading("Windows offers to format the drive");ui.label("Cancel that prompt for a raw Linux installer. Explorer visibility is not a bootability test.");
            ui.add_space(15.);ui.heading("FAT12 and Secure Boot");ui.label("UEFI permits FAT12, FAT16 and FAT32 removable media. FAT12 alone does not establish a Dell/Alienware incompatibility. Axentra preserves the source EFI binaries and does not change firmware settings. Actual boot and Secure Boot trust remain untested.");
            ui.add_space(15.);ui.heading("Preview limits");ui.label("The compatibility profile is gated on the Casper revision inspected in Zorin OS 18.1 Pro x64. Raw writing does not make every ISO USB-bootable. Windows installer extraction is not supported. Physical USB writes and booting on Alienware still require hardware validation. Test with a spare USB.");
            ui.add_space(12.);ui.hyperlink_to("UEFI removable-media specification","https://uefi.org/specs/UEFI/2.10/13_Protocols_Media_Access.html");ui.hyperlink_to("Zorin installation guide","https://help.zorin.com/docs/getting-started/install-zorin-os/");
        });});
    }
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive();
        if self.busy && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.phase = "Use Stop and wait for the operation to finish before closing.".into();
        }
        if self.busy || self.scanning {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        egui::Panel::top("brand")
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(28, 18)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.image((self.logo.id(), Vec2::splat(44.)));
                    ui.label(RichText::new("AXENTRA").size(21.).strong());
                    ui.label(RichText::new("USB").size(21.).color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        muted(ui, concat!("PREVIEW ", env!("CARGO_PKG_VERSION")));
                    });
                });
            });
        egui::Panel::bottom("actions")
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(28, 16)),
            )
            .show(ui, |ui| {
                ui.add(
                    egui::ProgressBar::new(self.progress)
                        .fill(GREEN)
                        .desired_height(5.),
                );
                ui.add_space(10.);
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width() - 440., 50.),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.colored_label(
                                if self.error {
                                    RED
                                } else if self.write_verified {
                                    GREEN
                                } else {
                                    MUTED
                                },
                                &self.phase,
                            );
                        },
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(self.busy, btn("Stop")).clicked() {
                            self.stop();
                        }
                        if ui
                            .add_enabled(self.can_write(), primary("Create USB  →"))
                            .clicked()
                        {
                            self.confirm_text.clear();
                            self.confirm = true;
                        }
                        if ui
                            .add_enabled(self.ready(), btn("Verify existing USB"))
                            .clicked()
                        {
                            self.verify_prompt = true;
                        }
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(28, 14)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Create a bootable USB").size(30.).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(btn("Boot help")).clicked() {
                            self.help = true;
                        }
                        if ui
                            .add_enabled(
                                self.image.is_some() || self.report.is_some(),
                                btn("Export report"),
                            )
                            .clicked()
                        {
                            self.export();
                        }
                    });
                });
                muted(
                    ui,
                    "Choose your image. Check the boot chain. Write with evidence.",
                );
                ui.add_space(18.);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let available = ui.available_width();
                        let left = (available - 22.) * 0.60;
                        let right = available - left - 22.;
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui_with_layout(
                                Vec2::new(left, 0.),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.set_min_width(left);
                                    self.source_card(ui, &ctx);
                                    ui.add_space(4.);
                                    self.target_card(ui, &ctx);
                                    ui.add_space(4.);
                                    self.method_card(ui);
                                },
                            );
                            ui.add_space(2.);
                            ui.allocate_ui_with_layout(
                                Vec2::new(right, 0.),
                                Layout::top_down(Align::Min),
                                |ui| {
                                    ui.set_min_width(right);
                                    self.readiness(ui);
                                    ui.add_space(8.);
                                    if ui.add(btn("View inspection details")).clicked() {
                                        self.details = true;
                                    }
                                },
                            );
                        });
                    });
            });
        self.popups(&ctx);
    }
}

pub fn launch(image: Option<PathBuf>) -> eframe::Result<()> {
    #[cfg(target_os = "windows")]
    unsafe {
        // Match the installer shortcuts so Windows groups this window under its own icon.
        let id: Vec<u16> = "Axentra.USB".encode_utf16().chain(Some(0)).collect();
        windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
    }
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/axentra-usb-256.png"))
        .expect("bundled Axentra USB window icon");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(icon)
            .with_app_id("axentra-usb")
            .with_inner_size([1160., 840.])
            .with_min_inner_size([1000., 700.]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Axentra USB",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, image)))),
    )
}
