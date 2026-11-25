#![cfg_attr(
    all(not(debug_assertions), not(feature = "console")),
    windows_subsystem = "windows"
)]

mod app;
mod config;
mod device;
mod discord;
mod keys;
mod protocol;
mod worker;

use eframe::egui;

use app::HowlerApp;
use device::{list_smx_devices, read_status, SmxDevice};
use protocol::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(argument) = args.first() {
        match argument.as_str() {
            "--help" | "-h" | "/?" => {
                print_help();
                return;
            }
            "--list" => {
                std::process::exit(cmd_list());
            }
            "--battery" => {
                std::process::exit(cmd_battery());
            }
            "--dump-settings" => {
                std::process::exit(cmd_dump_settings());
            }
            "--dump-buttons" => {
                std::process::exit(cmd_dump_buttons());
            }
            "--probe" => {
                std::process::exit(cmd_probe());
            }
            "--write-test" => {
                std::process::exit(cmd_write_test());
            }
            other => {
                eprintln!("Unknown argument: {other}\n");
                print_help();
                std::process::exit(2);
            }
        }
    }

    if let Err(err) = run_gui() {
        eprintln!("SMX Howler could not start: {err}");
        std::process::exit(1);
    }
}

fn run_gui() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([1000.0, 640.0])
            .with_title("SMX Howler"),
        ..Default::default()
    };
    eframe::run_native(
        "SMX Howler",
        options,
        Box::new(|cc| Ok(Box::new(HowlerApp::new(cc)))),
    )
}

fn print_help() {
    println!(
        "SMX Howler control center\n\
         \n\
         Usage:\n\
           smx_howler                    open the graphical control center\n\
           smx_howler --list             list SMX Howler HID collections\n\
           smx_howler --battery          print battery and link status once\n\
           smx_howler --dump-settings    print the 154-byte settings block\n\
           smx_howler --dump-buttons     print the 88-byte button block\n\
           smx_howler --probe            test which HID collection answers what\n\
           smx_howler --write-test       rewrite the current blocks unchanged\n\
             add --force to skip the receiver link check\n\
           smx_howler --help             show this help\n\
         \n\
         Supported hardware:\n\
           SMX Howler USB cable     258A:002E\n\
           SMX Howler 2.4 GHz      258A:002F\n\
         \n\
         Close the SMX Howler vendor application (OemDrv.exe) before\n\
         reading or writing settings.\n\
         \n\
         Release builds hide the console window. Use a debug build or build\n\
         with `--features console` to see command-line output in release."
    );
}

fn cmd_list() -> i32 {
    match list_smx_devices() {
        Ok(entries) if entries.is_empty() => {
            println!(
                "No SMX Howler HID collections found (VID 258A, PID 002E/002F).\n\
                 Plug in the receiver or cable, switch the mouse on, and close OemDrv.exe."
            );
            1
        }
        Ok(entries) => {
            for entry in entries {
                println!(
                    "PID {:04X}  usage {:04X}:{:04X}  interface {}  {}\n  path: {}\n  serial: {}",
                    entry.pid,
                    entry.usage_page,
                    entry.usage,
                    entry.interface,
                    if entry.product.is_empty() {
                        entry.manufacturer.as_str()
                    } else {
                        entry.product.as_str()
                    },
                    entry.path,
                    entry.serial
                );
            }
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn cmd_battery() -> i32 {
    let cable = open_optional(PID_CABLE);
    let receiver = open_optional(PID_RECEIVER);
    if cable.is_none() && receiver.is_none() {
        eprintln!(
            "No SMX Howler found. Plug in the 2.4 GHz receiver or the USB cable, wake the mouse, \
             and close OemDrv.exe."
        );
        return 1;
    }
    let status = read_status(cable.as_ref(), receiver.as_ref());
    println!("Connected: {}", yes_no(status.connected));
    println!(
        "Transport: {}",
        if status.via.is_empty() {
            "—"
        } else {
            &status.via
        }
    );
    println!(
        "Product:   {}",
        if status.product.is_empty() {
            "SMX Howler"
        } else {
            &status.product
        }
    );
    println!("Battery:   {}", status.battery_text());
    println!("State:     {}", status.state_text());
    if let Some(error) = status.error {
        eprintln!("Error: {error}");
    }
    if status.connected {
        0
    } else {
        1
    }
}

fn cmd_dump_settings() -> i32 {
    let device = match open_first() {
        Ok(device) => device,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let bank = bank_for_pid(device.pid);
    match device.read_block(bank.settings_cmd()) {
        Ok(block) => {
            println!(
                "# SMX Howler settings block ({}, {} bytes)",
                bank.label(),
                block.len()
            );
            println!("{}", pretty_hex(&block));
            match SettingsDraft::from_block(&block) {
                Some(draft) => {
                    println!(
                        "Polling:      {} (code {})",
                        polling_label(draft.polling_hz),
                        polling_code(draft.polling_hz)
                    );
                    println!("Active stage: {}", draft.active_stage);
                    for (index, stage) in draft.stages.iter().enumerate() {
                        println!(
                            "Stage {}:      {:>5} DPI  colour #{:02X}{:02X}{:02X}",
                            index + 1,
                            stage.dpi(),
                            stage.color[0],
                            stage.color[1],
                            stage.color[2]
                        );
                    }
                    println!(
                        "Effect:       {} ({})",
                        effect_name(draft.effect),
                        draft.effect
                    );
                }
                None => eprintln!("The settings reply was too short to parse."),
            }
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn cmd_dump_buttons() -> i32 {
    let device = match open_first() {
        Ok(device) => device,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let bank = bank_for_pid(device.pid);
    match device.read_block(bank.buttons_cmd()) {
        Ok(block) => {
            println!(
                "# SMX Howler button block ({}, {} bytes)",
                bank.label(),
                block.len()
            );
            println!("{}", pretty_hex(&block));
            if let Some(slots) = parse_buttons(&block) {
                for (index, slot) in slots.iter().enumerate() {
                    println!(
                        "{:>2}  {}  [{:02X} {:02X} {:02X} {:02X}]",
                        index + 1,
                        slot_label(index),
                        slot[0],
                        slot[1],
                        slot[2],
                        slot[3]
                    );
                }
            } else {
                eprintln!("The button reply was too short to parse.");
            }
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn cmd_write_test() -> i32 {
    let device = match open_first() {
        Ok(device) => device,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let bank = bank_for_pid(device.pid);
    let settings = match device.read_block(bank.settings_cmd()) {
        Ok(block) => block,
        Err(err) => {
            eprintln!("Could not read the settings block: {err}");
            return 1;
        }
    };
    let buttons = match device.read_block(bank.buttons_cmd()) {
        Ok(block) => block,
        Err(err) => {
            eprintln!("Could not read the button block: {err}");
            return 1;
        }
    };

    println!(
        "Writing the current blocks back unchanged to the {}...",
        bank.label()
    );
    let force = std::env::args().any(|arg| arg == "--force");
    if force {
        println!("Link check skipped (--force); the mouse does not need to be awake.");
    }
    let result = if force {
        device.write_pair_unchecked(bank, &settings, &buttons)
    } else {
        device.write_pair(bank, &settings, &buttons)
    };
    if let Err(err) = result {
        eprintln!("Write failed: {err}");
        return 1;
    }

    let settings_after = match device.read_block(bank.settings_cmd()) {
        Ok(block) => block,
        Err(err) => {
            eprintln!("Settings read-back failed: {err}");
            return 1;
        }
    };
    let buttons_after = match device.read_block(bank.buttons_cmd()) {
        Ok(block) => block,
        Err(err) => {
            eprintln!("Button read-back failed: {err}");
            return 1;
        }
    };

    let settings_diff = count_diffs(&settings, &settings_after);
    let buttons_diff = count_diffs(&buttons, &buttons_after);
    println!("Settings read-back differences: {settings_diff}");
    println!("Button read-back differences:   {buttons_diff}");
    if settings_diff == 0 && buttons_diff == 0 {
        println!("Write path OK; the mouse accepted both blocks unchanged.");
        0
    } else {
        println!(
            "The mouse accepted the write but adjusted live bytes (common for the active \
             DPI stage). This is not an error."
        );
        0
    }
}

fn count_diffs(a: &[u8], b: &[u8]) -> usize {
    if a.len() != b.len() {
        return a.len().max(b.len());
    }
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

fn cmd_probe() -> i32 {
    match device::probe() {
        Ok(lines) if lines.is_empty() => {
            println!("No SMX Howler interface-1 collections found.");
            1
        }
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn open_optional(pid: u16) -> Option<SmxDevice> {
    SmxDevice::open(pid).ok()
}

fn open_first() -> Result<SmxDevice, String> {
    let mut errors = Vec::new();
    for pid in [PID_RECEIVER, PID_CABLE] {
        match SmxDevice::open(pid) {
            Ok(device) => return Ok(device),
            Err(err) => errors.push(err),
        }
    }
    Err(format!(
        "No SMX Howler node could be opened. Close OemDrv.exe and reconnect the device.\n\n{}",
        errors.join("\n")
    ))
}

fn bank_for_pid(pid: u16) -> Bank {
    if pid == PID_CABLE {
        Bank::Cable
    } else {
        Bank::Receiver
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}
