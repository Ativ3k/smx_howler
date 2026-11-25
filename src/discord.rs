//! Minimal Discord Rich Presence client over the local IPC named pipe.
//!
//! No third-party Discord crate is used: the protocol is just a little-endian
//! opcode/length header followed by a JSON object, written to
//! `\\.\pipe\discord-ipc-0` … `discord-ipc-9`.
//!
//! Discord has no public API for muting or push-to-talk, so the application
//! provides ready-made keyboard shortcuts that can be assigned to mouse
//! buttons and configured in Discord's own keybind settings.

use std::fs::File;
use std::io::{Read, Write};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const MAX_FRAME: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordConfig {
    pub enabled: bool,
    pub client_id: String,
    pub details: String,
    pub state: String,
    pub show_battery: bool,
    pub show_dpi: bool,
    pub show_buttons: bool,
    pub show_device: bool,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            client_id: String::new(),
            details: "SMX Howler".to_owned(),
            state: "{battery} · {dpi} DPI".to_owned(),
            show_battery: true,
            show_dpi: true,
            show_buttons: true,
            show_device: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DiscordStatus {
    pub connected: bool,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Presence {
    pub details: String,
    pub state: String,
}

#[derive(Clone, Copy, Debug)]
pub struct DiscordPreset {
    pub name: &'static str,
    pub description: &'static str,
    pub modifiers: u8,
    pub usage: u8,
}

/// Discord's default keyboard shortcuts. The user can change them in Discord,
/// so these are suggestions rather than fixed protocol values.
pub const PRESETS: [DiscordPreset; 4] = [
    DiscordPreset {
        name: "Mute",
        description: "Ctrl+Shift+M (Discord default)",
        modifiers: crate::protocol::MOD_CTRL | crate::protocol::MOD_SHIFT,
        usage: 0x10,
    },
    DiscordPreset {
        name: "Deafen",
        description: "Ctrl+Shift+D (Discord default)",
        modifiers: crate::protocol::MOD_CTRL | crate::protocol::MOD_SHIFT,
        usage: 0x07,
    },
    DiscordPreset {
        name: "Push to talk",
        description: "F13 — assign F13 in Discord keybinds",
        modifiers: 0,
        usage: 0x68,
    },
    DiscordPreset {
        name: "Push to mute",
        description: "F14 — assign F14 in Discord keybinds",
        modifiers: 0,
        usage: 0x69,
    },
];

enum Command {
    Update {
        client_id: String,
        presence: Option<Presence>,
    },
}

pub struct Discord {
    tx: Sender<Command>,
    status: Arc<Mutex<DiscordStatus>>,
}

impl Discord {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(DiscordStatus {
            connected: false,
            message: "Discord Rich Presence is disabled.".to_owned(),
        }));
        let status_clone = Arc::clone(&status);
        let _ = thread::Builder::new()
            .name("smx-howler-discord".to_owned())
            .spawn(move || run(rx, status_clone));
        Self { tx, status }
    }

    pub fn update(&self, client_id: String, presence: Option<Presence>) {
        let _ = self.tx.send(Command::Update {
            client_id,
            presence,
        });
    }

    pub fn status(&self) -> DiscordStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }
}

fn run(rx: mpsc::Receiver<Command>, status: Arc<Mutex<DiscordStatus>>) {
    let mut connection: Option<Connection> = None;
    let mut client_id = String::new();
    let mut desired: Option<Presence> = None;
    let mut dirty = false;
    let mut last_attempt = Instant::now() - Duration::from_secs(60);

    loop {
        let timeout = if dirty && connection.is_none() {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(1)
        };

        match rx.recv_timeout(timeout) {
            Ok(Command::Update {
                client_id: new_id,
                presence,
            }) => {
                if presence.is_none() {
                    connection = None;
                    desired = None;
                    dirty = false;
                    set_status(
                        &status,
                        false,
                        "Discord Rich Presence is disabled.".to_owned(),
                    );
                    continue;
                }
                if new_id != client_id {
                    client_id = new_id;
                    connection = None;
                    last_attempt = Instant::now() - Duration::from_secs(60);
                }
                if client_id.trim().is_empty() {
                    connection = None;
                    desired = None;
                    dirty = false;
                    set_status(
                        &status,
                        false,
                        "Enter a Discord application Client ID to enable Rich Presence.".to_owned(),
                    );
                    continue;
                }
                desired = presence;
                dirty = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if !dirty {
            continue;
        }

        if connection.is_none() && last_attempt.elapsed() >= Duration::from_secs(5) {
            last_attempt = Instant::now();
            match connect(&client_id) {
                Ok(conn) => {
                    set_status(
                        &status,
                        true,
                        format!("Connected to Discord as {}.", conn.user),
                    );
                    connection = Some(conn);
                }
                Err(err) => {
                    set_status(&status, false, err);
                    continue;
                }
            }
        }

        if let (Some(conn), Some(presence)) = (connection.as_mut(), desired.as_ref()) {
            match conn.set_activity(presence) {
                Ok(()) => {
                    dirty = false;
                    set_status(
                        &status,
                        true,
                        format!("Rich Presence is active ({}).", conn.user),
                    );
                }
                Err(err) => {
                    set_status(&status, false, err);
                    connection = None;
                    last_attempt = Instant::now();
                }
            }
        }
    }
}

fn set_status(status: &Arc<Mutex<DiscordStatus>>, connected: bool, message: String) {
    if let Ok(mut status) = status.lock() {
        status.connected = connected;
        status.message = message;
    }
}

struct Connection {
    file: File,
    user: String,
    nonce: u64,
}

impl Connection {
    fn set_activity(&mut self, presence: &Presence) -> Result<(), String> {
        let mut activity = Map::new();
        if presence.details.trim().len() >= 2 {
            activity.insert("details".to_owned(), json!(limit(&presence.details, 128)));
        }
        if presence.state.trim().len() >= 2 {
            activity.insert("state".to_owned(), json!(limit(&presence.state, 128)));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        activity.insert("timestamps".to_owned(), json!({ "start": now }));
        activity.insert("instance".to_owned(), json!(false));

        self.nonce += 1;
        let payload = json!({
            "cmd": "SET_ACTIVITY",
            "args": {
                "pid": std::process::id(),
                "activity": Value::Object(activity),
            },
            "nonce": format!("smx-howler-{}", self.nonce),
        });
        write_frame(&mut self.file, OP_FRAME, &payload)?;
        let (opcode, response) = read_frame(&mut self.file)?;
        if opcode != OP_FRAME {
            return Err(format!("Discord sent unexpected IPC opcode {opcode}."));
        }
        if let Some(event) = response.get("evt").and_then(Value::as_str) {
            if event.eq_ignore_ascii_case("ERROR") {
                let message = response
                    .pointer("/data/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Discord rejected the Rich Presence update.");
                return Err(format!("Discord error: {message}"));
            }
        }
        Ok(())
    }
}

fn connect(client_id: &str) -> Result<Connection, String> {
    let mut file = None;
    let mut last_error = String::new();
    for index in 0..10 {
        let path = format!(r"\\.\pipe\discord-ipc-{index}");
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(handle) => {
                file = Some(handle);
                break;
            }
            Err(err) => last_error = err.to_string(),
        }
    }
    let mut file = file.ok_or_else(|| {
        format!(
            "Discord is not running — no IPC pipe found ({last_error}). Start the Discord \
             desktop app and try again."
        )
    })?;

    let handshake = json!({ "v": 1, "client_id": client_id.trim() });
    write_frame(&mut file, OP_HANDSHAKE, &handshake)?;
    let (opcode, response) = read_frame(&mut file)?;
    if opcode != OP_FRAME {
        return Err(format!("Discord handshake failed (opcode {opcode})."));
    }
    let event = response.get("evt").and_then(Value::as_str).unwrap_or("");
    if !event.eq_ignore_ascii_case("READY") {
        let message = response
            .get("data")
            .map(|data| data.to_string())
            .unwrap_or_else(|| response.to_string());
        return Err(format!(
            "Discord rejected the Client ID. Create an application at \
             discord.com/developers/applications and paste its Application ID. [{message}]"
        ));
    }
    let user = response
        .pointer("/data/user/username")
        .and_then(Value::as_str)
        .unwrap_or("your account")
        .to_owned();

    Ok(Connection {
        file,
        user,
        nonce: 0,
    })
}

fn write_frame(file: &mut File, opcode: u32, payload: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(payload)
        .map_err(|err| format!("Could not encode the Discord message: {err}"))?;
    if bytes.len() > MAX_FRAME {
        return Err("Discord message was too large.".to_owned());
    }
    let mut header = [0u8; 8];
    header[..4].copy_from_slice(&opcode.to_le_bytes());
    header[4..].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    file.write_all(&header)
        .and_then(|()| file.write_all(&bytes))
        .and_then(|()| file.flush())
        .map_err(|err| format!("Could not write to Discord: {err}"))
}

fn read_frame(file: &mut File) -> Result<(u32, Value), String> {
    let mut header = [0u8; 8];
    file.read_exact(&mut header)
        .map_err(|err| format!("Could not read from Discord: {err}"))?;
    let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if length > MAX_FRAME {
        return Err("Discord sent an oversized IPC message.".to_owned());
    }
    let mut payload = vec![0u8; length];
    file.read_exact(&mut payload)
        .map_err(|err| format!("Could not read the Discord response: {err}"))?;
    let value: Value = serde_json::from_slice(&payload)
        .map_err(|err| format!("Discord sent invalid JSON: {err}"))?;
    Ok((opcode, value))
}

fn limit(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_owned();
    }
    let mut out: String = trimmed.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Replace `{battery}`, `{dpi}`, `{polling}`, `{device}` and `{state}` in a
/// Rich Presence template. Unknown placeholders are left alone.
pub fn expand(template: &str, values: &[(&str, String)]) -> String {
    let mut out = template.to_owned();
    for (key, value) in values {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    limit(&out, 128)
}
