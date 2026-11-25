use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use eframe::egui;

use crate::device::{self, SmxDevice};
use crate::protocol::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageLevel {
    Info,
    Success,
    Warning,
    Error,
}

pub enum Command {
    Refresh,
    Reload,
    SetTarget(Bank),
    ApplySettings(Box<SettingsDraft>),
    ApplyButtons(Box<[[u8; 4]; BUTTON_SLOTS]>),
}

pub enum Event {
    Busy(bool),
    Status(Box<DeviceStatus>),
    Blocks {
        bank: Bank,
        settings: Vec<u8>,
        buttons: Vec<u8>,
    },
    Message(MessageLevel, String),
}

pub struct Worker {
    tx: Sender<Command>,
}

impl Worker {
    pub fn spawn(ctx: egui::Context) -> (Self, Receiver<Event>) {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (evt_tx, evt_rx) = mpsc::channel();
        let _ = thread::Builder::new()
            .name("smx-howler-hid".to_owned())
            .spawn(move || run(cmd_rx, evt_tx, ctx));
        (Self { tx: cmd_tx }, evt_rx)
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }
}

struct State {
    ctx: egui::Context,
    evt_tx: Sender<Event>,
    cable: Option<SmxDevice>,
    receiver: Option<SmxDevice>,
    target: Bank,
    open_errors: Vec<String>,
}

impl State {
    fn emit(&self, event: Event) {
        let _ = self.evt_tx.send(event);
        self.ctx.request_repaint();
    }

    fn note(&self, level: MessageLevel, text: impl Into<String>) {
        self.emit(Event::Message(level, text.into()));
    }

    fn ensure(&mut self, bank: Bank) -> Result<(), String> {
        match bank {
            Bank::Cable if self.cable.is_none() => match SmxDevice::open(PID_CABLE) {
                Ok(dev) => {
                    self.cable = Some(dev);
                    self.open_errors.retain(|e| !e.contains("cable"));
                }
                Err(err) => {
                    self.open_errors.push(err.clone());
                    return Err(err);
                }
            },
            Bank::Receiver if self.receiver.is_none() => match SmxDevice::open(PID_RECEIVER) {
                Ok(dev) => {
                    self.receiver = Some(dev);
                    self.open_errors.retain(|e| !e.contains("receiver"));
                }
                Err(err) => {
                    self.open_errors.push(err.clone());
                    return Err(err);
                }
            },
            _ => {}
        }
        Ok(())
    }

    fn device(&self, bank: Bank) -> &SmxDevice {
        match bank {
            Bank::Cable => self.cable.as_ref().expect("cable opened"),
            Bank::Receiver => self.receiver.as_ref().expect("receiver opened"),
        }
    }

    fn device_mut(&mut self, bank: Bank) -> &mut SmxDevice {
        match bank {
            Bank::Cable => self.cable.as_mut().expect("cable opened"),
            Bank::Receiver => self.receiver.as_mut().expect("receiver opened"),
        }
    }

    /// Pick the requested bank, falling back to the other connected node.
    fn ensure_target(&mut self) -> Result<Bank, String> {
        if self.ensure(self.target).is_ok() {
            return Ok(self.target);
        }
        let fallback = match self.target {
            Bank::Cable => Bank::Receiver,
            Bank::Receiver => Bank::Cable,
        };
        if self.ensure(fallback).is_ok() {
            self.target = fallback;
            return Ok(fallback);
        }
        Err(no_device_message(&self.open_errors))
    }

    fn refresh_status(&mut self) {
        let _ = self.ensure(Bank::Cable);
        let _ = self.ensure(Bank::Receiver);
        let mut status = device::read_status(self.cable.as_ref(), self.receiver.as_ref());
        if !status.connected {
            status.error = Some(no_device_message(&self.open_errors));
        } else {
            status.error = None;
            self.open_errors.clear();
        }
        self.emit(Event::Status(Box::new(status)));
    }

    fn load_blocks(&mut self) -> Result<(Bank, Vec<u8>, Vec<u8>), String> {
        let bank = self.ensure_target()?;
        self.ensure(bank)?;
        self.device_mut(bank).ensure_config()?;
        let dev = self.device(bank);
        let (settings, buttons) = read_blocks(dev, bank)?;
        Ok((bank, settings, buttons))
    }

    fn reload(&mut self) {
        self.emit(Event::Busy(true));
        match self.load_blocks() {
            Ok((bank, settings, buttons)) => {
                self.emit(Event::Blocks {
                    bank,
                    settings,
                    buttons,
                });
            }
            Err(err) => self.note(MessageLevel::Error, err),
        }
        self.refresh_status();
        self.emit(Event::Busy(false));
    }

    fn apply_settings(&mut self, draft: SettingsDraft) {
        self.emit(Event::Busy(true));
        match self.try_apply_settings(&draft) {
            Ok((bank, settings, buttons, warning)) => {
                self.emit(Event::Blocks {
                    bank,
                    settings,
                    buttons,
                });
                if let Some(warning) = warning {
                    self.note(MessageLevel::Warning, warning);
                } else {
                    self.note(
                        MessageLevel::Success,
                        format!("Lighting, DPI and polling saved to the {}.", bank.label()),
                    );
                }
            }
            Err(err) => self.note(MessageLevel::Error, err),
        }
        self.refresh_status();
        self.emit(Event::Busy(false));
    }

    fn try_apply_settings(
        &mut self,
        draft: &SettingsDraft,
    ) -> Result<(Bank, Vec<u8>, Vec<u8>, Option<String>), String> {
        let bank = self.ensure_target()?;
        self.ensure(bank)?;
        self.device_mut(bank).ensure_config()?;
        let dev = self.device(bank);
        let (mut settings, buttons) = read_blocks(dev, bank)?;
        draft.apply_to(&mut settings);
        dev.write_pair(bank, &settings, &buttons)
            .map_err(|err| apply_error(err, bank))?;

        let settings_after = dev.read_block(bank.settings_cmd())?;
        let buttons_after = dev.read_block(bank.buttons_cmd())?;
        let warning = read_back_warning(&settings, &settings_after);
        Ok((bank, settings_after, buttons_after, warning))
    }

    fn apply_buttons(&mut self, slots: [[u8; 4]; BUTTON_SLOTS]) {
        self.emit(Event::Busy(true));
        match self.try_apply_buttons(&slots) {
            Ok((bank, settings, buttons, warning)) => {
                self.emit(Event::Blocks {
                    bank,
                    settings,
                    buttons,
                });
                if let Some(warning) = warning {
                    self.note(MessageLevel::Warning, warning);
                } else {
                    self.note(
                        MessageLevel::Success,
                        format!("Button assignments saved to the {}.", bank.label()),
                    );
                }
            }
            Err(err) => self.note(MessageLevel::Error, err),
        }
        self.refresh_status();
        self.emit(Event::Busy(false));
    }

    fn try_apply_buttons(
        &mut self,
        slots: &[[u8; 4]; BUTTON_SLOTS],
    ) -> Result<(Bank, Vec<u8>, Vec<u8>, Option<String>), String> {
        let bank = self.ensure_target()?;
        self.ensure(bank)?;
        self.device_mut(bank).ensure_config()?;
        let dev = self.device(bank);
        let (settings, mut buttons) = read_blocks(dev, bank)?;
        let original = buttons.clone();
        apply_buttons(&mut buttons, slots);
        dev.write_pair(bank, &settings, &buttons)
            .map_err(|err| apply_error(err, bank))?;

        let settings_after = dev.read_block(bank.settings_cmd())?;
        let buttons_after = dev.read_block(bank.buttons_cmd())?;
        let warning = read_back_warning(&buttons, &buttons_after);
        let _ = original;
        Ok((bank, settings_after, buttons_after, warning))
    }
}

fn run(cmd_rx: Receiver<Command>, evt_tx: Sender<Event>, ctx: egui::Context) {
    let mut state = State {
        ctx,
        evt_tx,
        cable: None,
        receiver: None,
        target: Bank::Receiver,
        open_errors: Vec::new(),
    };
    state.emit(Event::Busy(false));
    while let Ok(command) = cmd_rx.recv() {
        match command {
            Command::Refresh => {
                state.emit(Event::Busy(true));
                state.refresh_status();
                state.emit(Event::Busy(false));
            }
            Command::Reload => state.reload(),
            Command::SetTarget(bank) => {
                state.target = bank;
                state.reload();
            }
            Command::ApplySettings(draft) => state.apply_settings(*draft),
            Command::ApplyButtons(slots) => state.apply_buttons(*slots),
        }
    }
}

fn read_blocks(dev: &SmxDevice, bank: Bank) -> Result<(Vec<u8>, Vec<u8>), String> {
    let settings = dev.read_block(bank.settings_cmd())?;
    let buttons = dev.read_block(bank.buttons_cmd())?;
    if settings.len() < SETTINGS_LEN || settings[0] != 8 {
        return Err("settings reply was too short.".to_owned());
    }
    if buttons.len() < BUTTONS_LEN || buttons[0] != 8 {
        return Err("button reply was too short.".to_owned());
    }
    Ok((settings, buttons))
}

fn apply_error(err: String, bank: Bank) -> String {
    if err.contains("asleep") || err.contains("link") {
        err
    } else {
        format!(
            "{err} If this keeps happening, close the SMX Howler vendor app and reconnect \
             the {}.",
            bank.label()
        )
    }
}

fn read_back_warning(written: &[u8], read_back: &[u8]) -> Option<String> {
    if written.len() != read_back.len() {
        return Some("The mouse returned a different block length after writing.".to_owned());
    }
    let diffs = written
        .iter()
        .zip(read_back)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .count();
    if diffs == 0 {
        None
    } else {
        Some(format!(
            "Written, but the read-back differs in {diffs} byte(s). The mouse may have \
             refreshed live values (for example the active DPI stage)."
        ))
    }
}

fn no_device_message(errors: &[String]) -> String {
    let mut text = String::from(
        "No SMX Howler found. Plug in the 2.4 GHz receiver or the USB cable, make sure the \
         mouse is switched on, and close the SMX Howler vendor app (OemDrv.exe).",
    );
    for error in errors.iter().take(2) {
        text.push_str("\n\n");
        text.push_str(error);
    }
    text
}
