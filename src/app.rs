use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Margin, RichText, Stroke};

use crate::config::{self, AppConfig};
use crate::device::{list_all_devices, list_smx_devices, HidEntry};
use crate::discord::{self, Discord, DiscordConfig, Presence, PRESETS};
use crate::keys;
use crate::protocol::*;
use crate::worker::{Command, Event, MessageLevel, Worker};

const BG: Color32 = Color32::from_rgb(15, 17, 22);
const HEADER_BG: Color32 = Color32::from_rgb(18, 21, 27);
const CARD_BG: Color32 = Color32::from_rgb(24, 27, 35);
const CARD_BG_ALT: Color32 = Color32::from_rgb(28, 32, 41);
const CARD_STROKE: Color32 = Color32::from_rgb(44, 50, 63);
const MUTED: Color32 = Color32::from_rgb(142, 151, 170);
const ACCENT: Color32 = Color32::from_rgb(86, 166, 255);
const GREEN: Color32 = Color32::from_rgb(74, 202, 132);
const AMBER: Color32 = Color32::from_rgb(240, 178, 74);
const RED: Color32 = Color32::from_rgb(240, 96, 96);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Dpi,
    Lighting,
    Buttons,
    Discord,
    Advanced,
}

impl Tab {
    const ALL: [(Tab, &'static str, &'static str); 6] = [
        (Tab::Overview, "Overview", "Battery and device status"),
        (Tab::Dpi, "DPI & polling", "Sensitivity and report rate"),
        (Tab::Lighting, "Lighting", "Effects and colours"),
        (Tab::Buttons, "Buttons", "Remap mouse buttons"),
        (Tab::Discord, "Discord", "Rich Presence and keybinds"),
        (Tab::Advanced, "Advanced", "Raw blocks and HID details"),
    ];
}

struct LogEntry {
    level: MessageLevel,
    text: String,
    at: Instant,
}

pub struct HowlerApp {
    worker: Worker,
    events: Receiver<Event>,
    config: AppConfig,
    status: DeviceStatus,
    status_at: Option<Instant>,
    bank: Bank,
    settings_block: Option<Vec<u8>>,
    buttons_block: Option<Vec<u8>>,
    draft: SettingsDraft,
    loaded_draft: SettingsDraft,
    buttons: [[u8; 4]; BUTTON_SLOTS],
    loaded_buttons: [[u8; 4]; BUTTON_SLOTS],
    tab: Tab,
    busy: bool,
    messages: Vec<LogEntry>,
    next_auto_refresh: Instant,
    show_all_slots: bool,
    capture: Option<usize>,
    editor: Option<ActionEditor>,
    discord: Discord,
    discord_form: DiscordConfig,
    discord_last_key: String,
    discord_slot: usize,
    hid_entries: Vec<HidEntry>,
    hid_scan_error: Option<String>,
    show_all_hid: bool,
    show_help: bool,
    connected_once: bool,
}

impl HowlerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);
        let config = config::load();
        let bank = Bank::from_config(&config.bank);
        let (worker, events) = Worker::spawn(cc.egui_ctx.clone());
        let discord = Discord::spawn();
        let draft = SettingsDraft::default();
        let mut app = Self {
            worker,
            events,
            discord_form: config.discord.clone(),
            config,
            status: DeviceStatus::default(),
            status_at: None,
            bank,
            settings_block: None,
            buttons_block: None,
            draft: draft.clone(),
            loaded_draft: draft,
            buttons: FACTORY_BUTTONS,
            loaded_buttons: FACTORY_BUTTONS,
            tab: Tab::Overview,
            busy: true,
            messages: Vec::new(),
            next_auto_refresh: Instant::now() + Duration::from_secs(30),
            show_all_slots: false,
            capture: None,
            editor: None,
            discord,
            discord_last_key: String::new(),
            discord_slot: 6,
            hid_entries: Vec::new(),
            hid_scan_error: None,
            show_all_hid: false,
            show_help: false,
            connected_once: false,
        };
        app.log(
            MessageLevel::Info,
            "Close the SMX Howler vendor app (OemDrv.exe) before applying settings — it keeps the vendor HID channel open.",
        );
        app.worker.send(Command::SetTarget(bank));
        app.refresh_presence(true);
        app
    }

    fn pump_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Busy(busy) => self.busy = busy,
                Event::Status(status) => {
                    self.status = *status;
                    self.status_at = Some(Instant::now());
                    if self.status.connected {
                        self.connected_once = true;
                    }
                    let secs = self.config.refresh_secs.clamp(10, 600);
                    self.next_auto_refresh = Instant::now() + Duration::from_secs(secs);
                    self.refresh_presence(false);
                }
                Event::Blocks {
                    bank,
                    settings,
                    buttons,
                } => {
                    self.bank = bank;
                    if let Some(draft) = SettingsDraft::from_block(&settings) {
                        self.draft = draft.clone();
                        self.loaded_draft = draft;
                    }
                    if let Some(slots) = parse_buttons(&buttons) {
                        self.buttons = slots;
                        self.loaded_buttons = slots;
                    }
                    self.settings_block = Some(settings);
                    self.buttons_block = Some(buttons);
                    self.connected_once = true;
                    self.refresh_presence(false);
                }
                Event::Message(level, text) => self.log(level, text),
            }
        }
    }

    fn log(&mut self, level: MessageLevel, text: impl Into<String>) {
        self.messages.push(LogEntry {
            level,
            text: text.into(),
            at: Instant::now(),
        });
        if self.messages.len() > 40 {
            self.messages.remove(0);
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        if self.config.auto_refresh && !self.busy && Instant::now() >= self.next_auto_refresh {
            let secs = self.config.refresh_secs.clamp(10, 600);
            self.next_auto_refresh = Instant::now() + Duration::from_secs(secs);
            self.worker.send(Command::Refresh);
        }
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if self.capture.is_some() {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
        if self.config.discord.enabled {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        // Keep relative timestamps and the automatic refresh countdown alive
        // even when the user is not interacting.
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn handle_capture(&mut self, ui: &mut egui::Ui) {
        let Some(slot) = self.capture else {
            return;
        };
        let mut result: Option<Result<(u8, u8), ()>> = None;
        ui.input(|input| {
            for event in &input.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                {
                    if *key == egui::Key::Escape {
                        result = Some(Err(()));
                        break;
                    }
                    if keys::is_modifier_only(*key) {
                        continue;
                    }
                    if let Some(usage) = keys::key_to_hid(*key) {
                        result = Some(Ok((keys::modifier_bits(*modifiers), usage)));
                        break;
                    }
                }
            }
        });
        if let Some(result) = result {
            self.capture = None;
            match result {
                Ok((modifiers, usage)) => {
                    let action = ButtonAction::Keyboard { modifiers, usage };
                    self.buttons[slot] = action.encode();
                    self.log(
                        MessageLevel::Info,
                        format!(
                            "{} set to {}. Press 'Apply assignments' to store it in the mouse.",
                            slot_label(slot),
                            action.label()
                        ),
                    );
                }
                Err(()) => {
                    self.log(MessageLevel::Info, "Key capture cancelled.");
                }
            }
        }
    }

    fn refresh_now(&mut self) {
        self.busy = true;
        self.worker.send(Command::Refresh);
    }

    fn reload(&mut self) {
        self.busy = true;
        self.worker.send(Command::Reload);
    }

    fn set_bank(&mut self, bank: Bank) {
        if bank == self.bank {
            return;
        }
        self.bank = bank;
        self.config.bank = bank.config_key().to_owned();
        self.save_config();
        self.busy = true;
        self.worker.send(Command::SetTarget(bank));
    }

    fn apply_settings(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.worker
            .send(Command::ApplySettings(Box::new(self.draft.clone())));
    }

    fn apply_buttons(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.worker
            .send(Command::ApplyButtons(Box::new(self.buttons)));
    }

    fn save_config(&mut self) {
        self.config.discord = self.discord_form.clone();
        if let Err(err) = config::save(&self.config) {
            self.log(MessageLevel::Warning, err);
        }
    }

    fn refresh_presence(&mut self, force: bool) {
        let cfg = self.config.discord.clone();
        if !cfg.enabled || cfg.client_id.trim().is_empty() {
            if force || !self.discord_last_key.is_empty() {
                self.discord_last_key.clear();
                self.discord.update(String::new(), None);
            }
            return;
        }

        let mut values: Vec<(&str, String)> = Vec::new();
        values.push((
            "battery",
            if cfg.show_battery {
                self.status.battery_text()
            } else {
                String::new()
            },
        ));
        values.push((
            "dpi",
            if cfg.show_dpi {
                self.draft.active_dpi().to_string()
            } else {
                String::new()
            },
        ));
        values.push((
            "polling",
            if cfg.show_dpi {
                polling_label(self.draft.polling_hz).to_owned()
            } else {
                String::new()
            },
        ));
        values.push((
            "device",
            if cfg.show_device {
                if self.status.connected {
                    self.status.via.clone()
                } else {
                    "Disconnected".to_owned()
                }
            } else {
                String::new()
            },
        ));
        values.push((
            "buttons",
            if cfg.show_buttons {
                if self.buttons == FACTORY_BUTTONS {
                    "Factory buttons".to_owned()
                } else {
                    "Custom buttons".to_owned()
                }
            } else {
                String::new()
            },
        ));
        values.push(("state", self.status.state_text().to_owned()));

        let details = discord::expand(&cfg.details, &values);
        let state = discord::expand(&cfg.state, &values);
        let key = format!("{details}\u{1f}{state}");
        if force || key != self.discord_last_key {
            self.discord_last_key = key;
            self.discord
                .update(cfg.client_id.clone(), Some(Presence { details, state }));
        }
    }

    fn scan_hid(&mut self) {
        let result = if self.show_all_hid {
            list_all_devices()
        } else {
            list_smx_devices()
        };
        match result {
            Ok(entries) => {
                self.hid_entries = entries;
                self.hid_scan_error = None;
            }
            Err(err) => {
                self.hid_entries.clear();
                self.hid_scan_error = Some(err);
            }
        }
    }

    // -----------------------------------------------------------------
    // Chrome
    // -----------------------------------------------------------------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            ui.label(RichText::new("SMX").size(21.0).strong().color(ACCENT));
            ui.label(RichText::new("HOWLER").size(21.0).strong());
            ui.add_space(6.0);
            ui.label(RichText::new("Control center").size(12.0).color(MUTED));

            right_aligned(ui, 44.0, |ui| {
                battery_chip(ui, &self.status);
                if self.busy {
                    ui.add_space(8.0);
                    ui.spinner();
                }
            });
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Configure via").size(12.0).color(MUTED));
            let mut chosen = self.bank;
            ui.add(egui::Button::selectable(
                chosen == Bank::Receiver,
                "2.4 GHz receiver",
            ))
            .clicked()
            .then(|| chosen = Bank::Receiver);
            ui.add(egui::Button::selectable(chosen == Bank::Cable, "USB cable"))
                .clicked()
                .then(|| chosen = Bank::Cable);
            if chosen != self.bank {
                self.set_bank(chosen);
            }

            ui.add_space(10.0);
            ui.label(RichText::new("·").color(MUTED));
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!(
                    "PID {:04X} · {}",
                    self.bank.pid(),
                    self.bank.label()
                ))
                .size(12.0)
                .color(MUTED),
            );

            right_aligned(ui, 24.0, |ui| {
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Refresh"))
                    .clicked()
                {
                    self.refresh_now();
                }
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Reload settings"))
                    .clicked()
                {
                    self.reload();
                }
                if let Some(at) = self.status_at {
                    ui.label(
                        RichText::new(format!("battery updated {}s ago", at.elapsed().as_secs()))
                            .size(11.0)
                            .color(MUTED),
                    );
                }
            });
        });
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.label(RichText::new("DEVICE").size(10.0).color(MUTED));
        ui.add_space(4.0);
        let color = if self.status.connected { GREEN } else { RED };
        ui.horizontal(|ui| {
            ui.label(RichText::new("•").color(color));
            ui.label(
                RichText::new(if self.status.connected {
                    self.status.via.as_str()
                } else {
                    "Not connected"
                })
                .size(12.0),
            );
        });
        ui.add_space(14.0);

        ui.label(RichText::new("SECTIONS").size(10.0).color(MUTED));
        ui.add_space(4.0);
        for (tab, title, hint) in Tab::ALL {
            let selected = self.tab == tab;
            let response = ui.add_sized(
                [ui.available_width(), 36.0],
                egui::Button::selectable(selected, RichText::new(title).size(14.0)),
            );
            let response = response.on_hover_text(hint);
            if response.clicked() {
                self.tab = tab;
            }
        }

        ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
            ui.add_space(4.0);
            if ui.button("Help & notes").clicked() {
                self.show_help = true;
            }
            ui.label(
                RichText::new("v0.1.0 · 258A:002E/002F")
                    .size(10.0)
                    .color(MUTED),
            );
            ui.add_space(6.0);
            if self.messages.iter().any(|e| e.level == MessageLevel::Error) {
                ui.label(
                    RichText::new("Last operation had an error")
                        .size(10.0)
                        .color(RED),
                );
            } else {
                ui.label(
                    RichText::new("Reads preserve unknown bytes")
                        .size(10.0)
                        .color(MUTED),
                );
            }
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(last) = self.messages.last() {
                let color = level_color(last.level);
                ui.label(RichText::new("•").size(9.0).color(color));
                ui.label(RichText::new(truncate(&last.text, 150)).size(11.0));
                ui.label(
                    RichText::new(format!("· {}s ago", last.at.elapsed().as_secs()))
                        .size(10.0)
                        .color(MUTED),
                );
            } else {
                ui.label(
                    RichText::new(
                        "Ready. Close the SMX Howler vendor app before applying settings.",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{} · {}", self.bank.label(), self.bank.pid()))
                        .size(10.0)
                        .color(MUTED),
                );
            });
        });
    }

    // -----------------------------------------------------------------
    // Overview
    // -----------------------------------------------------------------

    fn overview(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "Overview",
            "Live battery, link state and a quick health check for the mouse.",
        );

        if let Some(error) = self.status.error.clone() {
            error_banner(ui, &error);
            ui.add_space(8.0);
        }

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let color = battery_color(&self.status);
                    ui.label(
                        RichText::new(match self.status.percent {
                            Some(percent) => format!("{percent}%"),
                            None if self.status.charging => "Charging".to_owned(),
                            None => "—".to_owned(),
                        })
                        .size(46.0)
                        .strong()
                        .color(color),
                    );
                    ui.label(
                        RichText::new(self.status.state_text())
                            .size(14.0)
                            .color(MUTED),
                    );
                });
                ui.add_space(18.0);
                ui.separator();
                ui.add_space(18.0);
                ui.vertical(|ui| {
                    let source = if self.status.connected {
                        self.status.via.clone()
                    } else {
                        "No device".to_owned()
                    };
                    pill(
                        ui,
                        &source,
                        if self.status.connected { ACCENT } else { RED },
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!(
                            "{} · VID 258A · PID {:04X}",
                            if self.status.product.is_empty() {
                                "SMX Howler"
                            } else {
                                self.status.product.as_str()
                            },
                            self.bank.pid()
                        ))
                        .size(12.0)
                        .color(MUTED),
                    );
                    if !self.status.identity.is_empty() {
                        ui.label(
                            RichText::new(format!("Firmware identity: {}", self.status.identity))
                                .size(12.0)
                                .color(MUTED),
                        );
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Reload settings"))
                        .clicked()
                    {
                        self.reload();
                    }
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Refresh battery"))
                        .clicked()
                    {
                        self.refresh_now();
                    }
                });
            });

            ui.add_space(14.0);
            let fraction = self.status.percent.map(|p| p as f32 / 100.0).unwrap_or(0.0);
            let bar = egui::ProgressBar::new(fraction)
                .desired_width(ui.available_width())
                .fill(battery_color(&self.status))
                .corner_radius(7);
            ui.add(bar);
            ui.add_space(6.0);
            ui.label(
                RichText::new(if self.status.charging {
                    "Charging is only reported over USB cable. Over the receiver, the level is the last value sent by the mouse."
                } else {
                    "Battery is polled automatically. Over the receiver the mouse may briefly sleep and report a stale level."
                })
                .size(11.0)
                .color(MUTED),
            );
        });

        ui.add_space(12.0);
        ui.columns(2, |columns| {
            card(&mut columns[0], |ui| {
                ui.label(RichText::new("Device details").size(15.0).strong());
                ui.add_space(8.0);
                egui::Grid::new("device_details")
                    .num_columns(2)
                    .spacing([22.0, 8.0])
                    .show(ui, |ui| {
                        grid_row(
                            ui,
                            "Connection",
                            if self.status.connected {
                                "Online"
                            } else {
                                "Offline"
                            },
                        );
                        grid_row(ui, "Transport", &self.bank.label());
                        grid_row(
                            ui,
                            "Product",
                            if self.status.product.is_empty() {
                                "SMX Howler"
                            } else {
                                &self.status.product
                            },
                        );
                        grid_row(
                            ui,
                            "Receiver link",
                            match self.status.link {
                                Some(true) => "Awake",
                                Some(false) => "Asleep",
                                None => "—",
                            },
                        );
                        grid_row(
                            ui,
                            "Identity",
                            if self.status.identity.is_empty() {
                                "—"
                            } else {
                                &self.status.identity
                            },
                        );
                        grid_row(ui, "Configured via", &self.bank.label());
                    });
            });

            card(&mut columns[1], |ui| {
                ui.label(RichText::new("Current profile").size(15.0).strong());
                ui.add_space(8.0);
                egui::Grid::new("profile_summary")
                    .num_columns(2)
                    .spacing([22.0, 8.0])
                    .show(ui, |ui| {
                        grid_row(
                            ui,
                            "Active DPI",
                            &format!("{} DPI", self.draft.active_dpi()),
                        );
                        grid_row(ui, "Polling", polling_label(self.draft.polling_hz));
                        grid_row(
                            ui,
                            "DPI stage",
                            &format!("{} of {}", self.draft.active_stage, DPI_STAGES),
                        );
                        grid_row(ui, "Lighting", effect_name(self.draft.effect));
                        grid_row(
                            ui,
                            "Buttons",
                            &format!(
                                "{} customised",
                                self.buttons
                                    .iter()
                                    .enumerate()
                                    .filter(|(i, bytes)| **bytes != FACTORY_BUTTONS[*i])
                                    .count()
                            ),
                        );
                        grid_row(
                            ui,
                            "Settings block",
                            &match self.settings_block {
                                Some(ref block) => format!("{} bytes", block.len()),
                                None => "not loaded".to_owned(),
                            },
                        );
                    });
            });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(RichText::new("Recent activity").size(15.0).strong());
            ui.add_space(8.0);
            if self.messages.is_empty() {
                ui.label(RichText::new("No activity yet.").size(12.0).color(MUTED));
            } else {
                for entry in self.messages.iter().rev().take(6) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("•").size(8.0).color(level_color(entry.level)));
                        ui.label(RichText::new(&entry.text).size(12.0));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{}s ago", entry.at.elapsed().as_secs()))
                                    .size(10.0)
                                    .color(MUTED),
                            );
                        });
                    });
                    ui.add_space(2.0);
                }
            }
        });

        if !self.status.connected {
            ui.add_space(12.0);
            card(ui, |ui| {
                ui.label(RichText::new("Mouse not found?").size(15.0).strong());
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "1. Plug in the 2.4 GHz receiver or the USB cable.\n\
                         2. Switch the mouse on and move it so the receiver wakes it.\n\
                         3. Close the SMX Howler tray app (OemDrv.exe) — it holds the vendor HID channel.\n\
                         4. Click Refresh. Only one program can change settings at a time.",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
            });
        }
    }

    // -----------------------------------------------------------------
    // DPI and polling
    // -----------------------------------------------------------------

    fn dpi_tab(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "DPI & polling",
            "Six DPI stages are stored in the mouse. The active stage is used immediately.",
        );

        card(ui, |ui| {
            ui.label(RichText::new("Polling rate").size(15.0).strong());
            ui.label(
                RichText::new(
                    "USB report interval. Lower is faster; higher uses slightly less power.",
                )
                .size(11.0)
                .color(MUTED),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for hz in [125u16, 250, 500, 1000] {
                    let selected = self.draft.polling_hz == hz;
                    let label = format!("{}  ·  {}", polling_label(hz), polling_ms(hz));
                    if ui
                        .add_sized([150.0, 32.0], egui::Button::selectable(selected, label))
                        .clicked()
                    {
                        self.draft.polling_hz = hz;
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "1000 Hz is the factory setting. Some receivers prefer 500 Hz for a more stable link.",
                )
                .size(10.0)
                .color(MUTED),
            );
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("DPI stages").size(15.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(
                            "The colour is shown on the scroll-wheel LED for that stage.",
                        )
                        .size(10.0)
                        .color(MUTED),
                    );
                });
            });
            ui.add_space(8.0);
            egui::Grid::new("dpi_stages")
                .num_columns(4)
                .spacing([18.0, 10.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("ACTIVE").size(10.0).color(MUTED));
                    ui.label(RichText::new("STAGE").size(10.0).color(MUTED));
                    ui.label(RichText::new("SENSITIVITY").size(10.0).color(MUTED));
                    ui.label(RichText::new("WHEEL LED").size(10.0).color(MUTED));
                    ui.end_row();

                    for index in 0..DPI_STAGES {
                        let stage_number = (index + 1) as u8;
                        ui.radio_value(&mut self.draft.active_stage, stage_number, "");
                        ui.label(RichText::new(format!("Stage {}", stage_number)).size(13.0));

                        let mut current = self.draft.stages[index].dpi_index;
                        egui::ComboBox::from_id_salt(("dpi_stage", index))
                            .width(140.0)
                            .selected_text(format!("{} DPI", self.draft.stages[index].dpi()))
                            .show_ui(ui, |ui| {
                                for (dpi_index, value) in DPI_VALUES.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut current,
                                        dpi_index as u8,
                                        format!("{value} DPI"),
                                    );
                                }
                            });
                        self.draft.stages[index].dpi_index = current;

                        ui.horizontal(|ui| {
                            ui.color_edit_button_srgb(&mut self.draft.stages[index].color);
                            let [r, g, b] = self.draft.stages[index].color;
                            ui.label(
                                RichText::new(format!("#{r:02X}{g:02X}{b:02X}"))
                                    .size(11.0)
                                    .monospace()
                                    .color(MUTED),
                            );
                        });
                        ui.end_row();
                    }
                });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let dirty = self.draft != self.loaded_draft;
                if ui
                    .add_enabled(
                        !self.busy && dirty,
                        egui::Button::new("Apply DPI & polling"),
                    )
                    .clicked()
                {
                    self.apply_settings();
                }
                if ui.button("Factory stages").clicked() {
                    self.draft.stages = factory_stages();
                    self.draft.active_stage = 1;
                }
                if ui.button("Revert").clicked() {
                    self.draft = self.loaded_draft.clone();
                }
                if dirty {
                    ui.label(RichText::new("Unsaved changes").size(11.0).color(AMBER));
                } else {
                    ui.label(RichText::new("Matches the mouse").size(11.0).color(MUTED));
                }
            });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(RichText::new("Stage preview").size(15.0).strong());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for index in 0..DPI_STAGES {
                    let stage = &self.draft.stages[index];
                    let selected = self.draft.active_stage == (index + 1) as u8;
                    egui::Frame::new()
                        .fill(if selected {
                            CARD_BG_ALT
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::new(
                            1.0,
                            if selected { ACCENT } else { CARD_STROKE },
                        ))
                        .corner_radius(8)
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                let dot = Color32::from_rgb(
                                    stage.color[0],
                                    stage.color[1],
                                    stage.color[2],
                                );
                                ui.label(RichText::new("•").size(18.0).color(dot));
                                ui.label(RichText::new(format!("{}", stage.dpi())).strong());
                                ui.label(
                                    RichText::new(format!("Stage {}", index + 1))
                                        .size(9.0)
                                        .color(MUTED),
                                );
                            });
                        });
                }
            });
        });
    }

    // -----------------------------------------------------------------
    // Lighting
    // -----------------------------------------------------------------

    fn lighting_tab(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "Lighting",
            "Pick an effect, then tune the parameters that belong to it.",
        );

        card(ui, |ui| {
            ui.label(RichText::new("Effect").size(15.0).strong());
            ui.add_space(8.0);
            let width = (ui.available_width() - 12.0) / 2.0;
            egui::Grid::new("effect_grid")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    for code in 0..EFFECT_COUNT {
                        let selected = self.draft.effect == code;
                        let label = format!("{}  {}", code, effect_name(code));
                        let response = ui.add_sized(
                            [width, 30.0],
                            egui::Button::selectable(selected, RichText::new(label).size(13.0)),
                        );
                        let response = response.on_hover_text(effect_description(code));
                        if response.clicked() {
                            self.draft.effect = code;
                        }
                        if code % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
            ui.add_space(6.0);
            ui.label(
                RichText::new(effect_description(self.draft.effect))
                    .size(12.0)
                    .color(MUTED),
            );
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(RichText::new("Parameters").size(15.0).strong());
            ui.add_space(8.0);

            match self.draft.effect {
                0 => {
                    ui.label(
                        RichText::new("All body lighting is off. The scroll-wheel DPI indicator can stay lit.")
                            .size(12.0)
                            .color(MUTED),
                    );
                }
                1 => {
                    level_selector(ui, "Brightness", &mut self.draft.streaming_brightness);
                    ui.add_space(6.0);
                    level_selector(ui, "Speed", &mut self.draft.streaming_speed);
                }
                2 => {
                    level_selector(ui, "Brightness", &mut self.draft.steady_brightness);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Colour slot");
                        let mut slot = self.draft.steady_slot as usize;
                        egui::ComboBox::from_id_salt("steady_slot")
                            .width(110.0)
                            .selected_text(format!("Colour {}", slot + 1))
                            .show_ui(ui, |ui| {
                                for index in 0..STEADY_COLORS {
                                    ui.selectable_value(
                                        &mut slot,
                                        index,
                                        format!("Colour {}", index + 1),
                                    );
                                }
                            });
                        self.draft.steady_slot = slot as u8;
                        let color = self.draft.steady_colors[slot.min(STEADY_COLORS - 1)];
                        ui.label(
                            RichText::new("•")
                                .size(18.0)
                                .color(Color32::from_rgb(color[0], color[1], color[2])),
                        );
                    });
                    ui.add_space(8.0);
                    palette_editor(ui, &mut self.draft.steady_colors);
                }
                3 => {
                    level_selector(ui, "Brightness", &mut self.draft.breathing_brightness);
                    ui.add_space(6.0);
                    level_selector(ui, "Speed", &mut self.draft.breathing_speed);
                    ui.add_space(8.0);
                    ui.label(RichText::new("Palette").size(12.0).color(MUTED));
                    palette_editor(ui, &mut self.draft.breathing_colors);
                }
                7 => {
                    ui.horizontal(|ui| {
                        ui.label("Blink colour");
                        ui.color_edit_button_srgb(&mut self.draft.blink_color);
                        let [r, g, b] = self.draft.blink_color;
                        ui.label(
                            RichText::new(format!("#{r:02X}{g:02X}{b:02X}"))
                                .size(11.0)
                                .monospace()
                                .color(MUTED),
                        );
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("The default blink timing is used; 0x84/0x63 are preserved from the mouse.")
                            .size(11.0)
                            .color(MUTED),
                    );
                }
                _ => {
                    level_selector(ui, "Speed", &mut self.draft.shared_speed);
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("This shared speed value is also used by the other animated effects (4, 5, 6, 8 and 9).")
                            .size(11.0)
                            .color(MUTED),
                    );
                }
            }

            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let dirty = self.draft != self.loaded_draft;
                if ui
                    .add_enabled(!self.busy && dirty, egui::Button::new("Apply lighting"))
                    .clicked()
                {
                    self.apply_settings();
                }
                if ui.button("Revert").clicked() {
                    self.draft = self.loaded_draft.clone();
                }
                if dirty {
                    ui.label(RichText::new("Unsaved changes").size(11.0).color(AMBER));
                }
            });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(RichText::new("How the effects map").size(15.0).strong());
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "None · Colorful streaming · Steady · Breathing · Tail · Neon · Colorful steady · Blinking · Star blink · Wave\n\
                     Brightness and speed use 0–4. Steady chooses one of seven palette colours; Breathing cycles the whole palette.",
                )
                .size(11.0)
                .color(MUTED),
            );
        });
    }

    // -----------------------------------------------------------------
    // Buttons
    // -----------------------------------------------------------------

    fn buttons_tab(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "Buttons",
            "Remap the physical buttons. Assignments are written to the mouse, not stored on the PC.",
        );

        if let Some(slot) = self.capture {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("•").size(12.0).color(AMBER));
                    ui.label(
                        RichText::new(format!(
                            "Listening for a key for {}… press any key, or Esc to cancel.",
                            slot_label(slot)
                        ))
                        .size(13.0)
                        .strong(),
                    );
                });
            });
            ui.add_space(10.0);
        }

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Assignments").size(15.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.checkbox(&mut self.show_all_slots, "Show unused slots 8–20");
                });
            });
            ui.label(
                RichText::new("Buttons 1–7 are the physical buttons. Slots 8–20 are stored by the firmware but are normally unused.")
                    .size(11.0)
                    .color(MUTED),
            );
            ui.add_space(10.0);

            let visible = if self.show_all_slots { BUTTON_SLOTS } else { 7 };
            let mut edit_slot = None;
            let mut capture_slot = None;
            let mut reset_slot = None;

            for index in 0..visible {
                let action = ButtonAction::decode(self.buttons[index]);
                egui::Frame::new()
                    .fill(CARD_BG_ALT)
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{:02}", index + 1))
                                    .size(12.0)
                                    .monospace()
                                    .color(MUTED),
                            );
                            ui.add_space(4.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(slot_label(index)).size(13.0).strong());
                                ui.label(RichText::new(action.label()).size(12.0).color(MUTED));
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.button("Reset").clicked() {
                                    reset_slot = Some(index);
                                }
                                if ui.button("Edit…").clicked() {
                                    edit_slot = Some(index);
                                }
                                if ui.button("Capture key").clicked() {
                                    capture_slot = Some(index);
                                }
                            });
                        });
                    });
                ui.add_space(4.0);
            }

            if let Some(index) = reset_slot {
                self.buttons[index] = FACTORY_BUTTONS[index];
                self.log(
                    MessageLevel::Info,
                    format!("{} reset to the factory assignment.", slot_label(index)),
                );
            }
            if let Some(index) = edit_slot {
                self.editor = Some(ActionEditor::new(
                    index,
                    ButtonAction::decode(self.buttons[index]),
                ));
            }
            if let Some(index) = capture_slot {
                self.capture = Some(index);
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let dirty = self.buttons != self.loaded_buttons;
                if ui
                    .add_enabled(!self.busy && dirty, egui::Button::new("Apply assignments"))
                    .clicked()
                {
                    self.apply_buttons();
                }
                if ui.button("Factory defaults").clicked() {
                    self.buttons = FACTORY_BUTTONS;
                }
                if ui.button("Revert").clicked() {
                    self.buttons = self.loaded_buttons;
                }
                if dirty {
                    ui.label(RichText::new("Unsaved changes").size(11.0).color(AMBER));
                } else {
                    ui.label(RichText::new("Matches the mouse").size(11.0).color(MUTED));
                }
            });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(RichText::new("Supported actions").size(15.0).strong());
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Mouse buttons, back/forward, volume up/down, DPI up/down/loop, lighting toggle, disabled, \
                     keyboard shortcuts (with Ctrl/Shift/Alt), custom 4-byte actions, and firmware macros.\n\
                     Macros must first be created in the vendor application; the macro number is stored here.",
                )
                .size(11.0)
                .color(MUTED),
            );
        });
    }

    // -----------------------------------------------------------------
    // Discord
    // -----------------------------------------------------------------

    fn discord_tab(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "Discord",
            "Rich Presence shows the mouse state on your Discord profile. Mute and push-to-talk use keybinds.",
        );

        let status = self.discord.status();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Rich Presence").size(15.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    pill(
                        ui,
                        if status.connected {
                            "Connected"
                        } else {
                            "Offline"
                        },
                        if status.connected { GREEN } else { MUTED },
                    );
                });
            });
            ui.label(
                RichText::new(status.message.as_str())
                    .size(11.0)
                    .color(if status.connected { MUTED } else { AMBER }),
            );
            ui.add_space(10.0);
            ui.checkbox(
                &mut self.discord_form.enabled,
                "Enable Discord Rich Presence while this app is running",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Application Client ID");
                ui.add_sized(
                    [280.0, 24.0],
                    egui::TextEdit::singleline(&mut self.discord_form.client_id)
                        .hint_text("Paste the Discord application ID"),
                );
            });
            ui.label(
                RichText::new(
                    "Create an application at discord.com/developers/applications -> New Application, then copy its Application ID. \
                     Rich Presence works without a bot and without adding the app to a server.",
                )
                .size(11.0)
                .color(MUTED),
            );

            ui.add_space(12.0);
            egui::Grid::new("discord_fields")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Details");
                    ui.add_sized(
                        [380.0, 24.0],
                        egui::TextEdit::singleline(&mut self.discord_form.details),
                    );
                    ui.end_row();
                    ui.label("State");
                    ui.add_sized(
                        [380.0, 24.0],
                        egui::TextEdit::singleline(&mut self.discord_form.state),
                    );
                    ui.end_row();
                });
            ui.label(
                RichText::new(
                    "Placeholders: {battery}, {dpi}, {polling}, {device}, {buttons}, {state}. \
                     Each field is limited to 128 characters by Discord.",
                )
                .size(10.0)
                .color(MUTED),
            );

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.discord_form.show_battery, "Include battery");
                ui.checkbox(&mut self.discord_form.show_dpi, "Include DPI");
                ui.checkbox(&mut self.discord_form.show_device, "Include transport");
                ui.checkbox(
                    &mut self.discord_form.show_buttons,
                    "Include button profile",
                );
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Apply Discord settings").clicked() {
                    self.config.discord = self.discord_form.clone();
                    self.save_config();
                    self.discord_last_key.clear();
                    self.refresh_presence(true);
                    self.log(MessageLevel::Success, "Discord settings saved.");
                }
                if ui.button("Reconnect now").clicked() {
                    self.config.discord = self.discord_form.clone();
                    self.save_config();
                    self.discord_last_key.clear();
                    self.refresh_presence(true);
                }
                if ui.button("Disconnect").clicked() {
                    self.discord_last_key.clear();
                    self.discord.update(String::new(), None);
                }
            });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(
                RichText::new("Mute / push-to-talk quick assign")
                    .size(15.0)
                    .strong(),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Discord has no public mute or push-to-talk API. Assign Discord's own keyboard shortcuts to a mouse button, \
                     then match them in Discord -> User Settings -> Keybinds.",
                )
                .size(11.0)
                .color(MUTED),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Target button");
                let mut slot = self.discord_slot;
                egui::ComboBox::from_id_salt("discord_slot")
                    .width(150.0)
                    .selected_text(slot_label(slot))
                    .show_ui(ui, |ui| {
                        for index in 0..BUTTON_SLOTS {
                            ui.selectable_value(&mut slot, index, slot_label(index));
                        }
                    });
                self.discord_slot = slot;
            });
            ui.add_space(8.0);
            for preset in PRESETS {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(preset.name).size(13.0).strong());
                        ui.label(RichText::new(preset.description).size(11.0).color(MUTED));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("Assign & save"))
                            .clicked()
                        {
                            let action = ButtonAction::Keyboard {
                                modifiers: preset.modifiers,
                                usage: preset.usage,
                            };
                            self.buttons[self.discord_slot] = action.encode();
                            self.apply_buttons();
                            self.log(
                                MessageLevel::Info,
                                format!(
                                    "{} assigned to {} and sent to the mouse.",
                                    preset.name,
                                    slot_label(self.discord_slot)
                                ),
                            );
                        }
                    });
                });
                ui.add_space(2.0);
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Tip: F13/F14 are excellent push-to-talk keys because almost nothing else uses them. \
                     Windows may not report them in every app, but Discord accepts them.",
                )
                .size(10.0)
                .color(MUTED),
            );
        });
    }

    // -----------------------------------------------------------------
    // Advanced
    // -----------------------------------------------------------------

    fn advanced_tab(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "Advanced",
            "Raw protocol blocks, HID collections and diagnostics.",
        );

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("HID collections").size(15.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Scan").clicked() {
                        self.scan_hid();
                    }
                    ui.checkbox(&mut self.show_all_hid, "Show all HID devices");
                });
            });
            ui.add_space(6.0);
            if let Some(error) = &self.hid_scan_error {
                ui.label(RichText::new(error).size(11.0).color(RED));
            }
            if self.hid_entries.is_empty() {
                ui.label(
                    RichText::new("Click Scan to enumerate connected HID collections.")
                        .size(11.0)
                        .color(MUTED),
                );
            } else {
                egui::Grid::new("hid_table")
                    .num_columns(6)
                    .spacing([16.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for header in ["PID", "USAGE", "IF", "KIND", "PRODUCT", "PATH"] {
                            ui.label(RichText::new(header).size(10.0).color(MUTED));
                        }
                        ui.end_row();
                        for entry in &self.hid_entries {
                            ui.label(
                                RichText::new(format!("{:04X}", entry.pid))
                                    .size(11.0)
                                    .monospace(),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{:04X}:{:04X}",
                                    entry.usage_page, entry.usage
                                ))
                                .size(11.0)
                                .monospace(),
                            );
                            ui.label(RichText::new(entry.interface.to_string()).size(11.0));
                            ui.label(
                                RichText::new(if entry.is_vendor() { "vendor" } else { "HID" })
                                    .size(10.0)
                                    .color(if entry.is_vendor() { GREEN } else { MUTED }),
                            );
                            ui.label(
                                RichText::new(if entry.product.is_empty() {
                                    &entry.manufacturer
                                } else {
                                    &entry.product
                                })
                                .size(11.0),
                            );
                            ui.label(
                                RichText::new(truncate(&entry.path, 52))
                                    .size(10.0)
                                    .monospace()
                                    .color(MUTED),
                            )
                            .on_hover_text(&entry.path);
                            ui.end_row();
                        }
                    });
            }
        });

        ui.add_space(12.0);
        ui.columns(2, |columns| {
            card(&mut columns[0], |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Settings block").size(15.0).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                self.settings_block.is_some(),
                                egui::Button::new("Copy hex"),
                            )
                            .clicked()
                        {
                            if let Some(block) = &self.settings_block {
                                ui.ctx().copy_text(hex(block));
                            }
                        }
                    });
                });
                ui.add_space(6.0);
                match &self.settings_block {
                    Some(block) => {
                        egui::ScrollArea::vertical()
                            .id_salt("settings_hex")
                            .max_height(260.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label(RichText::new(pretty_hex(block)).size(10.0).monospace());
                            });
                    }
                    None => {
                        ui.label(
                            RichText::new("Load the settings from the mouse to see the block.")
                                .size(11.0)
                                .color(MUTED),
                        );
                    }
                }
            });

            card(&mut columns[1], |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Button block").size(15.0).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                self.buttons_block.is_some(),
                                egui::Button::new("Copy hex"),
                            )
                            .clicked()
                        {
                            if let Some(block) = &self.buttons_block {
                                ui.ctx().copy_text(hex(block));
                            }
                        }
                    });
                });
                ui.add_space(6.0);
                match &self.buttons_block {
                    Some(block) => {
                        egui::ScrollArea::vertical()
                            .id_salt("buttons_hex")
                            .max_height(260.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label(RichText::new(pretty_hex(block)).size(10.0).monospace());
                            });
                    }
                    None => {
                        ui.label(
                            RichText::new("Load the settings from the mouse to see the block.")
                                .size(11.0)
                                .color(MUTED),
                        );
                    }
                }
            });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(RichText::new("Diagnostics").size(15.0).strong());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Save hex dumps").clicked() {
                    match self.save_dumps() {
                        Ok(path) => self.log(
                            MessageLevel::Success,
                            format!("Hex dumps saved to {}", path.display()),
                        ),
                        Err(err) => self.log(MessageLevel::Error, err),
                    }
                }
                if ui.button("Scan HID").clicked() {
                    self.scan_hid();
                }
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Protocol: short commands use feature report 5 (8 bytes); blocks use feature report 8 (520 bytes).\n\
                     Settings block: 154 bytes incl. the 8-byte header. Button block: 88 bytes incl. header.\n\
                     Every write re-reads both blocks first, then sends the settings block followed by the button block, \
                     exactly like the vendor app. Unknown bytes are preserved.\n\
                     The vendor app (OemDrv.exe) must be closed while this app owns the HID channel.",
                )
                .size(11.0)
                .color(MUTED),
            );
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(RichText::new("Command line").size(15.0).strong());
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "smx_howler --list             list SMX Howler HID collections\n\
                     smx_howler --battery          print battery/status once\n\
                     smx_howler --dump-settings    print the settings block as hex\n\
                     smx_howler --dump-buttons     print the button block as hex\n\
                     smx_howler --help             show usage\n\n\
                     Release builds hide the console. Use `cargo run -- --battery` (debug) or build with \
                     `--features console` for command-line output.",
                )
                .size(11.0)
                .monospace()
                .color(MUTED),
            );
        });
    }

    fn save_dumps(&self) -> Result<std::path::PathBuf, String> {
        let settings = self
            .settings_block
            .as_ref()
            .ok_or_else(|| "Load the settings from the mouse first.".to_owned())?;
        let buttons = self
            .buttons_block
            .as_ref()
            .ok_or_else(|| "Load the settings from the mouse first.".to_owned())?;
        let dir = config::config_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|err| format!("Could not create {}: {err}", dir.display()))?;
        std::fs::write(dir.join("settings.hex"), pretty_hex(settings))
            .map_err(|err| format!("Could not write settings.hex: {err}"))?;
        std::fs::write(dir.join("buttons.hex"), pretty_hex(buttons))
            .map_err(|err| format!("Could not write buttons.hex: {err}"))?;
        Ok(dir)
    }

    // -----------------------------------------------------------------
    // Windows
    // -----------------------------------------------------------------

    fn editor_window(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;

        egui::Window::new(format!("Edit {}", slot_label(editor.slot)))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Choose an action, then apply it to the button.")
                        .size(11.0)
                        .color(MUTED),
                );
                ui.add_space(8.0);

                egui::Grid::new("action_editor_grid")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Action");
                        let old_kind = editor.kind;
                        egui::ComboBox::from_id_salt("action_kind")
                            .width(230.0)
                            .selected_text(editor.kind.label())
                            .show_ui(ui, |ui| {
                                for kind in ActionKind::ALL {
                                    ui.selectable_value(
                                        &mut editor.kind,
                                        kind,
                                        kind.label(),
                                    );
                                }
                            });
                        if editor.kind != old_kind {
                            editor.on_kind_changed();
                        }
                        ui.end_row();
                    });

                ui.add_space(8.0);
                match editor.kind {
                    ActionKind::Keyboard => {
                        ui.label(RichText::new("Modifiers").size(12.0).color(MUTED));
                        let mut ctrl = editor.modifiers & MOD_CTRL != 0;
                        let mut shift = editor.modifiers & MOD_SHIFT != 0;
                        let mut alt = editor.modifiers & MOD_ALT != 0;
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut ctrl, "Ctrl");
                            ui.checkbox(&mut shift, "Shift");
                            ui.checkbox(&mut alt, "Alt");
                        });
                        editor.modifiers = (if ctrl { MOD_CTRL } else { 0 })
                            | (if shift { MOD_SHIFT } else { 0 })
                            | (if alt { MOD_ALT } else { 0 });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label("Key");
                            egui::ComboBox::from_id_salt("key_usage")
                                .width(220.0)
                                .selected_text(editor.key_label())
                                .show_ui(ui, |ui| {
                                    for usage in key_usage_list() {
                                        ui.selectable_value(
                                            &mut editor.usage,
                                            usage,
                                            format!(
                                                "0x{usage:02X}  {}",
                                                hid_usage_name(usage)
                                            ),
                                        );
                                    }
                                });
                        });
                    }
                    ActionKind::Macro => {
                        egui::Grid::new("macro_fields")
                            .num_columns(2)
                            .spacing([14.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("Macro number");
                                ui.add(
                                    egui::DragValue::new(&mut editor.macro_number)
                                        .range(0..=255),
                                );
                                ui.end_row();
                                ui.label("Mode");
                                ui.add(
                                    egui::DragValue::new(&mut editor.macro_mode).range(0..=255),
                                );
                                ui.end_row();
                                ui.label("Event count");
                                ui.add(
                                    egui::DragValue::new(&mut editor.macro_count).range(0..=255),
                                );
                                ui.end_row();
                            });
                        ui.label(
                            RichText::new(
                                "The macro itself must be created in the vendor app first; only its number is stored here.",
                            )
                            .size(10.0)
                            .color(MUTED),
                        );
                    }
                    ActionKind::Custom => {
                        ui.horizontal(|ui| {
                            ui.label("Bytes");
                            ui.add_sized(
                                [190.0, 24.0],
                                egui::TextEdit::singleline(&mut editor.custom)
                                    .hint_text("AA BB CC DD"),
                            );
                        });
                        ui.label(
                            RichText::new(
                                "Four bytes, sent to the mouse exactly as entered. Useful for experimenting with vendor actions.",
                            )
                            .size(10.0)
                            .color(MUTED),
                        );
                    }
                    _ => {
                        ui.label(
                            RichText::new("This action has no extra parameters.")
                                .size(11.0)
                                .color(MUTED),
                        );
                    }
                }

                ui.add_space(10.0);
                match editor.build() {
                    Ok(action) => {
                        ui.label(
                            RichText::new(format!("Result: {}", action.label()))
                                .size(12.0)
                                .color(ACCENT),
                        );
                    }
                    Err(err) => {
                        ui.label(RichText::new(err).size(11.0).color(RED));
                    }
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(editor.build().is_ok(), egui::Button::new("Apply"))
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        if apply {
            match editor.build() {
                Ok(action) => {
                    self.buttons[editor.slot] = action.encode();
                    self.log(
                        MessageLevel::Info,
                        format!(
                            "{} set to {}. Press 'Apply assignments' to store it.",
                            slot_label(editor.slot),
                            action.label()
                        ),
                    );
                }
                Err(err) => {
                    self.log(MessageLevel::Error, err);
                    self.editor = Some(editor);
                }
            }
        } else if open && !cancel {
            self.editor = Some(editor);
        }
    }

    fn help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_help;
        egui::Window::new("Help & notes")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Getting connected").size(15.0).strong());
                ui.label(
                    RichText::new(
                        "Plug in the 2.4 GHz receiver (258A:002F) or the USB cable (258A:002E), wake the mouse, \
                         and close the SMX Howler tray app. Windows allows one program to own the vendor HID \
                         collection, so the vendor app and this app cannot both change settings.",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(10.0);

                ui.label(RichText::new("Battery").size(15.0).strong());
                ui.label(
                    RichText::new(
                        "Over USB the mouse reports charging or fully charged; over the receiver it reports a \
                         percentage. A sleeping mouse may keep the last value until it wakes. The battery is \
                         refreshed automatically every 30 seconds and can be refreshed manually at any time.",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(10.0);

                ui.label(RichText::new("Saving settings").size(15.0).strong());
                ui.label(
                    RichText::new(
                        "All settings are stored in the mouse. On every apply the app first re-reads both blocks, \
                         then writes the settings block and the button block, so bytes it does not understand \
                         survive unchanged. Keep the mouse awake while applying; the receiver rejects writes \
                         when the link is asleep.",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(10.0);

                ui.label(RichText::new("Discord").size(15.0).strong());
                ui.label(
                    RichText::new(
                        "Rich Presence needs a Discord application Client ID (Developer Portal -> New Application). \
                         Discord has no public mute/PTT API, so the Discord tab can assign Discord's default \
                         keyboard shortcuts to mouse buttons. Match them in Discord -> User Settings -> Keybinds.",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(10.0);
                ui.label(RichText::new("Troubleshooting").size(15.0).strong());
                ui.label(
                    RichText::new(
                        "• 'node not found': check the receiver/cable and try another USB port.\n\
                         • 'Could not open': close OemDrv.exe, then click Refresh.\n\
                         • Writes fail: move the mouse to wake the receiver link.\n\
                         • Lighting unchanged: confirm you clicked Apply, and that the mouse accepted the write \
                         (the activity log reports read-back differences).",
                    )
                    .size(12.0)
                    .color(MUTED),
                );
            });
        self.show_help = open;
    }
}

impl eframe::App for HowlerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump_events();
        self.handle_capture(ui);
        self.tick(ui.ctx());

        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(HEADER_BG)
                    .inner_margin(Margin::symmetric(18, 10)),
            )
            .show(ui, |ui| self.top_bar(ui));

        egui::Panel::left("navigation")
            .resizable(false)
            .exact_size(190.0)
            .frame(
                egui::Frame::new()
                    .fill(HEADER_BG)
                    .inner_margin(Margin::symmetric(12, 10)),
            )
            .show(ui, |ui| self.nav(ui));

        egui::Panel::bottom("footer")
            .frame(
                egui::Frame::new()
                    .fill(HEADER_BG)
                    .inner_margin(Margin::symmetric(18, 6)),
            )
            .show(ui, |ui| self.footer(ui));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(18, 12)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        match self.tab {
                            Tab::Overview => self.overview(ui),
                            Tab::Dpi => self.dpi_tab(ui),
                            Tab::Lighting => self.lighting_tab(ui),
                            Tab::Buttons => self.buttons_tab(ui),
                            Tab::Discord => self.discord_tab(ui),
                            Tab::Advanced => self.advanced_tab(ui),
                        }
                        ui.add_space(18.0);
                    });
            });

        self.editor_window(ui.ctx());
        self.help_window(ui.ctx());
    }
}

impl Drop for HowlerApp {
    fn drop(&mut self) {
        self.config.discord = self.discord_form.clone();
        let _ = config::save(&self.config);
    }
}

// ---------------------------------------------------------------------------
// Action editor
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActionKind {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    VolumeUp,
    VolumeDown,
    DpiUp,
    DpiDown,
    DpiLoop,
    LightingToggle,
    Disabled,
    Keyboard,
    Macro,
    Custom,
}

impl ActionKind {
    const ALL: [ActionKind; 15] = [
        ActionKind::Left,
        ActionKind::Right,
        ActionKind::Middle,
        ActionKind::Back,
        ActionKind::Forward,
        ActionKind::VolumeUp,
        ActionKind::VolumeDown,
        ActionKind::DpiUp,
        ActionKind::DpiDown,
        ActionKind::DpiLoop,
        ActionKind::LightingToggle,
        ActionKind::Disabled,
        ActionKind::Keyboard,
        ActionKind::Macro,
        ActionKind::Custom,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Left => "Left click",
            Self::Right => "Right click",
            Self::Middle => "Middle click",
            Self::Back => "Back",
            Self::Forward => "Forward",
            Self::VolumeUp => "Volume up",
            Self::VolumeDown => "Volume down",
            Self::DpiUp => "DPI up",
            Self::DpiDown => "DPI down",
            Self::DpiLoop => "DPI loop",
            Self::LightingToggle => "Lighting on/off",
            Self::Disabled => "Disabled",
            Self::Keyboard => "Keyboard shortcut",
            Self::Macro => "Macro",
            Self::Custom => "Custom bytes",
        }
    }

    fn simple(self) -> Option<ButtonAction> {
        Some(match self {
            Self::Left => ButtonAction::Left,
            Self::Right => ButtonAction::Right,
            Self::Middle => ButtonAction::Middle,
            Self::Back => ButtonAction::Back,
            Self::Forward => ButtonAction::Forward,
            Self::VolumeUp => ButtonAction::VolumeUp,
            Self::VolumeDown => ButtonAction::VolumeDown,
            Self::DpiUp => ButtonAction::DpiUp,
            Self::DpiDown => ButtonAction::DpiDown,
            Self::DpiLoop => ButtonAction::DpiLoop,
            Self::LightingToggle => ButtonAction::LightingToggle,
            Self::Disabled => ButtonAction::Disabled,
            _ => return None,
        })
    }
}

struct ActionEditor {
    slot: usize,
    kind: ActionKind,
    modifiers: u8,
    usage: u8,
    macro_number: u8,
    macro_mode: u8,
    macro_count: u8,
    custom: String,
}

impl ActionEditor {
    fn new(slot: usize, action: ButtonAction) -> Self {
        let mut editor = Self {
            slot,
            kind: ActionKind::Left,
            modifiers: 0,
            usage: 0x10,
            macro_number: 1,
            macro_mode: 1,
            macro_count: 1,
            custom: "00 00 00 00".to_owned(),
        };
        editor.kind = match action {
            ButtonAction::Left => ActionKind::Left,
            ButtonAction::Right => ActionKind::Right,
            ButtonAction::Middle => ActionKind::Middle,
            ButtonAction::Back => ActionKind::Back,
            ButtonAction::Forward => ActionKind::Forward,
            ButtonAction::VolumeUp => ActionKind::VolumeUp,
            ButtonAction::VolumeDown => ActionKind::VolumeDown,
            ButtonAction::DpiUp => ActionKind::DpiUp,
            ButtonAction::DpiDown => ActionKind::DpiDown,
            ButtonAction::DpiLoop => ActionKind::DpiLoop,
            ButtonAction::LightingToggle => ActionKind::LightingToggle,
            ButtonAction::Disabled => ActionKind::Disabled,
            ButtonAction::Keyboard { modifiers, usage } => {
                editor.modifiers = modifiers;
                editor.usage = usage;
                ActionKind::Keyboard
            }
            ButtonAction::Macro {
                number,
                mode,
                count,
            } => {
                editor.macro_number = number;
                editor.macro_mode = mode;
                editor.macro_count = count;
                ActionKind::Macro
            }
            ButtonAction::Custom(bytes) => {
                editor.custom = format!(
                    "{:02X} {:02X} {:02X} {:02X}",
                    bytes[0], bytes[1], bytes[2], bytes[3]
                );
                ActionKind::Custom
            }
        };
        editor
    }

    fn on_kind_changed(&mut self) {
        if self.kind == ActionKind::Custom {
            self.custom = "00 00 00 00".to_owned();
        }
    }

    fn key_label(&self) -> String {
        let mut text = String::new();
        if self.modifiers & MOD_CTRL != 0 {
            text.push_str("Ctrl+");
        }
        if self.modifiers & MOD_SHIFT != 0 {
            text.push_str("Shift+");
        }
        if self.modifiers & MOD_ALT != 0 {
            text.push_str("Alt+");
        }
        text.push_str(hid_usage_name(self.usage));
        text
    }

    fn build(&self) -> Result<ButtonAction, String> {
        if let Some(action) = self.kind.simple() {
            return Ok(action);
        }
        match self.kind {
            ActionKind::Keyboard => Ok(ButtonAction::Keyboard {
                modifiers: self.modifiers & 0x0F,
                usage: self.usage,
            }),
            ActionKind::Macro => Ok(ButtonAction::Macro {
                number: self.macro_number,
                mode: self.macro_mode,
                count: self.macro_count,
            }),
            ActionKind::Custom => parse_custom(&self.custom).map(ButtonAction::Custom),
            _ => Ok(ButtonAction::Disabled),
        }
    }
}

fn parse_custom(text: &str) -> Result<[u8; 4], String> {
    let cleaned: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if cleaned.len() != 8 {
        return Err("Enter exactly four bytes (eight hex digits).".to_owned());
    }
    let mut bytes = [0u8; 4];
    for index in 0..4 {
        let start = index * 2;
        bytes[index] = u8::from_str_radix(&cleaned[start..start + 2], 16)
            .map_err(|_| "Could not parse the hex bytes.".to_owned())?;
    }
    Ok(bytes)
}

fn key_usage_list() -> Vec<u8> {
    let mut usages: Vec<u8> = (0x04u8..=0x73).collect();
    usages.extend([0x7F, 0x80, 0x81]);
    usages
}

// ---------------------------------------------------------------------------
// Widgets and helpers
// ---------------------------------------------------------------------------

fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = CARD_BG;
    visuals.extreme_bg_color = Color32::from_rgb(12, 14, 18);
    visuals.faint_bg_color = Color32::from_rgb(28, 32, 40);
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(34, 38, 48);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(30, 34, 43);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, CARD_STROKE);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(44, 50, 63);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(40, 45, 57);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.7));
    visuals.widgets.active.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.widgets.active.weak_bg_fill = ACCENT.gamma_multiply(0.30);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.window_stroke = Stroke::new(1.0, CARD_STROKE);
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|style| {
        style.visuals = visuals.clone();
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 24.0;
    });
}

fn right_aligned<R>(
    ui: &mut egui::Ui,
    height: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let spacing = ui.spacing().item_spacing.x;
    let width = (ui.available_width() - spacing).max(0.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        Layout::right_to_left(Align::Center),
        add_contents,
    )
    .inner
}

fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(26.0).strong());
    ui.label(RichText::new(subtitle).size(12.0).color(MUTED));
    ui.add_space(12.0);
}

fn card<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(CARD_BG)
        .stroke(Stroke::new(1.0, CARD_STROKE))
        .corner_radius(10)
        .inner_margin(Margin::same(14))
        .show(ui, add_contents)
        .inner
}

fn grid_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).size(11.0).color(MUTED));
    ui.label(RichText::new(value).size(12.0));
    ui.end_row();
}

fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.55)))
        .corner_radius(20)
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(color));
        });
}

fn battery_chip(ui: &mut egui::Ui, status: &DeviceStatus) {
    let color = battery_color(status);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new(if status.connected {
                    status.battery_text()
                } else {
                    "Offline".to_owned()
                })
                .size(22.0)
                .strong()
                .color(color),
            );
            ui.label(RichText::new(status.state_text()).size(9.0).color(MUTED));
        });
        ui.label(RichText::new("•").size(18.0).color(color));
    });
}

fn battery_color(status: &DeviceStatus) -> Color32 {
    match status.percent {
        Some(percent) if percent <= 15 => RED,
        Some(percent) if percent <= 35 => AMBER,
        Some(_) => GREEN,
        None if status.full || status.charging => GREEN,
        None => MUTED,
    }
}

fn level_color(level: MessageLevel) -> Color32 {
    match level {
        MessageLevel::Info => ACCENT,
        MessageLevel::Success => GREEN,
        MessageLevel::Warning => AMBER,
        MessageLevel::Error => RED,
    }
}

fn error_banner(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(RED.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0, RED.gamma_multiply(0.45)))
        .corner_radius(8)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("!").size(18.0).strong().color(RED));
                ui.label(RichText::new(text).size(12.0));
            });
        });
}

fn level_selector(ui: &mut egui::Ui, label: &str, value: &mut u8) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0));
        for level in 0..=4u8 {
            if ui
                .add_sized(
                    [34.0, 26.0],
                    egui::Button::selectable(*value == level, level.to_string()),
                )
                .clicked()
            {
                *value = level;
            }
        }
        ui.label(
            RichText::new(match *value {
                0 => "off / slowest",
                1 => "low",
                2 => "medium",
                3 => "high",
                _ => "maximum",
            })
            .size(10.0)
            .color(MUTED),
        );
    });
}

fn palette_editor(ui: &mut egui::Ui, colors: &mut [[u8; 3]; 7]) {
    ui.horizontal_wrapped(|ui| {
        for (index, color) in colors.iter_mut().enumerate() {
            ui.vertical_centered(|ui| {
                ui.color_edit_button_srgb(color);
                ui.label(
                    RichText::new(format!("{}", index + 1))
                        .size(9.0)
                        .color(MUTED),
                );
            });
            ui.add_space(4.0);
        }
    });
}

fn factory_stages() -> [DpiStage; DPI_STAGES] {
    [
        DpiStage {
            dpi_index: 2,
            color: [0, 0, 0],
        },
        DpiStage {
            dpi_index: 6,
            color: [0, 0, 255],
        },
        DpiStage {
            dpi_index: 14,
            color: [0, 255, 0],
        },
        DpiStage {
            dpi_index: 30,
            color: [255, 0, 255],
        },
        DpiStage {
            dpi_index: 51,
            color: [0, 255, 255],
        },
        DpiStage {
            dpi_index: 58,
            color: [255, 255, 0],
        },
    ]
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
