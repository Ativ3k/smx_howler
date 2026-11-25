use std::ffi::CString;
use std::thread;
use std::time::Duration;

use hidapi::{DeviceInfo, HidApi, HidDevice};

use crate::i18n::{tr, tr_format};
use crate::protocol::*;

const SHORT_DELAY: Duration = Duration::from_millis(100);
const BLOCK_DELAY: Duration = Duration::from_millis(160);
const RETRIES: usize = 3;

#[derive(Clone, Debug)]
pub struct HidEntry {
    pub pid: u16,
    pub product: String,
    pub manufacturer: String,
    pub usage_page: u16,
    pub usage: u16,
    pub interface: i32,
    pub path: String,
    pub serial: String,
}

impl HidEntry {
    pub fn is_vendor(&self) -> bool {
        self.usage_page == 0xFF00 && self.usage == 0x01
    }
}

pub struct SmxDevice {
    config: Option<HidDevice>,
    status: HidDevice,
    pub pid: u16,
    pub product: String,
    pub path: String,
}

impl SmxDevice {
    /// Open the two vendor collections used by one PID. Most Sinowealth
    /// receivers split the configuration blocks (feature report 8) and the
    /// battery/status replies (feature report 5) across different HID
    /// collections, so both are probed separately.
    pub fn open(pid: u16) -> Result<Self, String> {
        let api = HidApi::new().map_err(|err| {
            tr_format!(
                "HID initialisation failed: {err}",
                "Inicjalizacja HID nie powiodła się: {err}"
            )
        })?;
        let paths = candidate_paths(&api, pid);
        if paths.is_empty() {
            return Err(tr_format!(
                "SMX Howler {} node not found (VID 258A, PID {pid:04X}).",
                "Nie znaleziono węzła SMX Howler {} (VID 258A, PID {pid:04X}).",
                pid_name(pid)
            ));
        }

        let config_cmd = if pid == PID_CABLE {
            CMD_SETTINGS_CABLE
        } else {
            CMD_SETTINGS_RECEIVER
        };
        let status_cmd = if pid == PID_CABLE {
            CMD_BATTERY
        } else {
            CMD_LINK
        };

        let mut status = None;
        let mut status_errors = Vec::new();
        for (path, product) in &paths {
            match api.open_path(path) {
                Ok(dev) => match query_short(&dev, status_cmd) {
                    Ok(_) => {
                        status = Some((dev, product.clone(), path.to_string_lossy().to_string()));
                        break;
                    }
                    Err(err) => status_errors.push(format!("{err}")),
                },
                Err(err) => status_errors.push(tr_format!(
                    "open failed: {err}",
                    "otwarcie nie powiodło się: {err}"
                )),
            }
        }

        let Some((status, product, status_path)) = status else {
            return Err(tr_format!("The SMX Howler {} battery/status collection did not answer. \
                     Move the mouse to wake it, close OemDrv.exe, and reconnect the device.{}", "Kolekcja baterii/statusu SMX Howler {} nie odpowiedziała. \
                     Porusz myszą, aby ją obudzić, zamknij OemDrv.exe i podłącz urządzenie ponownie.{}", pid_name(pid),
                error_suffix(&status_errors)));
        };

        let mut config = None;
        let mut config_errors = Vec::new();
        for (path, _product) in &paths {
            match api.open_path(path) {
                Ok(dev) => {
                    if block_answers(&dev, config_cmd) {
                        config = Some((dev, path.to_string_lossy().to_string()));
                        break;
                    }
                    config_errors.push(
                        tr(
                            "no settings-block reply",
                            "brak odpowiedzi z blokiem ustawień",
                        )
                        .to_owned(),
                    );
                }
                Err(err) => config_errors.push(tr_format!(
                    "open failed: {err}",
                    "otwarcie nie powiodło się: {err}"
                )),
            }
        }

        let (config, path) = match config {
            Some((dev, path)) => (Some(dev), path),
            None => (None, status_path),
        };

        Ok(Self {
            config,
            status,
            pid,
            product,
            path,
        })
    }

    /// Retry opening the settings channel if the device was asleep when the
    /// app first connected.
    pub fn ensure_config(&mut self) -> Result<(), String> {
        if self.config.is_some() {
            return Ok(());
        }
        let api = HidApi::new().map_err(|err| {
            tr_format!(
                "HID initialisation failed: {err}",
                "Inicjalizacja HID nie powiodła się: {err}"
            )
        })?;
        let cmd = if self.pid == PID_CABLE {
            CMD_SETTINGS_CABLE
        } else {
            CMD_SETTINGS_RECEIVER
        };
        for (path, _product) in candidate_paths(&api, self.pid) {
            if let Ok(dev) = api.open_path(&path) {
                if block_answers(&dev, cmd) {
                    self.path = path.to_string_lossy().to_string();
                    self.config = Some(dev);
                    return Ok(());
                }
            }
        }
        Err(tr(
            "The settings channel is not answering. Move the mouse to wake it, \
             close OemDrv.exe, then click Reload settings.",
            "Kanał ustawień nie odpowiada. Porusz myszą, aby ją obudzić, \
             zamknij OemDrv.exe i kliknij Wczytaj ustawienia.",
        )
        .to_owned())
    }

    pub fn short(&self, cmd: u8) -> Result<[u8; 8], String> {
        query_short(&self.status, cmd)
    }

    pub fn identity(&self) -> Result<String, String> {
        let reply = self.short(CMD_IDENTITY)?;
        let text: String = reply[2..6]
            .iter()
            .map(|&b| {
                if (0x20..0x7F).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        Ok(text)
    }

    /// Read the settings (154 bytes) or buttons (88 bytes) block. The result
    /// includes the 8-byte report prefix, exactly like the vendor traffic.
    pub fn read_block(&self, cmd: u8) -> Result<Vec<u8>, String> {
        let expected_len = match cmd {
            CMD_SETTINGS_CABLE | CMD_SETTINGS_RECEIVER => SETTINGS_LEN,
            CMD_BUTTONS_CABLE | CMD_BUTTONS_RECEIVER => BUTTONS_LEN,
            _ => {
                return Err(tr_format!(
                    "not a block command: {cmd:02X}",
                    "to nie jest polecenie bloku: {cmd:02X}"
                ))
            }
        };
        let dev = self.config_device()?;
        let request = build_block_request(cmd);
        dev.send_feature_report(&request).map_err(|err| {
            tr_format!(
                "block request {cmd:02X} failed: {err}",
                "żądanie bloku {cmd:02X} nie powiodło się: {err}"
            )
        })?;
        thread::sleep(BLOCK_DELAY);

        let mut last_error = String::new();
        for attempt in 0..RETRIES {
            let mut response = vec![0u8; REPORT_LEN];
            response[0] = 8;
            let read = dev.get_feature_report(&mut response).map_err(|err| {
                tr_format!(
                    "block reply {cmd:02X} failed: {err}",
                    "odpowiedź bloku {cmd:02X} nie powiodła się: {err}"
                )
            })?;
            if read >= 8 && response[0] == 8 && response[1] == cmd {
                response.truncate(expected_len);
                return Ok(response);
            }
            last_error = tr_format!(
                "block reply {cmd:02X} invalid on attempt {} ({} bytes: {})",
                "niepoprawna odpowiedź bloku {cmd:02X} w próbie {} ({} bajtów: {})",
                attempt + 1,
                read,
                hex(&response[..read.min(16)])
            );
            thread::sleep(Duration::from_millis(120 * (attempt as u64 + 1)));
        }
        Err(last_error)
    }

    fn config_device(&self) -> Result<&HidDevice, String> {
        self.config.as_ref().ok_or_else(|| {
            tr(
                "The settings channel is unavailable. Move the mouse to wake it, \
                 close OemDrv.exe, then click Reload settings.",
                "Kanał ustawień jest niedostępny. Porusz myszą, aby ją obudzić, \
                 zamknij OemDrv.exe i kliknij Wczytaj ustawienia.",
            )
            .to_owned()
        })
    }

    pub fn write_report(&self, report: &[u8]) -> Result<(), String> {
        self.config_device()?
            .send_feature_report(report)
            .map_err(|err| tr_format!("write failed: {err}", "zapis nie powiódł się: {err}"))?;
        thread::sleep(Duration::from_millis(220));
        Ok(())
    }

    /// Re-read both blocks, write the settings block and then the button
    /// block, matching the vendor application's paired write.
    pub fn write_pair(&self, bank: Bank, settings: &[u8], buttons: &[u8]) -> Result<(), String> {
        self.write_pair_impl(bank, settings, buttons, true)
    }

    /// Diagnostic path: write both blocks without waiting for the receiver
    /// link flag. The mouse accepts the reports while asleep, but may undo
    /// them when it next connects, so the GUI should keep using
    /// [`SmxDevice::write_pair`].
    pub fn write_pair_unchecked(
        &self,
        bank: Bank,
        settings: &[u8],
        buttons: &[u8],
    ) -> Result<(), String> {
        self.write_pair_impl(bank, settings, buttons, false)
    }

    fn write_pair_impl(
        &self,
        bank: Bank,
        settings: &[u8],
        buttons: &[u8],
        require_link: bool,
    ) -> Result<(), String> {
        if require_link && bank == Bank::Receiver {
            let link = self.short(CMD_LINK)?;
            if link[2] == 0 {
                return Err(tr(
                    "The receiver reports the mouse as asleep/off. Move the mouse, then try again.",
                    "Odbiornik zgłasza mysz jako uśpioną/wyłączoną. Porusz myszą i spróbuj ponownie.",
                )
                .to_owned());
            }
        }
        let settings_report = build_block_write(bank.settings_cmd(), settings, false);
        self.write_report(&settings_report)?;
        let button_report = build_block_write(bank.buttons_cmd(), buttons, true);
        self.write_report(&button_report)?;
        Ok(())
    }
}

fn pid_name(pid: u16) -> &'static str {
    match pid {
        PID_CABLE => tr("cable", "kabel"),
        PID_RECEIVER => tr("receiver", "odbiornik"),
        _ => tr("device", "urządzenie"),
    }
}

fn error_suffix(errors: &[String]) -> String {
    if errors.is_empty() {
        String::new()
    } else {
        tr_format!(
            "\nLast error: {}",
            "\nOstatni błąd: {}",
            errors.last().unwrap()
        )
    }
}

fn query_short(dev: &HidDevice, cmd: u8) -> Result<[u8; 8], String> {
    let request = build_short(cmd);
    dev.send_feature_report(&request).map_err(|err| {
        tr_format!(
            "short command {cmd:02X} failed: {err}",
            "krótkie polecenie {cmd:02X} nie powiodło się: {err}"
        )
    })?;
    thread::sleep(SHORT_DELAY);
    let mut response = [0u8; 8];
    response[0] = 5;
    let read = dev.get_feature_report(&mut response).map_err(|err| {
        tr_format!(
            "short reply {cmd:02X} failed: {err}",
            "krótka odpowiedź {cmd:02X} nie powiodła się: {err}"
        )
    })?;
    if read < 8 || response[0] != 5 || response[1] != cmd {
        return Err(tr_format!(
            "short reply {cmd:02X} was invalid (got {} bytes: {})",
            "krótka odpowiedź {cmd:02X} była niepoprawna (otrzymano {} bajtów: {})",
            read,
            hex(&response[..read.min(response.len())])
        ));
    }
    Ok(response)
}

fn block_answers(dev: &HidDevice, cmd: u8) -> bool {
    let request = build_block_request(cmd);
    if dev.send_feature_report(&request).is_err() {
        return false;
    }
    thread::sleep(BLOCK_DELAY);
    for attempt in 0..RETRIES {
        let mut response = vec![0u8; REPORT_LEN];
        response[0] = 8;
        match dev.get_feature_report(&mut response) {
            Ok(read) if read >= 8 && response[0] == 8 && response[1] == cmd => return true,
            _ => thread::sleep(Duration::from_millis(120 * (attempt as u64 + 1))),
        }
    }
    false
}

fn candidate_paths(api: &HidApi, pid: u16) -> Vec<(CString, String)> {
    let mut vendor = Vec::new();
    let mut fallback = Vec::new();
    for info in api.device_list() {
        if info.vendor_id() != VID || info.product_id() != pid {
            continue;
        }
        if info.interface_number() != 1 {
            continue;
        }
        let product = info
            .product_string()
            .unwrap_or("SMX Howler")
            .trim()
            .to_owned();
        let path = info.path().to_owned();
        if info.usage_page() == 0xFF00 && info.usage() == 0x01 {
            vendor.push((path, product));
        } else {
            fallback.push((path, product));
        }
    }
    vendor.extend(fallback);
    vendor
}

pub fn list_smx_devices() -> Result<Vec<HidEntry>, String> {
    let api = HidApi::new().map_err(|err| {
        tr_format!(
            "HID initialisation failed: {err}",
            "Inicjalizacja HID nie powiodła się: {err}"
        )
    })?;
    let mut out = Vec::new();
    for info in api.device_list() {
        if info.vendor_id() != VID {
            continue;
        }
        if info.product_id() != PID_CABLE && info.product_id() != PID_RECEIVER {
            continue;
        }
        out.push(entry_from_info(info));
    }
    Ok(out)
}

pub fn list_all_devices() -> Result<Vec<HidEntry>, String> {
    let api = HidApi::new().map_err(|err| {
        tr_format!(
            "HID initialisation failed: {err}",
            "Inicjalizacja HID nie powiodła się: {err}"
        )
    })?;
    Ok(api.device_list().map(entry_from_info).collect())
}

fn entry_from_info(info: &DeviceInfo) -> HidEntry {
    HidEntry {
        pid: info.product_id(),
        product: info.product_string().unwrap_or("").trim().to_owned(),
        manufacturer: info.manufacturer_string().unwrap_or("").trim().to_owned(),
        usage_page: info.usage_page(),
        usage: info.usage(),
        interface: info.interface_number(),
        path: info.path().to_string_lossy().to_string(),
        serial: info.serial_number().unwrap_or("").trim().to_owned(),
    }
}

/// Diagnostic: probe every interface-1 collection of both PIDs with the short
/// status commands and the block commands. This shows which vendor collection
/// answers battery/link queries and which one carries the configuration blocks.
pub fn probe() -> Result<Vec<String>, String> {
    let api = HidApi::new().map_err(|err| {
        tr_format!(
            "HID initialisation failed: {err}",
            "Inicjalizacja HID nie powiodła się: {err}"
        )
    })?;
    let mut out = Vec::new();
    for info in api.device_list() {
        if info.vendor_id() != VID || info.interface_number() != 1 {
            continue;
        }
        let pid = info.product_id();
        if pid != PID_CABLE && pid != PID_RECEIVER {
            continue;
        }
        let header = format!(
            "PID {:04X}  usage {:04X}:{:04X}  product {}\n  path: {}",
            pid,
            info.usage_page(),
            info.usage(),
            info.product_string().unwrap_or("SMX Howler").trim(),
            info.path().to_string_lossy()
        );
        let dev = match api.open_path(info.path()) {
            Ok(dev) => dev,
            Err(err) => {
                out.push(format!(
                    "{header}\n  {}: {err}",
                    tr("open failed", "otwarcie nie powiodło się")
                ));
                continue;
            }
        };
        let link = probe_short(&dev, CMD_LINK);
        let battery = probe_short(&dev, CMD_BATTERY);
        let settings = probe_block(
            &dev,
            if pid == PID_CABLE {
                CMD_SETTINGS_CABLE
            } else {
                CMD_SETTINGS_RECEIVER
            },
        );
        let buttons = probe_block(
            &dev,
            if pid == PID_CABLE {
                CMD_BUTTONS_CABLE
            } else {
                CMD_BUTTONS_RECEIVER
            },
        );
        out.push(tr_format!(
            "{header}\n  short 0x80 link:    {link}\n  short 0x90 battery: {battery}\n  \
                 block settings:     {settings}\n  block buttons:      {buttons}",
            "{header}\n  krótkie 0x80 link:       {link}\n  krótkie 0x90 bateria:    {battery}\n  \
                 blok ustawień:           {settings}\n  blok przycisków:         {buttons}"
        ));
    }
    Ok(out)
}

fn probe_short(dev: &HidDevice, cmd: u8) -> String {
    let request = build_short(cmd);
    if let Err(err) = dev.send_feature_report(&request) {
        return tr_format!("send error: {err}", "błąd wysyłania: {err}");
    }
    thread::sleep(SHORT_DELAY);
    let mut response = [0u8; 8];
    response[0] = 5;
    match dev.get_feature_report(&mut response) {
        Ok(read) => {
            let shown = hex(&response[..read.min(response.len())]);
            if read == 8 && response[0] == 5 && response[1] == cmd {
                tr_format!("ok ({shown})", "ok ({shown})")
            } else {
                tr_format!(
                    "unexpected ({read} bytes: {shown})",
                    "nieoczekiwane ({read} bajtów: {shown})"
                )
            }
        }
        Err(err) => tr_format!("read error: {err}", "błąd odczytu: {err}"),
    }
}

fn probe_block(dev: &HidDevice, cmd: u8) -> String {
    let request = build_block_request(cmd);
    if let Err(err) = dev.send_feature_report(&request) {
        return tr_format!("send error: {err}", "błąd wysyłania: {err}");
    }
    thread::sleep(BLOCK_DELAY);
    let mut response = vec![0u8; REPORT_LEN];
    response[0] = 8;
    match dev.get_feature_report(&mut response) {
        Ok(read) if read >= 8 && response[0] == 8 && response[1] == cmd => {
            tr_format!(
                "ok ({read} bytes, header {})",
                "ok ({read} bajtów, nagłówek {})",
                hex(&response[..8])
            )
        }
        Ok(read) => tr_format!(
            "unexpected ({read} bytes: {})",
            "nieoczekiwane ({read} bajtów: {})",
            hex(&response[..read.min(16)])
        ),
        Err(err) => tr_format!("read error: {err}", "błąd odczytu: {err}"),
    }
}

fn display_product(product: &str) -> String {
    let trimmed = product.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("mouse")
        || trimmed.eq_ignore_ascii_case("hid-compliant mouse")
    {
        "SMX Howler".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Read the battery/status exactly like BatteryHub's Sinowealth driver:
/// cable first (it is the only path that reports charging), receiver second.
pub fn read_status(cable: Option<&SmxDevice>, receiver: Option<&SmxDevice>) -> DeviceStatus {
    let mut status = DeviceStatus::default();

    if let Some(dev) = cable {
        if let Ok(reply) = dev.short(CMD_BATTERY) {
            let transport = reply[2];
            let value = reply[3];
            if (transport & 0xFE) == 0x10 && (transport & 1) == 0 {
                status.connected = true;
                status.product = display_product(&dev.product);
                status.via = "USB cable".to_owned();
                status.full = value == 2;
                status.charging = value == 1 || status.full;
                status.percent = if status.full { Some(100) } else { None };
                status.identity = dev.identity().unwrap_or_default();
                return status;
            }
        }
    }

    if let Some(dev) = receiver {
        let link = dev.short(CMD_LINK).ok();
        let battery = dev.short(CMD_BATTERY).ok();
        if let (Some(link), Some(battery)) = (link, battery) {
            if link[2] <= 1
                && (battery[2] & 0xFE) == 0x10
                && (battery[2] & 1) == 1
                && battery[3] <= 100
            {
                let awake = link[2] != 0;
                status.connected = true;
                status.product = display_product(&dev.product);
                status.via = "2.4 GHz receiver".to_owned();
                status.asleep = !awake;
                status.link = Some(awake);
                status.charging = false;
                status.full = false;
                status.percent = if awake || battery[3] != 0 {
                    Some(battery[3])
                } else {
                    None
                };
                status.identity = dev.identity().unwrap_or_default();
                return status;
            }
        }
    }

    status.connected = false;
    status
}

#[cfg(test)]
mod hardware_tests {
    use super::*;

    /// Recovery path for a mouse whose blocks were written with a bad layout.
    /// Run with: `cargo test restore_reference_blocks -- --ignored --nocapture`
    #[test]
    #[ignore = "writes both configuration blocks to the connected mouse"]
    fn restore_reference_blocks() {
        let mut device = SmxDevice::open(PID_RECEIVER).expect("open the receiver");
        device.ensure_config().expect("reach the settings channel");
        device
            .write_pair_unchecked(
                Bank::Receiver,
                &RECOVERY_SETTINGS_BLOCK,
                &RECOVERY_BUTTONS_BLOCK,
            )
            .expect("write the reference blocks");
        thread::sleep(Duration::from_millis(400));

        let settings = device
            .read_block(CMD_SETTINGS_RECEIVER)
            .expect("read the settings back");
        let buttons = device
            .read_block(CMD_BUTTONS_RECEIVER)
            .expect("read the buttons back");
        assert_eq!(
            settings,
            RECOVERY_SETTINGS_BLOCK.to_vec(),
            "settings read-back"
        );
        assert_eq!(buttons, RECOVERY_BUTTONS_BLOCK.to_vec(), "button read-back");
    }
}
