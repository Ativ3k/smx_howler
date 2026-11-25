use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Margin, RichText, Stroke};

use crate::config::{self, AppConfig};
use crate::device::{available_transports, list_all_devices, list_smx_devices, HidEntry};
use crate::discord::{self, Discord, DiscordConfig, Presence, PRESETS};
use crate::i18n::{tr, tr_format, Lang};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Tab {
    Overview,
    Dpi,
    Lighting,
    Buttons,
    Discord,
    Advanced,
}

impl Tab {
    const ALL: [Tab; 6] = [
        Tab::Overview,
        Tab::Dpi,
        Tab::Lighting,
        Tab::Buttons,
        Tab::Discord,
        Tab::Advanced,
    ];

    fn title(self) -> &'static str {
        match self {
            Tab::Overview => tr("Overview", "Przegląd"),
            Tab::Dpi => tr("DPI & polling", "DPI i polling"),
            Tab::Lighting => tr("Lighting", "Podświetlenie"),
            Tab::Buttons => tr("Buttons", "Przyciski"),
            Tab::Discord => "Discord",
            Tab::Advanced => tr("Advanced", "Zaawansowane"),
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Tab::Overview => tr("Battery and device status", "Bateria i stan urządzenia"),
            Tab::Dpi => tr(
                "Sensitivity and report rate",
                "Czułość i częstotliwość raportowania",
            ),
            Tab::Lighting => tr("Effects and colours", "Efekty i kolory"),
            Tab::Buttons => tr("Remap mouse buttons", "Przypisz przyciski myszy"),
            Tab::Discord => tr("Rich Presence and keybinds", "Rich Presence i skróty"),
            Tab::Advanced => tr("Raw blocks and HID details", "Surowe bloki i szczegóły HID"),
        }
    }
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
    usb_available: bool,
    receiver_available: bool,
    next_transport_check: Instant,
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
        let mut config = config::load();
        Lang::from_config(&config.language).set();
        let (usb_available, receiver_available) = available_transports();
        let mut bank = Bank::from_config(&config.bank);
        let mut startup_note = None;
        if bank == Bank::Cable && !usb_available && receiver_available {
            bank = Bank::Receiver;
            config.bank = bank.config_key().to_owned();
            let _ = config::save(&config);
            startup_note = Some(tr(
                "USB cable is not connected. Using the 2.4 GHz receiver.",
                "Kabel USB nie jest podłączony. Używam odbiornika 2.4 GHz.",
            ));
        }
        let (worker, events) = Worker::spawn(cc.egui_ctx.clone());
        let discord = Discord::spawn();
        let draft = SettingsDraft::default();
        let initial_refresh = Duration::from_secs(config.refresh_secs.clamp(10, 600));
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
            next_auto_refresh: Instant::now() + initial_refresh,
            usb_available,
            receiver_available,
            next_transport_check: Instant::now() + Duration::from_secs(2),
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
            tr(
                "Close the SMX Howler vendor app (OemDrv.exe) before applying settings — it keeps the vendor HID channel open.",
                "Zamknij fabryczną aplikację SMX Howler (OemDrv.exe) przed zapisem ustawień — blokuje kanał HID producenta.",
            ),
        );
        if let Some(note) = startup_note {
            app.log(MessageLevel::Warning, note);
        }
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
        if Instant::now() >= self.next_transport_check {
            self.next_transport_check = Instant::now() + Duration::from_secs(2);
            let (usb, receiver) = available_transports();
            let usb_lost = self.usb_available && !usb;
            let receiver_lost = self.receiver_available && !receiver;
            self.usb_available = usb;
            self.receiver_available = receiver;

            if !self.busy {
                if self.bank == Bank::Cable
                    && !usb
                    && receiver
                    && (usb_lost || !self.status.connected)
                {
                    self.log(
                        MessageLevel::Warning,
                        tr(
                            "USB cable disconnected. Switched to the 2.4 GHz receiver.",
                            "Kabel USB został odłączony. Przełączono na odbiornik 2.4 GHz.",
                        ),
                    );
                    self.set_bank(Bank::Receiver);
                } else if self.bank == Bank::Receiver
                    && !receiver
                    && usb
                    && (receiver_lost || !self.status.connected)
                {
                    self.log(
                        MessageLevel::Warning,
                        tr(
                            "2.4 GHz receiver disconnected. Switched to the USB cable.",
                            "Odbiornik 2.4 GHz został odłączony. Przełączono na kabel USB.",
                        ),
                    );
                    self.set_bank(Bank::Cable);
                }
            }
        }

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
                        tr_format!("{} set to {}. Press 'Apply assignments' to store it in the mouse.", "{} ustawiono na {}. Kliknij „Zapisz przypisania”, aby zapisać w myszy.", slot_label(slot),
                            action.label()),
                    );
                }
                Err(()) => {
                    self.log(
                        MessageLevel::Info,
                        tr(
                            "Key capture cancelled.",
                            "Przechwytywanie klawisza anulowane.",
                        ),
                    );
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
        if bank == Bank::Cable && !self.usb_available {
            self.log(
                MessageLevel::Warning,
                tr(
                    "USB cable is not connected. Plug in the cable (258A:002E) to use it.",
                    "Kabel USB nie jest podłączony. Podłącz kabel (258A:002E), aby go użyć.",
                ),
            );
            return;
        }
        if bank == Bank::Receiver && !self.receiver_available {
            self.log(
                MessageLevel::Warning,
                tr(
                    "The 2.4 GHz receiver is not connected. Plug in the receiver (258A:002F) to use it.",
                    "Odbiornik 2.4 GHz nie jest podłączony. Podłącz odbiornik (258A:002F), aby go użyć.",
                ),
            );
            return;
        }
        self.config.bank = bank.config_key().to_owned();
        self.save_config();
        self.switch_bank(bank);
    }

    fn switch_bank(&mut self, bank: Bank) {
        if bank == self.bank {
            return;
        }
        self.bank = bank;
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

    fn set_language(&mut self, lang: Lang) {
        if Lang::current() == lang {
            return;
        }
        lang.set();
        self.config.language = lang.config_key().to_owned();
        self.save_config();
        self.log(
            MessageLevel::Info,
            tr(
                "Interface language set to English.",
                "Język interfejsu ustawiono na polski.",
            ),
        );
        self.refresh_presence(true);
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
                    transport_label(&self.status.via).to_owned()
                } else {
                    tr("Disconnected", "Brak połączenia").to_owned()
                }
            } else {
                String::new()
            },
        ));
        values.push((
            "buttons",
            if cfg.show_buttons {
                if self.buttons == FACTORY_BUTTONS {
                    tr("Factory buttons", "Przyciski fabryczne").to_owned()
                } else {
                    tr("Custom buttons", "Przyciski własne").to_owned()
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
            ui.label(
                RichText::new(tr("Control center", "Centrum sterowania"))
                    .size(12.0)
                    .color(MUTED),
            );

            right_aligned(ui, 32.0, |ui| {
                if self.busy {
                    ui.spinner();
                    ui.add_space(8.0);
                }
                let current = Lang::current();
                for lang in [Lang::Pl, Lang::En] {
                    if ui
                        .add(egui::Button::selectable(current == lang, lang.label()))
                        .clicked()
                    {
                        self.set_language(lang);
                    }
                }
                ui.label(
                    RichText::new(tr("Language", "Język"))
                        .size(10.0)
                        .color(MUTED),
                );
            });
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tr("Configure via", "Połącz przez"))
                    .size(12.0)
                    .color(MUTED),
            );
            let mut chosen = self.bank;

            let receiver_text = if self.receiver_available {
                tr("2.4 GHz receiver", "Odbiornik 2.4 GHz")
            } else {
                tr(
                    "2.4 GHz receiver (offline)",
                    "Odbiornik 2.4 GHz (brak)",
                )
            };
            let receiver = ui.add_enabled(
                self.receiver_available,
                egui::Button::selectable(chosen == Bank::Receiver, receiver_text),
            );
            let receiver = if self.receiver_available {
                receiver
            } else {
                receiver.on_disabled_hover_text(tr(
                    "The 2.4 GHz receiver is not connected. Plug in the receiver (258A:002F) to select it.",
                    "Odbiornik 2.4 GHz nie jest podłączony. Podłącz odbiornik (258A:002F), aby go wybrać.",
                ))
            };
            if receiver.clicked() {
                chosen = Bank::Receiver;
            }
            if !self.receiver_available {
                ui.label(RichText::new("!").size(12.0).strong().color(AMBER))
                    .on_hover_text(tr(
                        "The 2.4 GHz receiver is not connected. Plug in the receiver (258A:002F) to select it.",
                        "Odbiornik 2.4 GHz nie jest podłączony. Podłącz odbiornik (258A:002F), aby go wybrać.",
                    ));
            }

            let usb_text = if self.usb_available {
                tr("USB cable", "Kabel USB")
            } else {
                tr("USB cable (offline)", "Kabel USB (brak)")
            };
            let usb = ui.add_enabled(
                self.usb_available,
                egui::Button::selectable(chosen == Bank::Cable, usb_text),
            );
            let usb = if self.usb_available {
                usb
            } else {
                usb.on_disabled_hover_text(tr(
                    "The USB cable is not connected. Plug in the cable (258A:002E) to select it.",
                    "Kabel USB nie jest podłączony. Podłącz kabel (258A:002E), aby go wybrać.",
                ))
            };
            if usb.clicked() {
                chosen = Bank::Cable;
            }
            if !self.usb_available {
                ui.label(RichText::new("!").size(12.0).strong().color(AMBER))
                    .on_hover_text(tr(
                        "The USB cable is not connected. Plug in the cable (258A:002E) to select it.",
                        "Kabel USB nie jest podłączony. Podłącz kabel (258A:002E), aby go wybrać.",
                    ));
            }

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

            ui.add_space(10.0);
            ui.label(RichText::new("·").color(MUTED));
            ui.add_space(10.0);
            battery_chip(ui, &self.status);

            right_aligned(ui, 24.0, |ui| {
                if ui
                    .add_enabled(!self.busy, egui::Button::new(tr("Refresh", "Odśwież")))
                    .clicked()
                {
                    self.refresh_now();
                }
                if ui
                    .add_enabled(
                        !self.busy,
                        egui::Button::new(tr("Reload settings", "Wczytaj ustawienia")),
                    )
                    .clicked()
                {
                    self.reload();
                }
                if ui.available_width() > 150.0 {
                    if let Some(at) = self.status_at {
                        ui.label(
                            RichText::new(tr_format!(
                                "battery updated {}s ago",
                                "bateria odświeżona {} s temu",
                                at.elapsed().as_secs()
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    }
                }
            });
        });
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.label(
            RichText::new(tr("DEVICE", "URZĄDZENIE"))
                .size(10.0)
                .color(MUTED),
        );
        ui.add_space(4.0);
        let color = if self.status.connected { GREEN } else { RED };
        ui.horizontal(|ui| {
            ui.label(RichText::new("•").color(color));
            ui.label(
                RichText::new(if self.status.connected {
                    transport_label(&self.status.via)
                } else {
                    tr("Not connected", "Nie połączono")
                })
                .size(12.0),
            );
        });
        ui.add_space(14.0);

        ui.label(
            RichText::new(tr("SECTIONS", "SEKCJE"))
                .size(10.0)
                .color(MUTED),
        );
        ui.add_space(4.0);
        for tab in Tab::ALL {
            let selected = self.tab == tab;
            let response = ui.add_sized(
                [ui.available_width(), 36.0],
                egui::Button::selectable(selected, RichText::new(tab.title()).size(14.0)),
            );
            let response = response.on_hover_text(tab.hint());
            if response.clicked() {
                self.tab = tab;
            }
        }

        ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
            ui.add_space(4.0);
            if ui.button(tr("Help & notes", "Pomoc i uwagi")).clicked() {
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
                    RichText::new(tr(
                        "Last operation had an error",
                        "Ostatnia operacja zakończyła się błędem",
                    ))
                    .size(10.0)
                    .color(RED),
                );
            } else {
                ui.label(
                    RichText::new(tr(
                        "Reads preserve unknown bytes",
                        "Odczyt zachowuje nieznane bajty",
                    ))
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
                    RichText::new(tr_format!(
                        "· {}s ago",
                        "· {} s temu",
                        last.at.elapsed().as_secs()
                    ))
                    .size(10.0)
                    .color(MUTED),
                );
            } else {
                ui.label(
                    RichText::new(tr(
                        "Ready. Close the SMX Howler vendor app before applying settings.",
                        "Gotowe. Zamknij fabryczną aplikację SMX Howler przed zapisem ustawień.",
                    ))
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
            tr("Overview", "Przegląd"),
            tr(
                "Live battery, link state and a quick health check for the mouse.",
                "Bateria na żywo, stan połączenia i szybka diagnostyka myszy.",
            ),
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
                            None if self.status.charging => tr("Charging", "Ładowanie").to_owned(),
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
                        transport_label(&self.status.via).to_owned()
                    } else {
                        tr("No device", "Brak urządzenia").to_owned()
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
                            RichText::new(tr_format!(
                                "Firmware identity: {}",
                                "Identyfikator firmware: {}",
                                self.status.identity
                            ))
                            .size(12.0)
                            .color(MUTED),
                        );
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                    if ui
                        .add_enabled(
                            !self.busy,
                            egui::Button::new(tr("Reload settings", "Wczytaj ustawienia")),
                        )
                        .clicked()
                    {
                        self.reload();
                    }
                    if ui
                        .add_enabled(
                            !self.busy,
                            egui::Button::new(tr("Refresh battery", "Odśwież baterię")),
                        )
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
                    tr(
                        "Charging is only reported over USB cable. Over the receiver, the level is the last value sent by the mouse.",
                        "Ładowanie jest raportowane tylko przez kabel USB. Przy odbiorniku poziom to ostatnia wartość wysłana przez mysz.",
                    )
                } else {
                    tr(
                        "Battery is polled automatically. Over the receiver the mouse may briefly sleep and report a stale level.",
                        "Bateria jest odpytywana automatycznie. Przy odbiorniku mysz może na chwilę zasnąć i pokazywać ostatnią wartość.",
                    )
                })
                .size(11.0)
                .color(MUTED),
            );
        });

        ui.add_space(12.0);
        ui.columns(2, |columns| {
            card(&mut columns[0], |ui| {
                ui.label(
                    RichText::new(tr("Device details", "Szczegóły urządzenia"))
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(8.0);
                egui::Grid::new("device_details")
                    .num_columns(2)
                    .spacing([22.0, 8.0])
                    .show(ui, |ui| {
                        grid_row(
                            ui,
                            tr("Connection", "Połączenie"),
                            if self.status.connected {
                                tr("Online", "Online")
                            } else {
                                tr("Offline", "Brak połączenia")
                            },
                        );
                        grid_row(ui, tr("Transport", "Transport"), self.bank.label());
                        grid_row(
                            ui,
                            tr("Product", "Produkt"),
                            if self.status.product.is_empty() {
                                "SMX Howler"
                            } else {
                                &self.status.product
                            },
                        );
                        grid_row(
                            ui,
                            tr("Receiver link", "Link odbiornika"),
                            match self.status.link {
                                Some(true) => tr("Awake", "Aktywny"),
                                Some(false) => tr("Asleep", "Uśpiony"),
                                None => "—",
                            },
                        );
                        grid_row(
                            ui,
                            tr("Identity", "Identyfikator"),
                            if self.status.identity.is_empty() {
                                "—"
                            } else {
                                &self.status.identity
                            },
                        );
                        grid_row(
                            ui,
                            tr("Configured via", "Skonfigurowano przez"),
                            self.bank.label(),
                        );
                    });
            });

            card(&mut columns[1], |ui| {
                ui.label(
                    RichText::new(tr("Current profile", "Bieżący profil"))
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(8.0);
                egui::Grid::new("profile_summary")
                    .num_columns(2)
                    .spacing([22.0, 8.0])
                    .show(ui, |ui| {
                        grid_row(
                            ui,
                            tr("Active DPI", "Aktywne DPI"),
                            &format!("{} DPI", self.draft.active_dpi()),
                        );
                        grid_row(
                            ui,
                            tr("Polling", "Polling"),
                            polling_label(self.draft.polling_hz),
                        );
                        grid_row(
                            ui,
                            tr("DPI stage", "Etap DPI"),
                            &tr_format!("{} of {}", "{} z {}", self.draft.active_stage, DPI_STAGES),
                        );
                        grid_row(
                            ui,
                            tr("Lighting", "Podświetlenie"),
                            effect_name(self.draft.effect),
                        );
                        grid_row(
                            ui,
                            tr("Buttons", "Przyciski"),
                            &tr_format!(
                                "{} customised",
                                "{} zmienionych",
                                self.buttons
                                    .iter()
                                    .enumerate()
                                    .filter(|(i, bytes)| **bytes != FACTORY_BUTTONS[*i])
                                    .count()
                            ),
                        );
                        grid_row(
                            ui,
                            tr("Settings block", "Blok ustawień"),
                            &match self.settings_block {
                                Some(ref block) => {
                                    tr_format!("{} bytes", "{} bajtów", block.len())
                                }
                                None => tr("not loaded", "nie wczytano").to_owned(),
                            },
                        );
                    });
            });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(
                RichText::new(tr("Recent activity", "Ostatnia aktywność"))
                    .size(15.0)
                    .strong(),
            );
            ui.add_space(8.0);
            if self.messages.is_empty() {
                ui.label(
                    RichText::new(tr("No activity yet.", "Brak aktywności."))
                        .size(12.0)
                        .color(MUTED),
                );
            } else {
                for entry in self.messages.iter().rev().take(6) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("•").size(8.0).color(level_color(entry.level)));
                        ui.label(RichText::new(&entry.text).size(12.0));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(tr_format!(
                                    "{}s ago",
                                    "{} s temu",
                                    entry.at.elapsed().as_secs()
                                ))
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
                ui.label(
                    RichText::new(tr("Mouse not found?", "Nie znaleziono myszy?"))
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(tr(
                        "1. Plug in the 2.4 GHz receiver or the USB cable.\n\
                         2. Switch the mouse on and move it so the receiver wakes it.\n\
                         3. Close the SMX Howler tray app (OemDrv.exe) — it holds the vendor HID channel.\n\
                         4. Click Refresh. Only one program can change settings at a time.",
                        "1. Podłącz odbiornik 2.4 GHz lub kabel USB.\n\
                         2. Włącz mysz i porusz nią, aby obudzić odbiornik.\n\
                         3. Zamknij aplikację SMX Howler (OemDrv.exe) — zajmuje kanał HID producenta.\n\
                         4. Kliknij Odśwież. Tylko jeden program może zmieniać ustawienia naraz.",
                    ))
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
            tr("DPI & polling", "DPI i polling"),
            tr(
                "Six DPI stages are stored in the mouse. The active stage is used immediately.",
                "Sześć etapów DPI jest zapisanych w myszy. Aktywny etap działa od razu.",
            ),
        );

        let column_spacing = ui.spacing().item_spacing.x;
        ui.spacing_mut().item_spacing.x = 18.0;
        ui.columns(2, |cols| {
            cols[0].spacing_mut().item_spacing.x = column_spacing;
            cols[1].spacing_mut().item_spacing.x = column_spacing;
            card(&mut cols[0], |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(
                    RichText::new(tr("Polling rate", "Częstotliwość raportowania"))
                        .size(15.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(tr(
                        "USB report interval. Lower is faster; higher uses slightly less power.",
                        "Interwał raportowania USB. Niższy jest szybszy; wyższy zużywa \
                     nieco mniej energii.",
                    ))
                    .size(11.0)
                    .color(MUTED),
                );
                ui.add_space(8.0);
                let button_width = (ui.available_width() - 8.0) / 2.0;
                egui::Grid::new("polling_grid")
                    .num_columns(2)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        for (index, hz) in [125u16, 250, 500, 1000].into_iter().enumerate() {
                            let selected = self.draft.polling_hz == hz;
                            let label = format!("{}  ·  {}", polling_label(hz), polling_ms(hz));
                            if ui
                                .add_sized(
                                    [button_width, 32.0],
                                    egui::Button::selectable(selected, label),
                                )
                                .clicked()
                            {
                                self.draft.polling_hz = hz;
                            }
                            if index % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(tr(
                        "1000 Hz is the factory setting. Some receivers prefer 500 Hz for a \
                     more stable link.",
                        "1000 Hz to ustawienie fabryczne. Niektóre odbiorniki wolą 500 Hz dla \
                     stabilniejszego połączenia.",
                    ))
                    .size(10.0)
                    .color(MUTED),
                );
            });

            card(&mut cols[1], |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(
                    RichText::new(tr("DPI stages", "Etapy DPI"))
                        .size(15.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(tr(
                        "The colour is shown on the scroll-wheel LED for that stage.",
                        "Kolor jest pokazywany na LED rolki dla danego etapu.",
                    ))
                    .size(10.0)
                    .color(MUTED),
                );
                ui.add_space(8.0);
                egui::Grid::new("dpi_stages")
                    .num_columns(4)
                    .spacing([8.0, 7.0])
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(tr("ACTIVE", "AKTYWNY"))
                                .size(10.0)
                                .color(MUTED),
                        );
                        ui.label(RichText::new(tr("STAGE", "ETAP")).size(10.0).color(MUTED));
                        ui.label(
                            RichText::new(tr("SENSITIVITY", "CZUŁOŚĆ"))
                                .size(10.0)
                                .color(MUTED),
                        );
                        ui.label(
                            RichText::new(tr("WHEEL LED", "LED ROLKI"))
                                .size(10.0)
                                .color(MUTED),
                        );
                        ui.end_row();

                        for index in 0..DPI_STAGES {
                            let stage_number = (index + 1) as u8;
                            ui.radio_value(&mut self.draft.active_stage, stage_number, "");
                            ui.label(
                                RichText::new(tr_format!("Stage {}", "Etap {}", stage_number))
                                    .size(13.0),
                            );

                            let mut current = self.draft.stages[index].dpi_index;
                            egui::ComboBox::from_id_salt(("dpi_stage", index))
                                .width(112.0)
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
                let dirty = self.draft != self.loaded_draft;
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.busy && dirty,
                            egui::Button::new(tr("Apply DPI & polling", "Zapisz DPI i polling")),
                        )
                        .clicked()
                    {
                        self.apply_settings();
                    }
                    if ui.button(tr("Factory stages", "Etapy fabryczne")).clicked() {
                        self.draft.stages = factory_stages();
                        self.draft.active_stage = 1;
                    }
                    if ui.button(tr("Revert", "Przywróć")).clicked() {
                        self.draft = self.loaded_draft.clone();
                    }
                });
                ui.add_space(4.0);
                if dirty {
                    ui.label(
                        RichText::new(tr("Unsaved changes", "Niezapisane zmiany"))
                            .size(11.0)
                            .color(AMBER),
                    );
                } else {
                    ui.label(
                        RichText::new(tr("Matches the mouse", "Zgodne z myszą"))
                            .size(11.0)
                            .color(MUTED),
                    );
                }
            });

            cols[0].add_space(12.0);
            card(&mut cols[0], |ui| {
                ui.label(
                    RichText::new(tr("Stage preview", "Podgląd etapów"))
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(8.0);
                let gap = 8.0;
                let preview_columns = 3usize;
                let cell_width = ((ui.available_width() - gap * (preview_columns as f32 - 1.0))
                    / preview_columns as f32)
                    .max(80.0);
                egui::Grid::new("stage_preview_grid")
                    .num_columns(preview_columns)
                    .spacing([gap, gap])
                    .show(ui, |ui| {
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
                                .inner_margin(Margin::same(8))
                                .show(ui, |ui| {
                                    ui.set_width((cell_width - 16.0).max(1.0));
                                    ui.spacing_mut().item_spacing.y = 3.0;
                                    ui.vertical_centered(|ui| {
                                        let dot = Color32::from_rgb(
                                            stage.color[0],
                                            stage.color[1],
                                            stage.color[2],
                                        );
                                        ui.label(RichText::new("•").size(16.0).color(dot));
                                        ui.label(
                                            RichText::new(format!("{}", stage.dpi())).strong(),
                                        );
                                        ui.label(
                                            RichText::new(tr_format!(
                                                "Stage {}",
                                                "Etap {}",
                                                index + 1
                                            ))
                                            .size(9.0)
                                            .color(MUTED),
                                        );
                                    });
                                });
                            if (index + 1) % preview_columns == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });
        });
        ui.spacing_mut().item_spacing.x = column_spacing;
    }

    // -----------------------------------------------------------------
    // Lighting
    // -----------------------------------------------------------------

    fn lighting_tab(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            tr("Lighting", "Podświetlenie"),
            tr(
                "Pick an effect, then tune the parameters that belong to it.",
                "Wybierz efekt, a następnie dostosuj należące do niego parametry.",
            ),
        );

        let column_spacing = ui.spacing().item_spacing.x;
        ui.spacing_mut().item_spacing.x = 18.0;
        ui.columns(2, |cols| {
            cols[0].spacing_mut().item_spacing.x = column_spacing;
            cols[1].spacing_mut().item_spacing.x = column_spacing;
            card(&mut cols[0], |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new(tr("Effect", "Efekt")).size(15.0).strong());
                ui.add_space(8.0);
                let width = (ui.available_width() - 10.0) / 2.0;
                egui::Grid::new("effect_grid")
                    .num_columns(2)
                    .spacing([10.0, 8.0])
                    .show(ui, |ui| {
                        for code in 0..EFFECT_COUNT {
                            let selected = self.draft.effect == code;
                            let response =
                                effect_button(ui, code, effect_name(code), selected, width);
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

            card(&mut cols[1], |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(
                    RichText::new(tr("Parameters", "Parametry"))
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(8.0);

                match self.draft.effect {
                    0 => {
                        ui.label(
                            RichText::new(tr(
                                "All body lighting is off. The scroll-wheel DPI indicator can \
                             stay lit.",
                                "Całe podświetlenie korpusu jest wyłączone. Wskaźnik DPI na rolce \
                             może pozostać podświetlony.",
                            ))
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
                            ui.label(tr("Colour slot", "Slot koloru"));
                            let mut slot = self.draft.steady_slot as usize;
                            egui::ComboBox::from_id_salt("steady_slot")
                                .width(110.0)
                                .selected_text(tr_format!("Colour {}", "Kolor {}", slot + 1))
                                .show_ui(ui, |ui| {
                                    for index in 0..STEADY_COLORS {
                                        ui.selectable_value(
                                            &mut slot,
                                            index,
                                            tr_format!("Colour {}", "Kolor {}", index + 1),
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
                        ui.label(
                            RichText::new(tr("Palette (7 colours)", "Paleta (7 kolorów)"))
                                .size(12.0)
                                .color(MUTED),
                        );
                        ui.add_space(4.0);
                        palette_editor(ui, &mut self.draft.steady_colors);
                    }
                    3 => {
                        level_selector(ui, "Brightness", &mut self.draft.breathing_brightness);
                        ui.add_space(6.0);
                        level_selector(ui, "Speed", &mut self.draft.breathing_speed);
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(tr("Palette", "Paleta"))
                                .size(12.0)
                                .color(MUTED),
                        );
                        palette_editor(ui, &mut self.draft.breathing_colors);
                    }
                    7 => {
                        ui.horizontal(|ui| {
                            ui.label(tr("Blink colour", "Kolor migania"));
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
                            RichText::new(tr(
                                "The default blink timing is used; 0x84/0x63 are preserved from \
                             the mouse.",
                                "Używane jest domyślne tempo migania; 0x84/0x63 są zachowywane \
                             z myszy.",
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    }
                    _ => {
                        level_selector(ui, "Speed", &mut self.draft.shared_speed);
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(tr(
                                "This shared speed value is also used by the other animated \
                             effects (4, 5, 6, 8 and 9).",
                                "Ta wspólna wartość prędkości jest używana także przez pozostałe \
                             animowane efekty (4, 5, 6, 8 i 9).",
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    }
                }

                ui.add_space(14.0);
                let dirty = self.draft != self.loaded_draft;
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.busy && dirty,
                            egui::Button::new(tr("Apply lighting", "Zapisz podświetlenie")),
                        )
                        .clicked()
                    {
                        self.apply_settings();
                    }
                    if ui.button(tr("Revert", "Przywróć")).clicked() {
                        self.draft = self.loaded_draft.clone();
                    }
                });
                if dirty {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(tr("Unsaved changes", "Niezapisane zmiany"))
                            .size(11.0)
                            .color(AMBER),
                    );
                }
            });
        });
        ui.spacing_mut().item_spacing.x = column_spacing;
    }

    // -----------------------------------------------------------------
    // Buttons
    // -----------------------------------------------------------------

    fn buttons_tab(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            tr("Buttons", "Przyciski"),
            tr(
                "Remap the physical buttons. Assignments are written to the mouse, not stored on the PC.",
                "Przypisz fizyczne przyciski. Ustawienia są zapisywane w myszy, a nie na komputerze.",
            ),
        );

        if let Some(slot) = self.capture {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("•").size(12.0).color(AMBER));
                    ui.label(
                        RichText::new(tr_format!("Listening for a key for {}… press any key, or Esc to cancel.", "Nasłuchiwanie klawisza dla {}… naciśnij dowolny klawisz lub Esc, aby anulować.", slot_label(slot)))
                        .size(13.0)
                        .strong(),
                    );
                });
            });
            ui.add_space(10.0);
        }

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(tr("Assignments", "Przypisania"))
                        .size(15.0)
                        .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.checkbox(
                        &mut self.show_all_slots,
                        tr("Show unused slots 8–20", "Pokaż nieużywane sloty 8–20"),
                    );
                });
            });
            ui.label(
                RichText::new(tr(
                    "Buttons 1–7 are the physical buttons. Slots 8–20 are stored by the firmware but are normally unused.",
                    "Przyciski 1–7 to fizyczne przyciski. Sloty 8–20 są przechowywane przez firmware, ale zwykle nieużywane.",
                ))
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
                                if ui.button(tr("Reset", "Resetuj")).clicked() {
                                    reset_slot = Some(index);
                                }
                                if ui.button(tr("Edit…", "Edytuj…")).clicked() {
                                    edit_slot = Some(index);
                                }
                                if ui.button(tr("Capture key", "Przechwyć klawisz")).clicked() {
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
                    tr_format!(
                        "{} reset to the factory assignment.",
                        "{} przywrócono do ustawienia fabrycznego.",
                        slot_label(index)
                    ),
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
                    .add_enabled(
                        !self.busy && dirty,
                        egui::Button::new(tr("Apply assignments", "Zapisz przypisania")),
                    )
                    .clicked()
                {
                    self.apply_buttons();
                }
                if ui
                    .button(tr("Factory defaults", "Ustawienia fabryczne"))
                    .clicked()
                {
                    self.buttons = FACTORY_BUTTONS;
                }
                if ui.button(tr("Revert", "Przywróć")).clicked() {
                    self.buttons = self.loaded_buttons;
                }
                if dirty {
                    ui.label(
                        RichText::new(tr("Unsaved changes", "Niezapisane zmiany"))
                            .size(11.0)
                            .color(AMBER),
                    );
                } else {
                    ui.label(
                        RichText::new(tr("Matches the mouse", "Zgodne z myszą"))
                            .size(11.0)
                            .color(MUTED),
                    );
                }
            });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(
                RichText::new(tr("Supported actions", "Obsługiwane akcje"))
                    .size(15.0)
                    .strong(),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr(
                    "Mouse buttons, back/forward, volume up/down, DPI up/down/loop, lighting toggle, disabled, \
                     keyboard shortcuts (with Ctrl/Shift/Alt), custom 4-byte actions, and firmware macros.\n\
                     Macros must first be created in the vendor application; the macro number is stored here.",
                    "Przyciski myszy, wstecz/dalej, głośność, DPI w górę/w dół/pętla, podświetlenie, wyłączenie, \
                     skróty klawiaturowe (z Ctrl/Shift/Alt), własne akcje 4-bajtowe i makra firmware.\n\
                     Makra trzeba najpierw utworzyć w aplikacji producenta; tutaj zapisywany jest tylko numer makra.",
                ))
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
            tr(
                "Rich Presence shows the mouse state on your Discord profile. Mute and push-to-talk use keybinds.",
                "Rich Presence pokazuje stan myszy na profilu Discord. Wyciszanie i push-to-talk działają przez skróty.",
            ),
        );

        let status = self.discord.status();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Rich Presence").size(15.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    pill(
                        ui,
                        if status.connected {
                            tr("Connected", "Połączono")
                        } else {
                            tr("Offline", "Brak połączenia")
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
                tr(
                    "Enable Discord Rich Presence while this app is running",
                    "Włącz Discord Rich Presence, gdy aplikacja działa",
                ),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(tr("Application Client ID", "ID aplikacji Discord"));
                ui.add_sized(
                    [280.0, 24.0],
                    egui::TextEdit::singleline(&mut self.discord_form.client_id).hint_text(tr(
                        "Paste the Discord application ID",
                        "Wklej ID aplikacji Discord",
                    )),
                );
            });
            ui.label(
                RichText::new(tr(
                    "Create an application at discord.com/developers/applications -> New Application, then copy its Application ID. \
                     Rich Presence works without a bot and without adding the app to a server.",
                    "Utwórz aplikację na discord.com/developers/applications -> New Application, a następnie skopiuj jej Application ID. \
                     Rich Presence działa bez bota i bez dodawania aplikacji do serwera.",
                ))
                .size(11.0)
                .color(MUTED),
            );

            ui.add_space(12.0);
            egui::Grid::new("discord_fields")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label(tr("Details", "Szczegóły"));
                    ui.add_sized(
                        [380.0, 24.0],
                        egui::TextEdit::singleline(&mut self.discord_form.details),
                    );
                    ui.end_row();
                    ui.label(tr("State", "Stan"));
                    ui.add_sized(
                        [380.0, 24.0],
                        egui::TextEdit::singleline(&mut self.discord_form.state),
                    );
                    ui.end_row();
                });
            ui.label(
                RichText::new(tr(
                    "Placeholders: {battery}, {dpi}, {polling}, {device}, {buttons}, {state}. \
                     Each field is limited to 128 characters by Discord.",
                    "Placeholdery: {battery}, {dpi}, {polling}, {device}, {buttons}, {state}. \
                     Discord ogranicza każde pole do 128 znaków.",
                ))
                .size(10.0)
                .color(MUTED),
            );

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.checkbox(
                    &mut self.discord_form.show_battery,
                    tr("Include battery", "Dołącz baterię"),
                );
                ui.checkbox(
                    &mut self.discord_form.show_dpi,
                    tr("Include DPI", "Dołącz DPI"),
                );
                ui.checkbox(
                    &mut self.discord_form.show_device,
                    tr("Include transport", "Dołącz transport"),
                );
                ui.checkbox(
                    &mut self.discord_form.show_buttons,
                    tr("Include button profile", "Dołącz profil przycisków"),
                );
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui
                    .button(tr("Apply Discord settings", "Zapisz ustawienia Discord"))
                    .clicked()
                {
                    self.config.discord = self.discord_form.clone();
                    self.save_config();
                    self.discord_last_key.clear();
                    self.refresh_presence(true);
                    self.log(
                        MessageLevel::Success,
                        tr("Discord settings saved.", "Ustawienia Discord zapisane."),
                    );
                }
                if ui.button(tr("Reconnect now", "Połącz ponownie")).clicked() {
                    self.config.discord = self.discord_form.clone();
                    self.save_config();
                    self.discord_last_key.clear();
                    self.refresh_presence(true);
                }
                if ui.button(tr("Disconnect", "Rozłącz")).clicked() {
                    self.discord_last_key.clear();
                    self.discord.update(String::new(), None);
                }
            });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(
                RichText::new(tr(
                    "Mute / push-to-talk quick assign",
                    "Szybkie przypisanie wyciszenia / push-to-talk",
                ))
                .size(15.0)
                .strong(),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr(
                    "Discord has no public mute or push-to-talk API. Assign Discord's own keyboard shortcuts to a mouse button, \
                     then match them in Discord -> User Settings -> Keybinds.",
                    "Discord nie udostępnia publicznego API wyciszania ani push-to-talk. Przypisz własne skróty Discorda do przycisku myszy, \
                     a następnie ustaw je w Discord -> Ustawienia użytkownika -> Skróty klawiszowe.",
                ))
                .size(11.0)
                .color(MUTED),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(tr("Target button", "Przycisk docelowy"));
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
                        ui.label(RichText::new(preset.localized_name()).size(13.0).strong());
                        ui.label(
                            RichText::new(preset.localized_description())
                                .size(11.0)
                                .color(MUTED),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.busy,
                                egui::Button::new(tr("Assign & save", "Przypisz i zapisz")),
                            )
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
                                tr_format!(
                                    "{} assigned to {} and sent to the mouse.",
                                    "{} przypisano do {} i wysłano do myszy.",
                                    preset.localized_name(),
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
                RichText::new(tr(
                    "Tip: F13/F14 are excellent push-to-talk keys because almost nothing else uses them. \
                     Windows may not report them in every app, but Discord accepts them.",
                    "Wskazówka: F13/F14 świetnie nadają się do push-to-talk, bo prawie nic ich nie używa. \
                     Windows może ich nie raportować w każdej aplikacji, ale Discord je akceptuje.",
                ))
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
            tr("Advanced", "Zaawansowane"),
            tr(
                "Raw protocol blocks, HID collections and diagnostics.",
                "Surowe bloki protokołu, kolekcje HID i diagnostyka.",
            ),
        );

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(tr("Automatic refresh", "Automatyczne odświeżanie"))
                        .size(15.0)
                        .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !self.busy,
                            egui::Button::new(tr("Refresh now", "Odśwież teraz")),
                        )
                        .clicked()
                    {
                        self.refresh_now();
                    }
                });
            });
            ui.add_space(8.0);

            let mut auto = self.config.auto_refresh;
            let mut secs = self.config.refresh_secs.clamp(10, 600);
            let mut changed = false;
            ui.horizontal(|ui| {
                changed |= ui
                    .checkbox(
                        &mut auto,
                        tr("Refresh automatically", "Odświeżaj automatycznie"),
                    )
                    .changed();
                ui.add_space(14.0);
                ui.label(tr("Interval", "Interwał"));
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut secs)
                            .range(10..=600)
                            .speed(1.0)
                            .suffix(" s"),
                    )
                    .changed();
                ui.label(
                    RichText::new(tr("10–600 s, default 30 s", "10–600 s, domyślnie 30 s"))
                        .size(10.0)
                        .color(MUTED),
                );
            });
            if changed {
                self.config.auto_refresh = auto;
                self.config.refresh_secs = secs.clamp(10, 600);
                self.next_auto_refresh =
                    Instant::now() + Duration::from_secs(self.config.refresh_secs);
                self.save_config();
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr(
                    "Changes are saved immediately. Automatic refresh runs only while the app is open.",
                    "Zmiany są zapisywane od razu. Automatyczne odświeżanie działa tylko przy otwartej aplikacji.",
                ))
                .size(10.0)
                .color(MUTED),
            );
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(tr("HID collections", "Kolekcje HID"))
                        .size(15.0)
                        .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(tr("Scan", "Skanuj")).clicked() {
                        self.scan_hid();
                    }
                    ui.checkbox(
                        &mut self.show_all_hid,
                        tr("Show all HID devices", "Pokaż wszystkie urządzenia HID"),
                    );
                });
            });
            ui.add_space(6.0);
            if let Some(error) = &self.hid_scan_error {
                ui.label(RichText::new(error).size(11.0).color(RED));
            }
            if self.hid_entries.is_empty() {
                ui.label(
                    RichText::new(tr(
                        "Click Scan to enumerate connected HID collections.",
                        "Kliknij Skanuj, aby wyliczyć podłączone kolekcje HID.",
                    ))
                    .size(11.0)
                    .color(MUTED),
                );
            } else {
                egui::Grid::new("hid_table")
                    .num_columns(6)
                    .spacing([16.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for header in [
                            tr("PID", "PID"),
                            tr("USAGE", "UŻYCIE"),
                            tr("IF", "IF"),
                            tr("KIND", "RODZAJ"),
                            tr("PRODUCT", "PRODUKT"),
                            tr("PATH", "ŚCIEŻKA"),
                        ] {
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
                                RichText::new(if entry.is_vendor() {
                                    tr("vendor", "producent")
                                } else {
                                    "HID"
                                })
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
                    ui.label(
                        RichText::new(tr("Settings block", "Blok ustawień"))
                            .size(15.0)
                            .strong(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                self.settings_block.is_some(),
                                egui::Button::new(tr("Copy hex", "Kopiuj hex")),
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
                            RichText::new(tr(
                                "Load the settings from the mouse to see the block.",
                                "Wczytaj ustawienia z myszy, aby zobaczyć blok.",
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    }
                }
            });

            card(&mut columns[1], |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(tr("Button block", "Blok przycisków"))
                            .size(15.0)
                            .strong(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                self.buttons_block.is_some(),
                                egui::Button::new(tr("Copy hex", "Kopiuj hex")),
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
                            RichText::new(tr(
                                "Load the settings from the mouse to see the block.",
                                "Wczytaj ustawienia z myszy, aby zobaczyć blok.",
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    }
                }
            });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(
                RichText::new(tr("Diagnostics", "Diagnostyka"))
                    .size(15.0)
                    .strong(),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .button(tr("Save hex dumps", "Zapisz zrzuty hex"))
                    .clicked()
                {
                    match self.save_dumps() {
                        Ok(path) => self.log(
                            MessageLevel::Success,
                            tr_format!(
                                "Hex dumps saved to {}",
                                "Zrzuty hex zapisano w {}",
                                path.display()
                            ),
                        ),
                        Err(err) => self.log(MessageLevel::Error, err),
                    }
                }
                if ui.button(tr("Scan HID", "Skanuj HID")).clicked() {
                    self.scan_hid();
                }
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(tr(
                    "Protocol: short commands use feature report 5 (8 bytes); blocks use feature report 8 (520 bytes).\n\
                     Settings block: 154 bytes incl. the 8-byte header. Button block: 88 bytes incl. header.\n\
                     Every write re-reads both blocks first, then sends the settings block followed by the button block, \
                     exactly like the vendor app. Unknown bytes are preserved.\n\
                     The vendor app (OemDrv.exe) must be closed while this app owns the HID channel.",
                    "Protokół: krótkie polecenia używają feature report 5 (8 bajtów); bloki używają feature report 8 (520 bajtów).\n\
                     Blok ustawień: 154 bajty z 8-bajtowym nagłówkiem. Blok przycisków: 88 bajtów z nagłówkiem.\n\
                     Każdy zapis najpierw odczytuje oba bloki, potem wysyła blok ustawień i blok przycisków, \
                     tak jak aplikacja producenta. Nieznane bajty są zachowywane.\n\
                     Aplikacja producenta (OemDrv.exe) musi być zamknięta, gdy ta aplikacja zajmuje kanał HID.",
                ))
                .size(11.0)
                .color(MUTED),
            );
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(
                RichText::new(tr("Command line", "Wiersz poleceń"))
                    .size(15.0)
                    .strong(),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr(
                    "smx_howler --list             list SMX Howler HID collections\n\
                     smx_howler --battery          print battery/status once\n\
                     smx_howler --dump-settings    print the settings block as hex\n\
                     smx_howler --dump-buttons     print the button block as hex\n\
                     smx_howler --help             show usage\n\n\
                     Release builds hide the console. Use `cargo run -- --battery` (debug) or build with \
                     `--features console` for command-line output.",
                    "smx_howler --list             lista kolekcji HID SMX Howler\n\
                     smx_howler --battery          jednorazowy odczyt baterii/statusu\n\
                     smx_howler --dump-settings    blok ustawień w formacie hex\n\
                     smx_howler --dump-buttons     blok przycisków w formacie hex\n\
                     smx_howler --help             pomoc\n\n\
                     Release ukrywa konsolę. Użyj `cargo run -- --battery` (debug) lub zbuduj z \
                     `--features console`, aby zobaczyć dane wyjściowe.",
                ))
                .size(11.0)
                .monospace()
                .color(MUTED),
            );
        });
    }

    fn save_dumps(&self) -> Result<std::path::PathBuf, String> {
        let settings = self.settings_block.as_ref().ok_or_else(|| {
            tr(
                "Load the settings from the mouse first.",
                "Najpierw wczytaj ustawienia z myszy.",
            )
            .to_owned()
        })?;
        let buttons = self.buttons_block.as_ref().ok_or_else(|| {
            tr(
                "Load the settings from the mouse first.",
                "Najpierw wczytaj ustawienia z myszy.",
            )
            .to_owned()
        })?;
        let dir = config::config_dir();
        std::fs::create_dir_all(&dir).map_err(|err| {
            tr_format!(
                "Could not create {}: {err}",
                "Nie można utworzyć {}: {err}",
                dir.display()
            )
        })?;
        std::fs::write(dir.join("settings.hex"), pretty_hex(settings)).map_err(|err| {
            tr_format!(
                "Could not write settings.hex: {err}",
                "Nie można zapisać settings.hex: {err}"
            )
        })?;
        std::fs::write(dir.join("buttons.hex"), pretty_hex(buttons)).map_err(|err| {
            tr_format!(
                "Could not write buttons.hex: {err}",
                "Nie można zapisać buttons.hex: {err}"
            )
        })?;
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

        egui::Window::new(tr_format!("Edit {}", "Edytuj {}", slot_label(editor.slot)))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .show(ctx, |ui| {
                ui.label(
                    RichText::new(tr(
                        "Choose an action, then apply it to the button.",
                        "Wybierz akcję, a następnie zastosuj ją do przycisku.",
                    ))
                    .size(11.0)
                    .color(MUTED),
                );
                ui.add_space(8.0);

                egui::Grid::new("action_editor_grid")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .show(ui, |ui| {
                        ui.label(tr("Action", "Akcja"));
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
                        ui.label(
                            RichText::new(tr("Modifiers", "Modyfikatory"))
                                .size(12.0)
                                .color(MUTED),
                        );
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
                            ui.label(tr("Key", "Klawisz"));
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
                                ui.label(tr("Macro number", "Numer makra"));
                                ui.add(
                                    egui::DragValue::new(&mut editor.macro_number)
                                        .range(0..=255),
                                );
                                ui.end_row();
                                ui.label(tr("Mode", "Tryb"));
                                ui.add(
                                    egui::DragValue::new(&mut editor.macro_mode).range(0..=255),
                                );
                                ui.end_row();
                                ui.label(tr("Event count", "Liczba zdarzeń"));
                                ui.add(
                                    egui::DragValue::new(&mut editor.macro_count).range(0..=255),
                                );
                                ui.end_row();
                            });
                        ui.label(
                            RichText::new(tr(
                                "The macro itself must be created in the vendor app first; only its number is stored here.",
                                "Samo makro trzeba najpierw utworzyć w aplikacji producenta; tutaj zapisywany jest tylko jego numer.",
                            ))
                            .size(10.0)
                            .color(MUTED),
                        );
                    }
                    ActionKind::Custom => {
                        ui.horizontal(|ui| {
                            ui.label(tr("Bytes", "Bajty"));
                            ui.add_sized(
                                [190.0, 24.0],
                                egui::TextEdit::singleline(&mut editor.custom)
                                    .hint_text("AA BB CC DD"),
                            );
                        });
                        ui.label(
                            RichText::new(tr(
                                "Four bytes, sent to the mouse exactly as entered. Useful for experimenting with vendor actions.",
                                "Cztery bajty wysyłane do myszy dokładnie tak, jak wpisano. Przydatne do testowania akcji producenta.",
                            ))
                            .size(10.0)
                            .color(MUTED),
                        );
                    }
                    _ => {
                        ui.label(
                            RichText::new(tr(
                                "This action has no extra parameters.",
                                "Ta akcja nie ma dodatkowych parametrów.",
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    }
                }

                ui.add_space(10.0);
                match editor.build() {
                    Ok(action) => {
                        ui.label(
                            RichText::new(tr_format!("Result: {}", "Wynik: {}", action.label()))
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
                        .add_enabled(
                            editor.build().is_ok(),
                            egui::Button::new(tr("Apply", "Zastosuj")),
                        )
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui.button(tr("Cancel", "Anuluj")).clicked() {
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
                        tr_format!("{} set to {}. Press 'Apply assignments' to store it.", "{} ustawiono na {}. Kliknij „Zapisz przypisania”, aby zapisać w myszy.", slot_label(editor.slot),
                            action.label()),
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
        egui::Window::new(tr("Help & notes", "Pomoc i uwagi"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(tr("Getting connected", "Połączenie"))
                        .size(15.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(tr(
                        "Plug in the 2.4 GHz receiver (258A:002F) or the USB cable (258A:002E), wake the mouse, \
                         and close the SMX Howler tray app. Windows allows one program to own the vendor HID \
                         collection, so the vendor app and this app cannot both change settings.",
                        "Podłącz odbiornik 2.4 GHz (258A:002F) lub kabel USB (258A:002E), obudź mysz \
                         i zamknij aplikację SMX Howler. Windows pozwala jednemu programowi zająć kolekcję HID \
                         producenta, więc aplikacja producenta i ta aplikacja nie mogą jednocześnie zmieniać ustawień.",
                    ))
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(10.0);

                ui.label(
                    RichText::new(tr("Battery", "Bateria"))
                        .size(15.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(tr(
                        "Over USB the mouse reports charging or fully charged; over the receiver it reports a \
                         percentage. A sleeping mouse may keep the last value until it wakes. The battery is \
                         refreshed automatically every 30 seconds and can be refreshed manually at any time.",
                        "Przez USB mysz raportuje ładowanie lub pełne naładowanie; przez odbiornik podaje \
                         procent. Uśpiona mysz może zachować ostatnią wartość do wybudzenia. Bateria jest \
                         odświeżana automatycznie co 30 sekund i można ją odświeżyć ręcznie w każdej chwili.",
                    ))
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(10.0);

                ui.label(
                    RichText::new(tr("Saving settings", "Zapis ustawień"))
                        .size(15.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(tr(
                        "All settings are stored in the mouse. On every apply the app first re-reads both blocks, \
                         then writes the settings block and the button block, so bytes it does not understand \
                         survive unchanged. Keep the mouse awake while applying; the receiver rejects writes \
                         when the link is asleep.",
                        "Wszystkie ustawienia są przechowywane w myszy. Przy każdym zapisie aplikacja najpierw \
                         odczytuje oba bloki, potem zapisuje blok ustawień i blok przycisków, więc nieznane bajty \
                         pozostają bez zmian. Utrzymuj mysz wybudzoną podczas zapisu; odbiornik odrzuca zapis, \
                         gdy link jest uśpiony.",
                    ))
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(10.0);

                ui.label(RichText::new("Discord").size(15.0).strong());
                ui.label(
                    RichText::new(tr(
                        "Rich Presence needs a Discord application Client ID (Developer Portal -> New Application). \
                         Discord has no public mute/PTT API, so the Discord tab can assign Discord's default \
                         keyboard shortcuts to mouse buttons. Match them in Discord -> User Settings -> Keybinds.",
                        "Rich Presence wymaga Client ID aplikacji Discord (Developer Portal -> New Application). \
                         Discord nie ma publicznego API wyciszania/PTT, więc zakładka Discord może przypisać \
                         domyślne skróty Discorda do przycisków myszy. Ustaw je w Discord -> Ustawienia użytkownika -> Skróty klawiszowe.",
                    ))
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(tr("Troubleshooting", "Rozwiązywanie problemów"))
                        .size(15.0)
                        .strong(),
                );
                ui.label(
                    RichText::new(tr(
                        "• 'node not found': check the receiver/cable and try another USB port.\n\
                         • 'Could not open': close OemDrv.exe, then click Refresh.\n\
                         • Writes fail: move the mouse to wake the receiver link.\n\
                         • Lighting unchanged: confirm you clicked Apply, and that the mouse accepted the write \
                         (the activity log reports read-back differences).",
                        "• „node not found”: sprawdź odbiornik/kabel i spróbuj innego portu USB.\n\
                         • „Could not open”: zamknij OemDrv.exe i kliknij Odśwież.\n\
                         • Zapis nie działa: porusz myszą, aby obudzić link odbiornika.\n\
                         • Podświetlenie się nie zmienia: upewnij się, że kliknięto Zastosuj i że mysz przyjęła \
                         zapis (dziennik aktywności pokazuje różnice po odczycie).",
                    ))
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
                    .id_salt(self.tab)
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
            Self::Left => tr("Left click", "Lewy klik"),
            Self::Right => tr("Right click", "Prawy klik"),
            Self::Middle => tr("Middle click", "Środkowy klik"),
            Self::Back => tr("Back", "Wstecz"),
            Self::Forward => tr("Forward", "Dalej"),
            Self::VolumeUp => tr("Volume up", "Głośniej"),
            Self::VolumeDown => tr("Volume down", "Ciszej"),
            Self::DpiUp => tr("DPI up", "DPI w górę"),
            Self::DpiDown => tr("DPI down", "DPI w dół"),
            Self::DpiLoop => tr("DPI loop", "Pętla DPI"),
            Self::LightingToggle => tr("Lighting on/off", "Podświetlenie wł./wył."),
            Self::Disabled => tr("Disabled", "Wyłączone"),
            Self::Keyboard => tr("Keyboard shortcut", "Skrót klawiaturowy"),
            Self::Macro => tr("Macro", "Makro"),
            Self::Custom => tr("Custom bytes", "Własne bajty"),
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
        return Err(tr(
            "Enter exactly four bytes (eight hex digits).",
            "Wpisz dokładnie cztery bajty (osiem cyfr hex).",
        )
        .to_owned());
    }
    let mut bytes = [0u8; 4];
    for index in 0..4 {
        let start = index * 2;
        bytes[index] = u8::from_str_radix(&cleaned[start..start + 2], 16).map_err(|_| {
            tr(
                "Could not parse the hex bytes.",
                "Nie można odczytać bajtów hex.",
            )
            .to_owned()
        })?;
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

fn transport_label(via: &str) -> &str {
    match via {
        "USB cable" => tr("USB cable", "Kabel USB"),
        "2.4 GHz receiver" => tr("2.4 GHz receiver", "Odbiornik 2.4 GHz"),
        other => other,
    }
}

fn battery_chip(ui: &mut egui::Ui, status: &DeviceStatus) {
    let color = battery_color(status);
    if status.connected {
        ui.label(
            RichText::new(status.battery_text())
                .size(14.0)
                .strong()
                .color(color),
        );
        ui.label(RichText::new("·").size(10.0).color(MUTED));
        ui.label(RichText::new(status.state_text()).size(11.0).color(MUTED));
    } else {
        ui.label(
            RichText::new(tr("Offline", "Brak połączenia"))
                .size(11.0)
                .color(MUTED),
        );
    }
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

fn effect_button(
    ui: &mut egui::Ui,
    code: u8,
    name: &str,
    selected: bool,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 32.0), egui::Sense::click());
    let (fill, stroke, badge_fill, badge_text, text_color) = {
        let visuals = ui.visuals();
        if selected {
            (
                visuals.selection.bg_fill,
                visuals.selection.stroke,
                ACCENT,
                BG,
                Color32::WHITE,
            )
        } else if response.hovered() {
            (
                visuals.widgets.hovered.weak_bg_fill,
                visuals.widgets.hovered.bg_stroke,
                ACCENT.gamma_multiply(0.18),
                ACCENT,
                visuals.text_color(),
            )
        } else {
            (
                visuals.widgets.inactive.weak_bg_fill,
                Stroke::new(1.0, CARD_STROKE),
                ACCENT.gamma_multiply(0.18),
                ACCENT,
                visuals.text_color(),
            )
        }
    };

    ui.painter()
        .rect(rect, 8.0, fill, stroke, egui::StrokeKind::Inside);
    let badge = egui::Rect::from_min_size(rect.min + egui::vec2(7.0, 6.0), egui::vec2(20.0, 20.0));
    ui.painter().rect_filled(badge, 6.0, badge_fill);
    ui.painter().text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        code.to_string(),
        egui::FontId::proportional(11.0),
        badge_text,
    );
    ui.painter().text(
        egui::pos2(badge.right() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(12.5),
        text_color,
    );

    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(effect_description(code))
}

fn level_selector(ui: &mut egui::Ui, label: &str, value: &mut u8) {
    let label = match label {
        "Brightness" => tr("Brightness", "Jasność"),
        "Speed" => tr("Speed", "Prędkość"),
        other => other,
    };
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
                0 => tr("off / slowest", "wył. / najwolniej"),
                1 => tr("low", "niska"),
                2 => tr("medium", "średnia"),
                3 => tr("high", "wysoka"),
                _ => tr("maximum", "maksymalna"),
            })
            .size(10.0)
            .color(MUTED),
        );
    });
}

fn palette_editor(ui: &mut egui::Ui, colors: &mut [[u8; 3]; 7]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        for (index, color) in colors.iter_mut().enumerate() {
            ui.allocate_ui_with_layout(
                egui::vec2(40.0, 44.0),
                Layout::top_down(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.color_edit_button_srgb(color);
                    egui::Frame::new()
                        .fill(ACCENT.gamma_multiply(0.18))
                        .stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.35)))
                        .corner_radius(6)
                        .inner_margin(Margin::symmetric(6, 1))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(format!("{}", index + 1))
                                    .size(9.5)
                                    .strong()
                                    .color(ACCENT),
                            );
                        });
                },
            );
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
