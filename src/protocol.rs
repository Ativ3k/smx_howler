//! SinoWealth / SMX Howler protocol constants, settings parsing and button actions.
//!
//! The settings and button "blocks" returned by the mouse include the 8-byte
//! report header (`08 cmd 00 00 00 00 00 00`). Offsets below are offsets into
//! that whole block, which is what the rest of the app stores.

use crate::i18n::{tr, tr_format};

pub const VID: u16 = 0x258A;
pub const PID_CABLE: u16 = 0x002E;
pub const PID_RECEIVER: u16 = 0x002F;

pub const SETTINGS_LEN: usize = 154;
pub const BUTTONS_LEN: usize = 88;
pub const REPORT_LEN: usize = 520;
pub const BLOCK_HEADER_LEN: usize = 8;
pub const BUTTON_SLOTS: usize = 20;

/// Settings block captured from a stock SMX Howler (BY-COMBO3) through the
/// receiver before this app wrote anything. Used by the ignored hardware test
/// that restores a mouse after a bad block write.
#[cfg(test)]
pub const RECOVERY_SETTINGS_BLOCK: [u8; SETTINGS_LEN] = [
    0x08, 0x21, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x64, 0x0B, 0x04, 0x11, 0x00, 0x03, 0x00, 0x07,
    0x00, 0x0F, 0x00, 0x1F, 0x00, 0x34, 0x00, 0x3B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x42, 0x00, 0x40, 0xFF, 0x00, 0xFF, 0x42, 0x07, 0xFF, 0x00,
    0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0xFA, 0x00, 0xFA,
    0xFF, 0xFF, 0xFF, 0x02, 0x42, 0x40, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF,
    0xFF, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x42, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0x42, 0x42, 0x02, 0xFF, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0xA5, 0x00,
];

/// Button block captured alongside [`RECOVERY_SETTINGS_BLOCK`].
#[cfg(test)]
pub const RECOVERY_BUTTONS_BLOCK: [u8; BUTTONS_LEN] = [
    0x08, 0x22, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x01, 0x00, 0x00, 0x11, 0x02, 0x00, 0x00,
    0x11, 0x04, 0x00, 0x00, 0x22, 0x40, 0x00, 0x00, 0x22, 0x80, 0x00, 0x00, 0x70, 0x01, 0x01, 0x01,
    0x50, 0x07, 0x00, 0x00, 0x50, 0x01, 0x00, 0x00, 0x50, 0x01, 0x00, 0x00, 0x50, 0x01, 0x00, 0x00,
    0x50, 0x01, 0x00, 0x00, 0x50, 0x01, 0x00, 0x00, 0x50, 0x01, 0x00, 0x00, 0x50, 0x01, 0x00, 0x00,
    0x50, 0x01, 0x00, 0x00, 0x50, 0x01, 0x00, 0x00, 0x50, 0x01, 0x00, 0x00, 0x50, 0x01, 0x00, 0x00,
    0x50, 0x01, 0x00, 0x00, 0x50, 0x01, 0x00, 0x00,
];

pub const CMD_IDENTITY: u8 = 0x01;
pub const CMD_SETTINGS_CABLE: u8 = 0x11;
pub const CMD_BUTTONS_CABLE: u8 = 0x12;
pub const CMD_SETTINGS_RECEIVER: u8 = 0x21;
pub const CMD_BUTTONS_RECEIVER: u8 = 0x22;
pub const CMD_LINK: u8 = 0x80;
pub const CMD_BATTERY: u8 = 0x90;

// Settings block offsets.
pub const OFF_POLLING: usize = 0x0A;
pub const OFF_ACTIVE_STAGE: usize = 0x0B;
pub const OFF_DPI: usize = 0x0D;
pub const OFF_STAGE_COLOR: usize = 0x2D;
pub const OFF_EFFECT: usize = 0x45;
pub const OFF_STREAMING: usize = 0x46;
pub const OFF_STEADY: usize = 0x48;
pub const OFF_BREATHING: usize = 0x4C;
pub const OFF_BREATHING_COUNT: usize = 0x4D;
pub const OFF_BREATHING_COLORS: usize = 0x4E;
pub const OFF_SHARED_SPEED: usize = 0x63;
pub const OFF_STEADY_COLORS: usize = 0x66;
pub const OFF_BLINK_COLOR: usize = 0x85;

/// The DPI table from `Cfg.ini` (`DPISET`), 59 entries.
pub const DPI_VALUES: [u16; DPI_VALUES_LEN] = [
    200, 300, 400, 500, 600, 700, 800, 900, 1000, 1100, 1200, 1300, 1400, 1500, 1600, 1700, 1800,
    1900, 2000, 2100, 2200, 2300, 2400, 2500, 2600, 2700, 2800, 2900, 3000, 3100, 3200, 3300, 3400,
    3500, 3600, 3700, 3800, 3900, 4000, 4100, 4200, 4300, 4400, 4500, 4600, 4700, 4800, 4900, 5000,
    5500, 6000, 6400, 7000, 7500, 8000, 8500, 9000, 9500, 10000,
];
pub const DPI_VALUES_LEN: usize = 59;
pub const DPI_STAGES: usize = 6;
pub const STEADY_COLORS: usize = 7;
pub const BREATHING_COLORS: usize = 7;

pub const EFFECT_COUNT: u8 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bank {
    Receiver,
    Cable,
}

impl Bank {
    pub fn pid(self) -> u16 {
        match self {
            Self::Cable => PID_CABLE,
            Self::Receiver => PID_RECEIVER,
        }
    }

    pub fn settings_cmd(self) -> u8 {
        match self {
            Self::Cable => CMD_SETTINGS_CABLE,
            Self::Receiver => CMD_SETTINGS_RECEIVER,
        }
    }

    pub fn buttons_cmd(self) -> u8 {
        match self {
            Self::Cable => CMD_BUTTONS_CABLE,
            Self::Receiver => CMD_BUTTONS_RECEIVER,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cable => tr("USB cable", "Kabel USB"),
            Self::Receiver => tr("2.4 GHz receiver", "Odbiornik 2.4 GHz"),
        }
    }

    pub fn config_key(self) -> &'static str {
        match self {
            Self::Cable => "cable",
            Self::Receiver => "receiver",
        }
    }

    pub fn from_config(value: &str) -> Self {
        if value.eq_ignore_ascii_case("cable") {
            Self::Cable
        } else {
            Self::Receiver
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DeviceStatus {
    pub connected: bool,
    pub product: String,
    pub identity: String,
    pub via: String,
    pub percent: Option<u8>,
    pub charging: bool,
    pub full: bool,
    pub asleep: bool,
    pub link: Option<bool>,
    pub error: Option<String>,
}

impl DeviceStatus {
    pub fn battery_text(&self) -> String {
        match self.percent {
            Some(p) => format!("{p}%"),
            None if self.charging => tr("Charging", "Ładowanie").to_owned(),
            None => tr("Unknown", "Nieznany").to_owned(),
        }
    }

    pub fn state_text(&self) -> &'static str {
        if self.full {
            tr("Fully charged", "W pełni naładowana")
        } else if self.charging {
            tr("Charging", "Ładowanie")
        } else if self.asleep {
            tr("Mouse asleep", "Mysz uśpiona")
        } else if self.percent.is_some() {
            tr("On battery", "Na baterii")
        } else if self.connected {
            tr("Battery level unavailable", "Poziom baterii niedostępny")
        } else {
            tr("Not connected", "Nie połączono")
        }
    }
}

pub fn polling_code(hz: u16) -> u8 {
    match hz {
        125 => 1,
        250 => 2,
        500 => 3,
        _ => 4,
    }
}

pub fn polling_hz(code: u8) -> u16 {
    match code {
        1 => 125,
        2 => 250,
        3 => 500,
        _ => 1000,
    }
}

pub fn polling_label(hz: u16) -> &'static str {
    match hz {
        125 => "125 Hz",
        250 => "250 Hz",
        500 => "500 Hz",
        _ => "1000 Hz",
    }
}

pub fn polling_ms(hz: u16) -> &'static str {
    match hz {
        125 => "8 ms",
        250 => "4 ms",
        500 => "2 ms",
        _ => "1 ms",
    }
}

pub fn effect_name(code: u8) -> &'static str {
    match code {
        0 => tr("Off", "Wyłączone"),
        1 => tr("Colorful streaming", "Tęczowy przepływ"),
        2 => tr("Steady", "Stały"),
        3 => tr("Breathing", "Oddychanie"),
        4 => tr("Tail", "Ogon komety"),
        5 => tr("Neon", "Neon"),
        6 => tr("Colorful steady", "Tęczowy stały"),
        7 => tr("Blinking", "Miganie"),
        8 => tr("Star blink", "Migoczące gwiazdy"),
        9 => tr("Wave", "Fala"),
        _ => tr("Unknown", "Nieznany"),
    }
}

pub fn effect_description(code: u8) -> &'static str {
    match code {
        0 => tr(
            "All body lighting off. The scroll-wheel DPI indicator can stay lit.",
            "Całe podświetlenie korpusu wyłączone. Wskaźnik DPI na rolce może pozostać podświetlony.",
        ),
        1 => tr(
            "A rainbow streaming around the body. Brightness and speed.",
            "Tęcza przepływająca wokół korpusu. Jasność i prędkość.",
        ),
        2 => tr(
            "One fixed colour. Brightness and colour slot.",
            "Jeden stały kolor. Jasność i slot koloru.",
        ),
        3 => tr(
            "A slow pulse through the palette. Brightness, speed and colours.",
            "Powolne pulsowanie przez paletę. Jasność, prędkość i kolory.",
        ),
        4 => tr(
            "A comet tail chasing around the shell. Speed.",
            "Ogon komety krążący wokół obudowy. Prędkość.",
        ),
        5 => tr(
            "Neon-style flowing lines. Speed.",
            "Neonowe, płynące linie. Prędkość.",
        ),
        6 => tr(
            "A fixed multicolour gradient. Speed.",
            "Stały wielokolorowy gradient. Prędkość.",
        ),
        7 => tr(
            "One colour blinking on and off.",
            "Jeden kolor migający włącz/wyłącz.",
        ),
        8 => tr(
            "Random twinkling stars. Speed.",
            "Losowo migoczące gwiazdy. Prędkość.",
        ),
        9 => tr(
            "A moving wave of colour. Speed.",
            "Przesuwająca się fala kolorów. Prędkość.",
        ),
        _ => "",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DpiStage {
    /// 0-based index into [`DPI_VALUES`].
    pub dpi_index: u8,
    pub color: [u8; 3],
}

impl DpiStage {
    pub fn dpi(&self) -> u16 {
        DPI_VALUES[self.dpi_index as usize % DPI_VALUES.len()]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsDraft {
    pub polling_hz: u16,
    pub active_stage: u8,
    pub stages: [DpiStage; DPI_STAGES],
    pub effect: u8,
    pub streaming_brightness: u8,
    pub streaming_speed: u8,
    pub steady_brightness: u8,
    pub steady_slot: u8,
    pub steady_colors: [[u8; 3]; STEADY_COLORS],
    pub breathing_brightness: u8,
    pub breathing_speed: u8,
    pub breathing_colors: [[u8; 3]; BREATHING_COLORS],
    pub shared_speed: u8,
    pub blink_color: [u8; 3],
}

impl Default for SettingsDraft {
    fn default() -> Self {
        Self {
            polling_hz: 1000,
            active_stage: 1,
            stages: [
                DpiStage {
                    dpi_index: 2,
                    color: [255, 0, 0],
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
            ],
            effect: 1,
            streaming_brightness: 4,
            streaming_speed: 2,
            steady_brightness: 4,
            steady_slot: 0,
            steady_colors: [
                [255, 0, 0],
                [0, 255, 0],
                [0, 0, 255],
                [255, 255, 0],
                [0, 255, 255],
                [255, 255, 255],
                [0, 0, 0],
            ],
            breathing_brightness: 4,
            breathing_speed: 2,
            breathing_colors: [
                [255, 0, 0],
                [0, 255, 0],
                [0, 0, 255],
                [0, 255, 255],
                [255, 255, 0],
                [255, 0, 255],
                [255, 255, 255],
            ],
            shared_speed: 2,
            blink_color: [0, 255, 0],
        }
    }
}

impl SettingsDraft {
    pub fn from_block(block: &[u8]) -> Option<Self> {
        if block.len() < SETTINGS_LEN {
            return None;
        }
        let mut draft = Self::default();

        draft.polling_hz = polling_hz(block[OFF_POLLING] & 0x0F);
        draft.active_stage = clamp_stage(block[OFF_ACTIVE_STAGE] >> 4);
        for (i, stage) in draft.stages.iter_mut().enumerate() {
            let code = block[OFF_DPI + i * 2];
            stage.dpi_index = code.saturating_sub(1).min(DPI_VALUES_LEN as u8 - 1);
            let off = OFF_STAGE_COLOR + i * 3;
            stage.color = [block[off], block[off + 1], block[off + 2]];
        }

        draft.effect = block[OFF_EFFECT].min(EFFECT_COUNT - 1);
        let stream = block[OFF_STREAMING];
        draft.streaming_brightness = clamp_nibble(stream >> 4);
        draft.streaming_speed = clamp_nibble(stream & 0x0F);
        let steady = block[OFF_STEADY];
        draft.steady_brightness = clamp_nibble(steady >> 4);
        draft.steady_slot = (steady & 0x0F).min(STEADY_COLORS as u8 - 1);
        let breath = block[OFF_BREATHING];
        draft.breathing_brightness = clamp_nibble(breath >> 4);
        draft.breathing_speed = clamp_nibble(breath & 0x0F);
        for (i, color) in draft.breathing_colors.iter_mut().enumerate() {
            let off = OFF_BREATHING_COLORS + i * 3;
            *color = [block[off], block[off + 1], block[off + 2]];
        }
        for (i, color) in draft.steady_colors.iter_mut().enumerate() {
            let off = OFF_STEADY_COLORS + i * 3;
            *color = [block[off], block[off + 1], block[off + 2]];
        }
        draft.shared_speed = clamp_nibble(block[OFF_SHARED_SPEED] & 0x0F);
        draft.blink_color = [
            block[OFF_BLINK_COLOR],
            block[OFF_BLINK_COLOR + 1],
            block[OFF_BLINK_COLOR + 2],
        ];
        Some(draft)
    }

    /// Apply this draft to a block that was freshly read from the mouse, so
    /// every byte we do not understand is preserved exactly.
    pub fn apply_to(&self, block: &mut [u8]) {
        if block.len() < SETTINGS_LEN {
            return;
        }

        block[OFF_POLLING] = (block[OFF_POLLING] & 0xF0) | (polling_code(self.polling_hz) & 0x0F);
        block[OFF_ACTIVE_STAGE] =
            (clamp_stage(self.active_stage) << 4) | (block[OFF_ACTIVE_STAGE] & 0x0F);
        for (i, stage) in self.stages.iter().enumerate() {
            block[OFF_DPI + i * 2] = stage.dpi_index.min(DPI_VALUES_LEN as u8 - 1) + 1;
            block[OFF_DPI + i * 2 + 1] = 0;
            let off = OFF_STAGE_COLOR + i * 3;
            block[off..off + 3].copy_from_slice(&stage.color);
        }

        block[OFF_EFFECT] = self.effect.min(EFFECT_COUNT - 1);
        block[OFF_STREAMING] =
            (clamp_nibble(self.streaming_brightness) << 4) | clamp_nibble(self.streaming_speed);
        block[OFF_STEADY] = (clamp_nibble(self.steady_brightness) << 4)
            | (self.steady_slot.min(STEADY_COLORS as u8 - 1) & 0x0F);
        block[OFF_BREATHING] =
            (clamp_nibble(self.breathing_brightness) << 4) | clamp_nibble(self.breathing_speed);
        block[OFF_BREATHING_COUNT] = BREATHING_COLORS as u8;
        for (i, color) in self.breathing_colors.iter().enumerate() {
            let off = OFF_BREATHING_COLORS + i * 3;
            block[off..off + 3].copy_from_slice(color);
        }
        block[OFF_SHARED_SPEED] =
            (block[OFF_SHARED_SPEED] & 0xF0) | (clamp_nibble(self.shared_speed) & 0x0F);
        for (i, color) in self.steady_colors.iter().enumerate() {
            let off = OFF_STEADY_COLORS + i * 3;
            block[off..off + 3].copy_from_slice(color);
        }
        block[OFF_BLINK_COLOR..OFF_BLINK_COLOR + 3].copy_from_slice(&self.blink_color);
    }

    pub fn active_dpi(&self) -> u16 {
        self.stages[(clamp_stage(self.active_stage) - 1) as usize].dpi()
    }
}

fn clamp_stage(stage: u8) -> u8 {
    stage.clamp(1, DPI_STAGES as u8)
}

fn clamp_nibble(value: u8) -> u8 {
    value.min(4)
}

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02X}");
    }
    out
}

pub fn pretty_hex(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in bytes.chunks(16).enumerate() {
        use std::fmt::Write as _;
        let _ = write!(out, "{:04X}  ", i * 16);
        for b in chunk {
            let _ = write!(out, "{b:02X} ");
        }
        let _ = writeln!(out);
    }
    out
}

// ---------------------------------------------------------------------------
// Button actions
// ---------------------------------------------------------------------------

pub const MOD_CTRL: u8 = 0x01;
pub const MOD_SHIFT: u8 = 0x02;
pub const MOD_ALT: u8 = 0x04;
pub const MOD_WIN: u8 = 0x08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonAction {
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
    Keyboard { modifiers: u8, usage: u8 },
    Macro { number: u8, mode: u8, count: u8 },
    Custom([u8; 4]),
}

impl ButtonAction {
    pub fn decode(bytes: [u8; 4]) -> Self {
        let tail_zero = bytes[2] == 0 && bytes[3] == 0;
        match bytes[0] {
            0x11 if tail_zero => match bytes[1] {
                0x01 => Self::Left,
                0x02 => Self::Right,
                0x04 => Self::Middle,
                0x08 => Self::Back,
                0x10 => Self::Forward,
                _ => Self::Custom(bytes),
            },
            0x22 if tail_zero => match bytes[1] {
                0x40 => Self::VolumeUp,
                0x80 => Self::VolumeDown,
                _ => Self::Custom(bytes),
            },
            0x41 if tail_zero => match bytes[1] {
                0x01 => Self::DpiUp,
                0x02 => Self::DpiDown,
                _ => Self::Custom(bytes),
            },
            0x50 if tail_zero => match bytes[1] {
                0x01 => Self::Disabled,
                0x02 => Self::LightingToggle,
                0x07 => Self::DpiLoop,
                _ => Self::Custom(bytes),
            },
            0x21 if bytes[3] == 0 => Self::Keyboard {
                modifiers: bytes[1],
                usage: bytes[2],
            },
            0x70 => Self::Macro {
                number: bytes[1],
                mode: bytes[2],
                count: bytes[3],
            },
            _ => Self::Custom(bytes),
        }
    }

    pub fn encode(self) -> [u8; 4] {
        match self {
            Self::Left => [0x11, 0x01, 0, 0],
            Self::Right => [0x11, 0x02, 0, 0],
            Self::Middle => [0x11, 0x04, 0, 0],
            Self::Back => [0x11, 0x08, 0, 0],
            Self::Forward => [0x11, 0x10, 0, 0],
            Self::VolumeUp => [0x22, 0x40, 0, 0],
            Self::VolumeDown => [0x22, 0x80, 0, 0],
            Self::DpiUp => [0x41, 0x01, 0, 0],
            Self::DpiDown => [0x41, 0x02, 0, 0],
            Self::DpiLoop => [0x50, 0x07, 0, 0],
            Self::LightingToggle => [0x50, 0x02, 0, 0],
            Self::Disabled => [0x50, 0x01, 0, 0],
            Self::Keyboard { modifiers, usage } => [0x21, modifiers, usage, 0],
            Self::Macro {
                number,
                mode,
                count,
            } => [0x70, number, mode, count],
            Self::Custom(bytes) => bytes,
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Left => tr("Left click", "Lewy klik").to_owned(),
            Self::Right => tr("Right click", "Prawy klik").to_owned(),
            Self::Middle => tr("Middle click", "Środkowy klik").to_owned(),
            Self::Back => tr("Back", "Wstecz").to_owned(),
            Self::Forward => tr("Forward", "Dalej").to_owned(),
            Self::VolumeUp => tr("Volume up", "Głośniej").to_owned(),
            Self::VolumeDown => tr("Volume down", "Ciszej").to_owned(),
            Self::DpiUp => tr("DPI up", "DPI w górę").to_owned(),
            Self::DpiDown => tr("DPI down", "DPI w dół").to_owned(),
            Self::DpiLoop => tr("DPI loop", "Pętla DPI").to_owned(),
            Self::LightingToggle => tr("Lighting on/off", "Podświetlenie wł./wył.").to_owned(),
            Self::Disabled => tr("Disabled", "Wyłączone").to_owned(),
            Self::Keyboard { modifiers, usage } => {
                let mut text = String::new();
                if modifiers & MOD_CTRL != 0 {
                    text.push_str("Ctrl+");
                }
                if modifiers & MOD_SHIFT != 0 {
                    text.push_str("Shift+");
                }
                if modifiers & MOD_ALT != 0 {
                    text.push_str("Alt+");
                }
                if modifiers & MOD_WIN != 0 {
                    text.push_str("Win+");
                }
                text.push_str(hid_usage_name(usage));
                text
            }
            Self::Macro {
                number,
                mode,
                count,
            } => tr_format!(
                "Macro {} (mode {}, {} event(s))",
                "Makro {} (tryb {}, zdarzenia: {})",
                number,
                mode,
                count
            ),
            Self::Custom(bytes) => tr_format!(
                "Custom {:02X} {:02X} {:02X} {:02X}",
                "Własne {:02X} {:02X} {:02X} {:02X}",
                bytes[0],
                bytes[1],
                bytes[2],
                bytes[3]
            ),
        }
    }
}

pub fn hid_usage_name(usage: u8) -> &'static str {
    match usage {
        0x04..=0x1D => LETTER_NAMES[(usage - 0x04) as usize],
        0x1E..=0x27 => DIGIT_NAMES[(usage - 0x1E) as usize],
        0x28 => "Enter",
        0x29 => "Esc",
        0x2A => "Backspace",
        0x2B => "Tab",
        0x2C => tr("Space", "Spacja"),
        0x2D => "-",
        0x2E => "=",
        0x2F => "[",
        0x30 => "]",
        0x31 => "\\",
        0x32 => "Non-US #",
        0x33 => ";",
        0x34 => "'",
        0x35 => "`",
        0x36 => ",",
        0x37 => ".",
        0x38 => "/",
        0x39 => "Caps Lock",
        0x3A => "F1",
        0x3B => "F2",
        0x3C => "F3",
        0x3D => "F4",
        0x3E => "F5",
        0x3F => "F6",
        0x40 => "F7",
        0x41 => "F8",
        0x42 => "F9",
        0x43 => "F10",
        0x44 => "F11",
        0x45 => "F12",
        0x46 => "Print Screen",
        0x47 => "Scroll Lock",
        0x48 => tr("Pause", "Pauza"),
        0x49 => "Insert",
        0x4A => "Home",
        0x4B => "Page Up",
        0x4C => "Delete",
        0x4D => "End",
        0x4E => "Page Down",
        0x4F => tr("Right arrow", "Strzałka w prawo"),
        0x50 => tr("Left arrow", "Strzałka w lewo"),
        0x51 => tr("Down arrow", "Strzałka w dół"),
        0x52 => tr("Up arrow", "Strzałka w górę"),
        0x53 => "Num Lock",
        0x54 => "Numpad /",
        0x55 => "Numpad *",
        0x56 => "Numpad -",
        0x57 => "Numpad +",
        0x58 => "Numpad Enter",
        0x59..=0x61 => NUM_KEY_NAMES[(usage - 0x59) as usize],
        0x62 => "Numpad 0",
        0x63 => "Numpad .",
        0x64 => "Non-US \\",
        0x65 => tr("Application", "Aplikacja"),
        0x66 => tr("Power", "Zasilanie"),
        0x67 => "Numpad =",
        0x68 => "F13",
        0x69 => "F14",
        0x6A => "F15",
        0x6B => "F16",
        0x6C => "F17",
        0x6D => "F18",
        0x6E => "F19",
        0x6F => "F20",
        0x70 => "F21",
        0x71 => "F22",
        0x72 => "F23",
        0x73 => "F24",
        _ => tr("Unknown key", "Nieznany klawisz"),
    }
}

const LETTER_NAMES: [&str; 26] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z",
];
const DIGIT_NAMES: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
const NUM_KEY_NAMES: [&str; 9] = [
    "Numpad 1", "Numpad 2", "Numpad 3", "Numpad 4", "Numpad 5", "Numpad 6", "Numpad 7", "Numpad 8",
    "Numpad 9",
];

pub const FACTORY_BUTTONS: [[u8; 4]; BUTTON_SLOTS] = [
    [0x11, 0x01, 0, 0],
    [0x11, 0x02, 0, 0],
    [0x11, 0x04, 0, 0],
    [0x22, 0x40, 0, 0],
    [0x22, 0x80, 0, 0],
    [0x70, 0x01, 0x01, 0x01],
    [0x50, 0x07, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
    [0x50, 0x01, 0, 0],
];

pub fn slot_label(index: usize) -> String {
    match index {
        0..=6 => tr_format!("Button {}", "Przycisk {}", index + 1),
        n => tr_format!("Slot {}", "Slot {}", n + 1),
    }
}

pub fn parse_buttons(block: &[u8]) -> Option<[[u8; 4]; BUTTON_SLOTS]> {
    if block.len() < BUTTONS_LEN {
        return None;
    }
    let mut out = [[0u8; 4]; BUTTON_SLOTS];
    for (i, slot) in out.iter_mut().enumerate() {
        let off = BLOCK_HEADER_LEN + i * 4;
        *slot = [block[off], block[off + 1], block[off + 2], block[off + 3]];
    }
    Some(out)
}

pub fn apply_buttons(block: &mut [u8], slots: &[[u8; 4]; BUTTON_SLOTS]) {
    if block.len() < BUTTONS_LEN {
        return;
    }
    for (i, slot) in slots.iter().enumerate() {
        let off = BLOCK_HEADER_LEN + i * 4;
        block[off..off + 4].copy_from_slice(slot);
    }
}

pub fn build_short(cmd: u8) -> [u8; 8] {
    let mut report = [0u8; 8];
    report[0] = 5;
    report[1] = cmd;
    report
}

pub fn build_block_request(cmd: u8) -> Vec<u8> {
    let mut report = vec![0u8; REPORT_LEN];
    report[0] = 8;
    report[1] = cmd;
    report
}

pub fn build_block_write(cmd: u8, block: &[u8], button_trailer: bool) -> Vec<u8> {
    let mut report = vec![0u8; REPORT_LEN];
    let len = block.len().min(REPORT_LEN);
    report[..len].copy_from_slice(&block[..len]);
    report[0] = 8;
    report[1] = cmd;
    report[2] = 0;
    report[3] = len.saturating_sub(BLOCK_HEADER_LEN) as u8;
    if button_trailer && len < REPORT_LEN {
        report[len] = 0xA5;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_codes_match_factory_stages() {
        let codes = [3u8, 7, 15, 31, 52, 59];
        let expected = [400u16, 800, 1600, 3200, 6400, 10000];
        for (code, dpi) in codes.into_iter().zip(expected) {
            assert_eq!(DPI_VALUES[(code - 1) as usize], dpi);
        }
    }

    #[test]
    fn button_actions_round_trip() {
        let cases = [
            ButtonAction::Left,
            ButtonAction::Right,
            ButtonAction::Middle,
            ButtonAction::VolumeUp,
            ButtonAction::VolumeDown,
            ButtonAction::DpiUp,
            ButtonAction::DpiDown,
            ButtonAction::DpiLoop,
            ButtonAction::LightingToggle,
            ButtonAction::Disabled,
            ButtonAction::Keyboard {
                modifiers: 3,
                usage: 0x10,
            },
            ButtonAction::Macro {
                number: 1,
                mode: 1,
                count: 1,
            },
            ButtonAction::Custom([0x12, 0x34, 0x56, 0x78]),
        ];
        for case in cases {
            assert_eq!(ButtonAction::decode(case.encode()), case);
        }
    }

    #[test]
    fn write_keeps_block_at_original_offsets() {
        let mut block = vec![0u8; SETTINGS_LEN];
        block[0] = 8;
        block[1] = CMD_SETTINGS_RECEIVER;
        block[8] = 0x64;
        block[9] = 0x0B;
        block[10] = 0x04;
        block[0x0D] = 3;
        let report = build_block_write(CMD_SETTINGS_RECEIVER, &block, false);
        assert_eq!(report[0], 8);
        assert_eq!(report[1], CMD_SETTINGS_RECEIVER);
        assert_eq!(report[2], 0);
        assert_eq!(report[3], (SETTINGS_LEN - 8) as u8);
        assert_eq!(&report[8..12], &[0x64, 0x0B, 0x04, 0x00]);
        assert_eq!(report[0x0D], 3);
        assert!(report[SETTINGS_LEN..].iter().all(|&b| b == 0));
    }

    #[test]
    fn button_write_appends_trailer() {
        let mut block = vec![0u8; BUTTONS_LEN];
        block[8] = 0x11;
        block[9] = 0x01;
        let report = build_block_write(CMD_BUTTONS_RECEIVER, &block, true);
        assert_eq!(report[3], (BUTTONS_LEN - 8) as u8);
        assert_eq!(report[8], 0x11);
        assert_eq!(report[9], 0x01);
        assert_eq!(report[BUTTONS_LEN], 0xA5);
    }
}
