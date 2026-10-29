use crate::{
    MUTED, WHITE,
    data::Result,
    hub_ui,
    profiles::{Settings, parse_gpu_selection},
    rect, text, theme, wide,
};
use serde_json::Value;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
};

pub const TAB: usize = 322;
pub const SCAN: usize = 323;
const MODE: usize = 390;
const DEVICES: usize = 391;
const ORDER: usize = 392;
const LIST: usize = 393;
const STATUS: usize = 394;
const UNAVAILABLE: isize = 1 << 32;

pub fn create(hwnd: HWND, font: HFONT) -> Result<()> {
    for (id, label) in [(TAB, "GPUs"), (SCAN, "Detect GPUs")] {
        hub_ui::add(hwnd, id, "BUTTON", label, BS_OWNERDRAW as u32, font)?;
    }
    for (id, labels) in [
        (MODE, &["Use source profile", "Choose GPUs"][..]),
        (ORDER, &["Automatic (Strata)", "Keep listed order"][..]),
    ] {
        let item = hub_ui::add(
            hwnd,
            id,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | CBS_OWNERDRAWFIXED as u32 | CBS_HASSTRINGS as u32,
            font,
        )?;
        unsafe {
            for label in labels {
                SendMessageW(item, CB_ADDSTRING, 0, wide(label).as_ptr() as LPARAM);
            }
            SendMessageW(item, CB_SETCURSEL, 0, 0);
        }
    }
    let editor = hub_ui::add(hwnd, DEVICES, "EDIT", "", ES_AUTOHSCROLL as u32, font)?;
    unsafe {
        SendMessageW(
            editor,
            windows_sys::Win32::UI::Controls::EM_SETLIMITTEXT,
            512,
            0,
        );
    }
    hub_ui::add(
        hwnd,
        LIST,
        "LISTBOX",
        "",
        LBS_MULTIPLESEL as u32
            | LBS_NOTIFY as u32
            | LBS_NOINTEGRALHEIGHT as u32
            | WS_VSCROLL
            | WS_HSCROLL,
        font,
    )?;
    hub_ui::add(
        hwnd,
        STATUS,
        "EDIT",
        "Detect GPUs to list device names and memory. No model is started.",
        ES_READONLY as u32 | ES_MULTILINE as u32,
        font,
    )?;
    Ok(())
}
fn item(hwnd: HWND, id: usize) -> HWND {
    unsafe { GetDlgItem(hwnd, id as i32) }
}
fn value(hwnd: HWND, id: usize) -> String {
    unsafe {
        let mut buffer = vec![0; GetWindowTextLengthW(item(hwnd, id)).max(0) as usize + 1];
        let length = GetWindowTextW(item(hwnd, id), buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..length.max(0) as usize])
    }
}
fn set(hwnd: HWND, id: usize, value: &str) {
    unsafe {
        SetWindowTextW(item(hwnd, id), wide(value).as_ptr());
    }
}
fn custom(hwnd: HWND) -> bool {
    unsafe { SendMessageW(item(hwnd, MODE), CB_GETCURSEL, 0, 0) == 1 }
}
pub fn read(hwnd: HWND) -> Result<(Option<Vec<u32>>, bool)> {
    let devices = if custom(hwnd) {
        Some(parse_gpu_selection(&value(hwnd, DEVICES))?)
    } else {
        None
    };
    let order = unsafe { SendMessageW(item(hwnd, ORDER), CB_GETCURSEL, 0, 0) };
    if let Some(devices) = &devices {
        unsafe {
            let list = item(hwnd, LIST);
            for row in 0..SendMessageW(list, LB_GETCOUNT, 0, 0) {
                let data = SendMessageW(list, LB_GETITEMDATA, row as usize, 0);
                if data & UNAVAILABLE != 0 && devices.contains(&(data as u32)) {
                    return Err(
                        "A selected GPU cannot run the installed engine. Choose a supported GPU."
                            .into(),
                    );
                }
            }
        }
    }
    if !(0..=1).contains(&order) {
        return Err("Select a GPU ordering mode".into());
    }
    Ok((devices, order == 1))
}
pub fn fill(hwnd: HWND, settings: &Settings, description: &str) {
    unsafe {
        SendMessageW(
            item(hwnd, MODE),
            CB_SETCURSEL,
            settings.gpu.is_some() as usize,
            0,
        );
        SendMessageW(
            item(hwnd, ORDER),
            CB_SETCURSEL,
            settings.gpu_keep_order as usize,
            0,
        );
    }
    set(
        hwnd,
        DEVICES,
        &settings
            .gpu
            .as_ref()
            .map(|v| v.iter().map(u32::to_string).collect::<Vec<_>>().join(","))
            .unwrap_or_default(),
    );
    clear_scan(hwnd);
    set(hwnd, STATUS, description);
}
pub fn clear_scan(hwnd: HWND) {
    unsafe {
        SendMessageW(item(hwnd, LIST), LB_RESETCONTENT, 0, 0);
    }
    set(
        hwnd,
        STATUS,
        "Detect GPUs to list device names and memory. No model is started.",
    );
}
pub fn scan_status(hwnd: HWND, message: &str) {
    set(hwnd, STATUS, message);
}
pub fn scan_result(hwnd: HWND, cards: &Value, message: &str) -> Result<()> {
    clear_scan(hwnd);
    let cards = cards.as_array().ok_or("Invalid GPU detection result")?;
    unsafe {
        let list = item(hwnd, LIST);
        for card in cards {
            let index = u32::try_from(card["index"].as_u64().ok_or("Invalid GPU index")?)?;
            let name = card["name"].as_str().ok_or("Missing GPU name")?;
            let memory = card["vram_gb"].as_f64().ok_or("Missing GPU memory")?;
            let problem = card["problem"].as_str();
            let label = format!(
                "GPU {index}    {name}    {memory:.1} GiB{}",
                problem
                    .map(|reason| format!("    [{reason}]"))
                    .unwrap_or_default()
            );
            let row = SendMessageW(list, LB_ADDSTRING, 0, wide(&label).as_ptr() as LPARAM);
            if row < 0 {
                return Err("Cannot populate the GPU list".into());
            }
            SendMessageW(
                list,
                LB_SETITEMDATA,
                row as usize,
                index as isize | if problem.is_some() { UNAVAILABLE } else { 0 },
            );
        }
        SendMessageW(list, LB_SETHORIZONTALEXTENT, 800, 0);
    }
    sync_selection(hwnd);
    set(hwnd, STATUS, message);
    Ok(())
}
fn sync_selection(hwnd: HWND) {
    // Invalid or incomplete text has no highlighted rows; validation happens on save/start.
    let devices = parse_gpu_selection(&value(hwnd, DEVICES));
    unsafe {
        let list = item(hwnd, LIST);
        for row in 0..SendMessageW(list, LB_GETCOUNT, 0, 0) {
            let index = SendMessageW(list, LB_GETITEMDATA, row as usize, 0) as u32;
            SendMessageW(
                list,
                LB_SETSEL,
                (custom(hwnd)
                    && devices
                        .as_ref()
                        .is_ok_and(|devices| devices.contains(&index))) as usize,
                row,
            );
        }
    }
}
pub fn notification(hwnd: HWND, id: usize, notification: u32) -> bool {
    if ![MODE, DEVICES, ORDER, LIST].contains(&id) {
        return false;
    }
    if unsafe { !GetPropW(hwnd, wide("StrataFilling").as_ptr()).is_null() } {
        return true;
    }
    if id == LIST && notification == LBN_SELCHANGE {
        unsafe {
            let list = item(hwnd, LIST);
            let mut devices = Vec::new();
            for row in 0..SendMessageW(list, LB_GETCOUNT, 0, 0) {
                if SendMessageW(list, LB_GETSEL, row as usize, 0) > 0 {
                    let data = SendMessageW(list, LB_GETITEMDATA, row as usize, 0);
                    devices.push((data as u32).to_string());
                }
            }
            SendMessageW(item(hwnd, MODE), CB_SETCURSEL, 1, 0);
            set(hwnd, DEVICES, &devices.join(","));
        }
        hub_ui::mark_dirty(hwnd);
    } else if (id == DEVICES && notification == EN_CHANGE)
        || ([MODE, ORDER].contains(&id) && notification == CBN_SELCHANGE)
    {
        sync_selection(hwnd);
        unsafe {
            for id in [DEVICES, ORDER, LIST] {
                EnableWindow(item(hwnd, id), custom(hwnd) as i32);
            }
        }
        hub_ui::mark_dirty(hwnd);
    }
    true
}
pub fn visible(hwnd: HWND, show: bool) {
    unsafe {
        for id in [MODE, DEVICES, ORDER, LIST, STATUS, SCAN] {
            ShowWindow(item(hwnd, id), if show { SW_SHOW } else { SW_HIDE });
        }
    }
}
pub fn enabled(hwnd: HWND, enabled: bool, scanning: bool) {
    unsafe {
        for id in [MODE, DEVICES, ORDER, LIST] {
            EnableWindow(
                item(hwnd, id),
                (enabled && (id == MODE || custom(hwnd))) as i32,
            );
        }
        EnableWindow(item(hwnd, SCAN), (enabled && !scanning) as i32);
    }
}
pub fn resize(hwnd: HWND, width: i32) {
    for (id, bounds) in [
        (MODE, rect(54, 406, 250, 240)),
        (DEVICES, rect(334, 413, width - 680, 22)),
        (ORDER, rect(width - 314, 406, 260, 240)),
        (SCAN, rect(width - 224, 478, 172, 36)),
        (LIST, rect(64, 528, width - 128, 74)),
        (STATUS, rect(64, 622, width - 128, 32)),
    ] {
        hub_ui::move_control(hwnd, id, bounds);
    }
}
pub fn paint(hwnd: HWND, dc: HDC, fonts: &[HFONT; 4], width: i32) {
    for (x, label) in [
        (52, "Selection"),
        (322, "GPU indices"),
        (width - 316, "Device order"),
    ] {
        text(dc, rect(x, 378, 250, 24), label, MUTED, fonts[0]);
    }
    for (id, bounds) in [
        (MODE, rect(52, 404, 254, 38)),
        (DEVICES, rect(322, 404, width - 656, 38)),
        (ORDER, rect(width - 316, 404, 264, 38)),
        (LIST, rect(52, 520, width - 104, 90)),
    ] {
        theme::input_frame(hwnd, id, dc, bounds);
    }
    text(
        dc,
        rect(52, 448, width - 104, 24),
        "One GPU: 0. Multiple GPUs: 0,1. Indices use NVIDIA / HIP device numbering.",
        MUTED,
        fonts[0],
    );
    text(
        dc,
        rect(52, 484, width - 300, 26),
        "Click each detected GPU to include or exclude it.",
        WHITE,
        fonts[0],
    );
}
