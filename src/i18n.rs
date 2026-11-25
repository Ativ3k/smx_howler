use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Pl,
}

static CURRENT: AtomicU8 = AtomicU8::new(Lang::En as u8);

impl Lang {
    pub fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "pl" | "pl-pl" | "polski" | "polish" => Self::Pl,
            _ => Self::En,
        }
    }

    pub fn config_key(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Pl => "pl",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::En => "EN",
            Self::Pl => "PL",
        }
    }

    pub fn set(self) {
        CURRENT.store(self as u8, Ordering::Relaxed);
    }

    pub fn current() -> Self {
        if CURRENT.load(Ordering::Relaxed) == Self::Pl as u8 {
            Self::Pl
        } else {
            Self::En
        }
    }
}

/// Returns the active translation for a pair of English and Polish strings.
pub fn tr(en: &'static str, pl: &'static str) -> &'static str {
    match Lang::current() {
        Lang::En => en,
        Lang::Pl => pl,
    }
}

/// Formats the active translation of a string with the given arguments.
macro_rules! tr_format {
    ($en:literal, $pl:literal $(, $($arg:tt)*)?) => {
        match $crate::i18n::Lang::current() {
            $crate::i18n::Lang::En => format!($en $(, $($arg)*)?),
            $crate::i18n::Lang::Pl => format!($pl $(, $($arg)*)?),
        }
    };
}
pub(crate) use tr_format;
