use crate::{
    ACCENT, MUTED, PANEL, WHITE,
    data::Result,
    engine_options::{Engine, OPTIONS},
    profiles::{
        ENVIRONMENT_SCOPES, FIELDS, Hub, PERFORMANCE_FIELD_START, Settings, parse_environment,
    },
    rect, text, theme, wide,
};
use std::ptr::null;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{BST_CHECKED, BST_UNCHECKED, EM_SETLIMITTEXT},
        Input::KeyboardAndMouse::EnableWindow,
        WindowsAndMessaging::*,
    },
};

pub const PROFILE: usize = 300;
pub const RUN: usize = 301;
pub const STOP: usize = 302;
pub const SAVE: usize = 303;
pub const RESET: usize = 304;
pub const LOG: usize = 305;
pub const EXIT: usize = 306;
pub const ENGINE: usize = 310;
pub const SAMPLING: usize = 311;
pub const ADVANCED: usize = 312;
pub const START_LOG: usize = 313;
pub const CALIBRATION: usize = 314;
pub const CALIBRATE: usize = 315;
pub const CALIBRATION_LOG: usize = 316;
pub const RESET_CALIBRATION: usize = 317;
pub const PERFORMANCE: usize = 318;
pub const RUN_EXPERIMENTAL: usize = 319;
pub const ENVIRONMENT: usize = 320;
pub const ENVIRONMENT_HELP: usize = 321;
const ASYNC: usize = 350;
const PIN: usize = 351;
const RESIDENT: usize = 352;
const VISION: usize = 353;
const PARALLEL: usize = 354;
const PORT: usize = 355;
const CONTEXT_PRESET: usize = 356;
const KV_GROW: usize = 357;
const KV_STREAM: usize = 358;
const CONTEXT_PRESETS: &[(&str, &str)] = &[
    ("4K", "4096"),
    ("8K", "8192"),
    ("16K", "16384"),
    ("32K", "32768"),
    ("64K", "65536"),
    ("96K", "98304"),
    ("128K", "131072"),
    ("192K", "196608"),
    ("256K", "262144"),
    ("Custom", ""),
];
const EXTRA: usize = 360;
const OPTION: usize = 361;
const ENVIRONMENT_SCOPE: usize = 362;
const FIRST_INHERIT: usize = 370;
const FIRST_ENVIRONMENT: usize = 380;
const FIRST_FIELD: usize = 400;
const FIRST_OPTION: usize = 500;
fn control(hwnd: HWND, id: usize) -> HWND {
    unsafe { GetDlgItem(hwnd, id as i32) }
}
fn set_text(hwnd: HWND, id: usize, value: &str) {
    unsafe {
        SetWindowTextW(control(hwnd, id), wide(value).as_ptr());
    }
}
fn get_text(hwnd: HWND, id: usize) -> String {
    unsafe {
        let item = control(hwnd, id);
        let length = GetWindowTextLengthW(item).max(0) as usize;
        let mut buffer = vec![0; length + 1];
        let copied = GetWindowTextW(item, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }
}
pub(crate) fn add(
    hwnd: HWND,
    id: usize,
    class: &str,
    title: &str,
    style: u32,
    font: HFONT,
) -> Result<HWND> {
    unsafe {
        let item = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(title).as_ptr(),
            WS_CHILD | WS_TABSTOP | style,
            0,
            0,
            200,
            if class == "COMBOBOX" { 240 } else { 30 },
            hwnd,
            id as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        if item.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        SendMessageW(item, WM_SETFONT, font as WPARAM, 1);
        if class == "COMBOBOX" {
            theme::combo(item)?;
            SendMessageW(
                item,
                CB_SETITEMHEIGHT,
                usize::MAX,
                if id == PROFILE { 34 } else { 26 },
            );
        }
        Ok(item)
    }
}
pub fn create(hwnd: HWND, hub: &Hub, fonts: &[HFONT; 4]) -> Result<()> {
    unsafe {
        let combo = add(
            hwnd,
            PROFILE,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32
                | CBS_OWNERDRAWFIXED as u32
                | CBS_HASSTRINGS as u32
                | WS_VSCROLL,
            fonts[0],
        )?;
        for profile in &hub.profiles {
            SendMessageW(
                combo,
                CB_ADDSTRING,
                0,
                wide(&profile.label).as_ptr() as LPARAM,
            );
        }
        for (index, field) in FIELDS.iter().enumerate() {
            let item = if field.choices.is_empty() {
                add(
                    hwnd,
                    FIRST_FIELD + index,
                    "EDIT",
                    "",
                    ES_AUTOHSCROLL as u32,
                    fonts[0],
                )?
            } else {
                add(
                    hwnd,
                    FIRST_FIELD + index,
                    "COMBOBOX",
                    "",
                    CBS_DROPDOWN as u32
                        | CBS_OWNERDRAWFIXED as u32
                        | CBS_HASSTRINGS as u32
                        | WS_VSCROLL,
                    fonts[0],
                )?
            };
            if field.choices.is_empty() {
                SendMessageW(item, EM_SETLIMITTEXT, 64, 0);
            } else {
                for value in field.choices {
                    SendMessageW(item, CB_ADDSTRING, 0, wide(value).as_ptr() as LPARAM);
                }
            }
        }
        add(
            hwnd,
            OPTION,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32
                | CBS_OWNERDRAWFIXED as u32
                | CBS_HASSTRINGS as u32
                | WS_VSCROLL,
            fonts[0],
        )?;
        for (index, option) in OPTIONS.iter().enumerate() {
            let choices = option.choices();
            let item = add(
                hwnd,
                FIRST_OPTION + index,
                if choices.is_empty() {
                    "EDIT"
                } else {
                    "COMBOBOX"
                },
                "",
                if choices.is_empty() {
                    ES_AUTOHSCROLL as u32
                } else {
                    CBS_DROPDOWN as u32
                        | CBS_OWNERDRAWFIXED as u32
                        | CBS_HASSTRINGS as u32
                        | WS_VSCROLL
                },
                fonts[0],
            )?;
            if choices.is_empty() {
                SendMessageW(item, EM_SETLIMITTEXT, 4096, 0);
            } else {
                SendMessageW(item, CB_ADDSTRING, 0, wide("").as_ptr() as LPARAM);
                for choice in choices {
                    SendMessageW(item, CB_ADDSTRING, 0, wide(choice).as_ptr() as LPARAM);
                }
            }
        }
        for (id, label) in [
            (PIN, "Pin by need"),
            (RESIDENT, "Resident experts"),
            (KV_GROW, "Elastic KV cache"),
            (KV_STREAM, "KV streaming (32K GPU, more RAM)"),
            (VISION, "Enable Vision"),
        ] {
            let checkbox = add(hwnd, id, "BUTTON", label, BS_AUTOCHECKBOX as u32, fonts[0])?;
            theme::checkbox(checkbox)?;
        }
        let adaptive = add(
            hwnd,
            ASYNC,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32
                | CBS_OWNERDRAWFIXED as u32
                | CBS_HASSTRINGS as u32
                | WS_VSCROLL,
            fonts[0],
        )?;
        for label in ["Auto (engine default)", "On", "Off"] {
            SendMessageW(adaptive, CB_ADDSTRING, 0, wide(label).as_ptr() as LPARAM);
        }
        let context = add(
            hwnd,
            CONTEXT_PRESET,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32
                | CBS_OWNERDRAWFIXED as u32
                | CBS_HASSTRINGS as u32
                | WS_VSCROLL,
            fonts[0],
        )?;
        for (label, _) in CONTEXT_PRESETS {
            SendMessageW(context, CB_ADDSTRING, 0, wide(label).as_ptr() as LPARAM);
        }
        SendMessageW(context, CB_SETDROPPEDWIDTH, 128, 0);
        SendMessageW(
            context,
            windows_sys::Win32::UI::Controls::CB_SETMINVISIBLE,
            CONTEXT_PRESETS.len(),
            0,
        );
        let parallel = add(
            hwnd,
            PARALLEL,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32
                | CBS_OWNERDRAWFIXED as u32
                | CBS_HASSTRINGS as u32
                | WS_VSCROLL,
            fonts[0],
        )?;
        for n in 1..=8 {
            let label = if n == 1 {
                "1 · sequential".into()
            } else {
                n.to_string()
            };
            SendMessageW(parallel, CB_ADDSTRING, 0, wide(&label).as_ptr() as LPARAM);
        }
        let extra = add(
            hwnd,
            EXTRA,
            "EDIT",
            "",
            WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_WANTRETURN as u32,
            fonts[3],
        )?;
        let port = add(
            hwnd,
            PORT,
            "EDIT",
            "",
            ES_AUTOHSCROLL as u32 | ES_NUMBER as u32,
            fonts[0],
        )?;
        SendMessageW(port, EM_SETLIMITTEXT, 5, 0);
        SendMessageW(extra, EM_SETLIMITTEXT, 32768, 0);
        let scopes = add(
            hwnd,
            ENVIRONMENT_SCOPE,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32
                | CBS_OWNERDRAWFIXED as u32
                | CBS_HASSTRINGS as u32
                | WS_VSCROLL,
            fonts[0],
        )?;
        for (index, label) in ENVIRONMENT_SCOPES.iter().enumerate() {
            let row = SendMessageW(scopes, CB_ADDSTRING, 0, wide(label).as_ptr() as LPARAM);
            SendMessageW(scopes, CB_SETITEMDATA, row as usize, index as isize);
        }
        SendMessageW(scopes, CB_SETCURSEL, 0, 0);
        for index in 0..ENVIRONMENT_SCOPES.len() {
            let editor = add(
                hwnd,
                FIRST_ENVIRONMENT + index,
                "EDIT",
                "",
                WS_VSCROLL
                    | WS_HSCROLL
                    | ES_MULTILINE as u32
                    | ES_AUTOVSCROLL as u32
                    | ES_AUTOHSCROLL as u32
                    | ES_WANTRETURN as u32,
                fonts[3],
            )?;
            SendMessageW(editor, EM_SETLIMITTEXT, 32768, 0);
        }
        for index in 0..2 {
            let checkbox = add(
                hwnd,
                FIRST_INHERIT + index,
                "BUTTON",
                "Inherit standard environment",
                BS_AUTOCHECKBOX as u32,
                fonts[0],
            )?;
            theme::checkbox(checkbox)?;
        }
        for (id, label) in [
            (RUN, "Start standard"),
            (RUN_EXPERIMENTAL, "Start experimental"),
            (STOP, "Stop"),
            (SAVE, "Save changes"),
            (RESET, "Reset"),
            (LOG, "Open log"),
            (START_LOG, "Startup details"),
            (EXIT, "Exit hub"),
            (ENGINE, "Engine"),
            (SAMPLING, "Sampling"),
            (ADVANCED, "Advanced"),
            (ENVIRONMENT, "Environment"),
            (ENVIRONMENT_HELP, "Available variables (?)"),
            (CALIBRATION, "Calibration"),
            (PERFORMANCE, "Performance"),
            (CALIBRATE, "Calibrate model"),
            (CALIBRATION_LOG, "Open calibration log"),
            (RESET_CALIBRATION, "Reset calibration"),
        ] {
            add(hwnd, id, "BUTTON", label, BS_OWNERDRAW as u32, fonts[0])?;
        }
        crate::environment_help::tooltip(hwnd, control(hwnd, ENVIRONMENT_HELP))?;
        crate::gpu_ui::create(hwnd, fonts[0])?;
        fill(hwnd, hub);
        Ok(())
    }
}
pub fn active_section(hwnd: HWND) -> usize {
    unsafe { GetWindowLongPtrW(control(hwnd, ENGINE), GWLP_USERDATA) as usize }
}
pub fn select_section(hwnd: HWND, section: usize) {
    unsafe {
        SetWindowLongPtrW(control(hwnd, ENGINE), GWLP_USERDATA, section as isize);
        resize(hwnd);
        visible(hwnd, true);
        for id in [
            ENGINE,
            SAMPLING,
            ADVANCED,
            CALIBRATION,
            PERFORMANCE,
            ENVIRONMENT,
            crate::gpu_ui::TAB,
        ] {
            InvalidateRect(control(hwnd, id), null(), 0);
        }
        InvalidateRect(hwnd, null(), 0);
    }
}
pub fn modified(hwnd: HWND) -> bool {
    unsafe { GetWindowLongPtrW(control(hwnd, SAVE), GWLP_USERDATA) != 0 }
}
pub fn mark_dirty(hwnd: HWND) {
    unsafe {
        SetWindowLongPtrW(control(hwnd, SAVE), GWLP_USERDATA, 1);
        InvalidateRect(control(hwnd, SAVE), null(), 0);
        InvalidateRect(hwnd, null(), 0);
    }
}
pub fn saved(hwnd: HWND) {
    unsafe {
        SetWindowLongPtrW(control(hwnd, SAVE), GWLP_USERDATA, 0);
        InvalidateRect(control(hwnd, SAVE), null(), 0);
        InvalidateRect(hwnd, null(), 0);
    }
}
pub fn edit_notification(hwnd: HWND, id: usize, notification: u32) -> bool {
    if crate::gpu_ui::notification(hwnd, id, notification) {
        return true;
    }
    unsafe {
        if id == OPTION || id == ENVIRONMENT_SCOPE {
            if notification == CBN_SELCHANGE {
                visible(hwnd, true);
                InvalidateRect(hwnd, null(), 0);
            }
            return true;
        }
        let is_field = (FIRST_FIELD..FIRST_FIELD + FIELDS.len()).contains(&id);
        let is_option = (FIRST_OPTION..FIRST_OPTION + OPTIONS.len()).contains(&id);
        let is_environment = (FIRST_ENVIRONMENT..FIRST_ENVIRONMENT + 4).contains(&id);
        let inherit = (FIRST_INHERIT..FIRST_INHERIT + 2).contains(&id);
        let checkbox = [PIN, RESIDENT, VISION, KV_GROW, KV_STREAM].contains(&id) || inherit;
        if !is_field
            && !is_option
            && !is_environment
            && !checkbox
            && id != EXTRA
            && id != PARALLEL
            && id != PORT
            && id != CONTEXT_PRESET
            && id != ASYNC
        {
            return false;
        }
        if !GetPropW(hwnd, wide("StrataFilling").as_ptr()).is_null() {
            return true;
        }
        if id == CONTEXT_PRESET {
            if notification == CBN_SELCHANGE {
                let index = SendMessageW(control(hwnd, CONTEXT_PRESET), CB_GETCURSEL, 0, 0);
                if let Some((_, value)) = CONTEXT_PRESETS.get(index as usize) {
                    if value.is_empty() {
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(control(
                            hwnd,
                            FIRST_FIELD,
                        ));
                        SendMessageW(
                            control(hwnd, FIRST_FIELD),
                            windows_sys::Win32::UI::Controls::EM_SETSEL,
                            0,
                            -1,
                        );
                    } else {
                        set_text(hwnd, FIRST_FIELD, value);
                    }
                }
            }
            return true;
        }
        if id == FIRST_FIELD && notification == EN_CHANGE {
            sync_context(hwnd);
        }
        if inherit && notification == BN_CLICKED {
            visible(hwnd, true);
        }
        if matches!(id, KV_GROW | KV_STREAM)
            && notification == BN_CLICKED
            && SendMessageW(control(hwnd, id), BM_GETCHECK, 0, 0) == BST_CHECKED as isize
        {
            let other = if id == KV_STREAM { KV_GROW } else { KV_STREAM };
            SendMessageW(control(hwnd, other), BM_SETCHECK, BST_UNCHECKED as usize, 0);
        }
        let combo = id == PARALLEL
            || id == ASYNC
            || (is_field && !FIELDS[id - FIRST_FIELD].choices.is_empty())
            || (is_option && !OPTIONS[id - FIRST_OPTION].choices().is_empty());
        if (checkbox && notification == BN_CLICKED)
            || (combo && [CBN_EDITCHANGE, CBN_SELCHANGE].contains(&notification))
            || (!combo && !checkbox && notification == EN_CHANGE)
        {
            mark_dirty(hwnd);
        } else {
            InvalidateRect(hwnd, null(), 0);
        }
        true
    }
}
fn sync_context(hwnd: HWND) {
    let value = get_text(hwnd, FIRST_FIELD);
    let index = CONTEXT_PRESETS
        .iter()
        .position(|(_, tokens)| {
            !tokens.is_empty() && value.trim().parse::<u32>().ok() == tokens.parse::<u32>().ok()
        })
        .unwrap_or(CONTEXT_PRESETS.len() - 1);
    unsafe {
        SendMessageW(control(hwnd, CONTEXT_PRESET), CB_SETCURSEL, index, 0);
    }
}
// All positions below use content coordinates; only native child windows add SIDE.
fn field_section(index: usize) -> usize {
    if index < 7 {
        0
    } else if index < PERFORMANCE_FIELD_START {
        1
    } else {
        4
    }
}
fn engine(hwnd: HWND) -> Engine {
    if unsafe { GetPropW(hwnd, wide("StrataDeepSeek").as_ptr()).is_null() } {
        Engine::Qwen
    } else {
        Engine::DeepSeek
    }
}
fn selected_option(hwnd: HWND) -> Option<usize> {
    unsafe {
        let combo = control(hwnd, OPTION);
        let index = SendMessageW(combo, CB_GETCURSEL, 0, 0);
        if index < 0 {
            return None;
        }
        let option = SendMessageW(combo, CB_GETITEMDATA, index as usize, 0);
        usize::try_from(option).ok().filter(|i| *i < OPTIONS.len())
    }
}
fn environment_scope(hwnd: HWND) -> usize {
    unsafe {
        let combo = control(hwnd, ENVIRONMENT_SCOPE);
        let row = SendMessageW(combo, CB_GETCURSEL, 0, 0);
        assert!(row >= 0, "No environment scope selected");
        let scope = SendMessageW(combo, CB_GETITEMDATA, row as usize, 0);
        assert!(
            (0..ENVIRONMENT_SCOPES.len() as isize).contains(&scope),
            "Invalid environment scope"
        );
        scope as usize
    }
}
pub fn open_environment_help(hwnd: HWND, fonts: [HFONT; 4]) -> Result<()> {
    crate::environment_help::show(hwnd, fonts, engine(hwnd), environment_scope(hwnd))
}
fn field_rect(index: usize, width: i32) -> RECT {
    let card = (width - 76) / 2;
    let cell = (card - 64) / 2;
    if index < 7 {
        let (side, row, column) = match index {
            0 => (0, 0, 0),
            1 => (0, 0, 1),
            4 => (0, 1, 0),
            3 => (0, 1, 1),
            2 => (1, 0, 0),
            5 => (1, 0, 1),
            6 => (1, 1, 0),
            _ => unreachable!(),
        };
        rect(
            52 + side * (card + 20) + column * (cell + 16),
            398 + row * 76,
            cell,
            38,
        )
    } else {
        let start = if index < PERFORMANCE_FIELD_START {
            7
        } else {
            PERFORMANCE_FIELD_START
        };
        let i = (index - start) as i32;
        let cell = (width - 152) / 3;
        rect(52 + (i % 3) * (cell + 24), 390 + (i / 3) * 76, cell, 38)
    }
}
pub fn visible(hwnd: HWND, show: bool) {
    let show = show && unsafe { GetPropW(hwnd, wide("StrataNoProfiles").as_ptr()).is_null() };
    unsafe {
        let section = active_section(hwnd);
        for id in [
            PROFILE,
            RUN,
            RUN_EXPERIMENTAL,
            STOP,
            SAVE,
            RESET,
            LOG,
            START_LOG,
            ENGINE,
            SAMPLING,
            ADVANCED,
            CALIBRATION,
            PERFORMANCE,
            ENVIRONMENT,
            crate::gpu_ui::TAB,
        ] {
            let available = match id {
                RUN_EXPERIMENTAL => {
                    !GetPropW(hwnd, wide("StrataExperimentalAvailable").as_ptr()).is_null()
                }
                PERFORMANCE | CALIBRATION => engine(hwnd) == Engine::Qwen,
                _ => true,
            };
            ShowWindow(
                control(hwnd, id),
                if show && available { SW_SHOW } else { SW_HIDE },
            );
        }
        for i in 0..FIELDS.len() {
            ShowWindow(
                control(hwnd, FIRST_FIELD + i),
                if show && field_section(i) == section && engine(hwnd).supports_field(i) {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        if show {
            for (i, field) in FIELDS.iter().enumerate() {
                if !field.choices.is_empty() {
                    SendMessageW(control(hwnd, FIRST_FIELD + i), CB_SETEDITSEL, 0, 0);
                }
            }
        }
        for id in [
            ASYNC,
            PIN,
            RESIDENT,
            KV_GROW,
            KV_STREAM,
            PARALLEL,
            PORT,
            CONTEXT_PRESET,
        ] {
            ShowWindow(
                control(hwnd, id),
                if show
                    && section == 0
                    && (engine(hwnd) == Engine::Qwen || [PORT, CONTEXT_PRESET].contains(&id))
                {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        ShowWindow(
            control(hwnd, VISION),
            if show && !GetPropW(hwnd, wide("StrataVisionAvailable").as_ptr()).is_null() {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        ShowWindow(
            control(hwnd, EXTRA),
            if show && section == 2 {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        ShowWindow(
            control(hwnd, OPTION),
            if show && section == 2 {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        ShowWindow(
            control(hwnd, ENVIRONMENT_SCOPE),
            if show && section == 5 {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        ShowWindow(
            control(hwnd, ENVIRONMENT_HELP),
            if show && section == 5 {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        let scope = environment_scope(hwnd);
        crate::gpu_ui::visible(hwnd, show && section == 6);
        for index in 0..4 {
            let editor = control(hwnd, FIRST_ENVIRONMENT + index);
            let inherited = index % 2 == 1
                && SendMessageW(control(hwnd, FIRST_INHERIT + index / 2), BM_GETCHECK, 0, 0)
                    == BST_CHECKED as isize;
            ShowWindow(
                editor,
                if show && section == 5 && scope == index {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            EnableWindow(editor, (!inherited) as i32);
        }
        for index in 0..2 {
            ShowWindow(
                control(hwnd, FIRST_INHERIT + index),
                if show && section == 5 && scope == index * 2 + 1 {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        let selected = selected_option(hwnd);
        for (i, option) in OPTIONS.iter().enumerate() {
            ShowWindow(
                control(hwnd, FIRST_OPTION + i),
                if show && section == 2 && selected == Some(i) && option.engine == engine(hwnd) {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        for id in [CALIBRATE, CALIBRATION_LOG, RESET_CALIBRATION] {
            ShowWindow(
                control(hwnd, id),
                if show && section == 3 && engine(hwnd) == Engine::Qwen {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        ShowWindow(control(hwnd, EXIT), SW_SHOW);
    }
}
pub(crate) fn move_control(hwnd: HWND, id: usize, r: RECT) {
    unsafe {
        MoveWindow(
            control(hwnd, id),
            r.left + theme::SIDE,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            1,
        );
    }
}
pub fn resize(hwnd: HWND) {
    unsafe {
        let mut client: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut client);
        let w = client.right - theme::SIDE;
        let h = client.bottom;
        move_control(hwnd, PROFILE, rect(52, 154, (w - 432).max(300), 300));
        move_control(hwnd, RUN, rect(w - 352, 153, 180, 44));
        move_control(hwnd, STOP, rect(w - 156, 153, 104, 44));
        move_control(hwnd, RUN_EXPERIMENTAL, rect(w - 388, 208, 168, 30));
        let tabs: &[usize] = if engine(hwnd) == Engine::DeepSeek {
            &[ENGINE, crate::gpu_ui::TAB, SAMPLING, ADVANCED, ENVIRONMENT]
        } else {
            &[
                ENGINE,
                crate::gpu_ui::TAB,
                PERFORMANCE,
                SAMPLING,
                ADVANCED,
                ENVIRONMENT,
                CALIBRATION,
            ]
        };
        for (i, id) in tabs.iter().enumerate() {
            let tab_width = (w - 56 - (tabs.len() as i32 - 1) * 8) / tabs.len() as i32;
            move_control(
                hwnd,
                *id,
                rect(28 + i as i32 * (tab_width + 8), 266, tab_width, 38),
            );
        }
        crate::gpu_ui::resize(hwnd, w);
        move_control(hwnd, CALIBRATE, rect(52, 592, 180, 40));
        move_control(hwnd, CALIBRATION_LOG, rect(248, 592, 192, 40));
        move_control(hwnd, RESET_CALIBRATION, rect(456, 592, 188, 40));
        for (i, field) in FIELDS.iter().enumerate() {
            let r = field_rect(i, w);
            move_control(
                hwnd,
                FIRST_FIELD + i,
                if i == 0 {
                    rect(r.left + 100, r.top + 9, r.right - r.left - 112, 22)
                } else if field.choices.is_empty() {
                    rect(r.left + 12, r.top + 9, r.right - r.left - 24, 22)
                } else {
                    rect(r.left + 2, r.top + 2, r.right - r.left - 4, 240)
                },
            );
        }
        let card = (w - 76) / 2;
        let cell = (card - 64) / 2;
        move_control(hwnd, CONTEXT_PRESET, rect(54, 400, 82, 320));
        move_control(hwnd, PARALLEL, rect(642, 634, (w - 694).min(260), 240));
        move_control(hwnd, START_LOG, rect(w - 204, 208, 152, 30));
        move_control(hwnd, PORT, rect(288, 641, 114, 22));
        for (id, x, y, width) in [
            (KV_GROW, 52, 522, card - 48),
            (KV_STREAM, 52, 550, card - 48),
            (PIN, 52, 578, cell),
            (RESIDENT, 52 + cell + 16, 578, cell),
            (VISION, 52, 212, 140),
        ] {
            move_control(hwnd, id, rect(x, y, width, 26));
        }
        move_control(
            hwnd,
            ASYNC,
            rect(54 + card + 20 + cell + 16, 476, cell - 4, 240),
        );
        let option_width = (w - 128) * 2 / 3;
        move_control(hwnd, OPTION, rect(54, 392, option_width - 4, 360));
        SendMessageW(
            control(hwnd, OPTION),
            CB_SETDROPPEDWIDTH,
            (w - 108) as usize,
            0,
        );
        for (i, option) in OPTIONS.iter().enumerate() {
            let x = 52 + option_width + 24;
            let width = w - x - 52;
            move_control(
                hwnd,
                FIRST_OPTION + i,
                if option.choices().is_empty() {
                    rect(x + 12, 399, width - 24, 22)
                } else {
                    rect(x + 2, 392, width - 4, 240)
                },
            );
        }
        move_control(hwnd, EXTRA, rect(64, 554, w - 128, 78));
        move_control(hwnd, ENVIRONMENT_SCOPE, rect(54, 380, 300, 240));
        move_control(hwnd, ENVIRONMENT_HELP, rect(w - 264, 334, 212, 32));
        for index in 0..4 {
            move_control(hwnd, FIRST_ENVIRONMENT + index, rect(64, 464, w - 128, 110));
        }
        for index in 0..2 {
            move_control(hwnd, FIRST_INHERIT + index, rect(384, 382, w - 436, 26));
        }
        move_control(hwnd, LOG, rect(28, h - 90, 112, 40));
        move_control(hwnd, RESET, rect(w - 332, h - 90, 112, 40));
        move_control(hwnd, SAVE, rect(w - 204, h - 90, 176, 40));
        MoveWindow(control(hwnd, EXIT), 16, h - 56, 168, 36, 1);
    }
}
pub fn paint(
    hwnd: HWND,
    dc: HDC,
    hub: &Hub,
    fonts: &[HFONT; 4],
    width: i32,
    height: i32,
    launch_color: u32,
    stage: usize,
    calibration: &crate::jobs::Status,
    calibration_started: Option<f64>,
) {
    let Some(profile) = hub.profiles.get(hub.selected) else {
        text(
            dc,
            rect(52, 140, width - 104, 42),
            "No prepared model profiles",
            WHITE,
            fonts[1],
        );
        text(
            dc,
            rect(52, 198, width - 104, 32),
            "Download and prepare a model in Model library to create a runnable profile.",
            MUTED,
            fonts[0],
        );
        return;
    };
    let section = active_section(hwnd);
    let card = (width - 76) / 2;
    theme::rounded(
        dc,
        rect(28, 112, width - 56, 136),
        PANEL,
        Some(theme::BORDER),
        16,
    );
    text(dc, rect(52, 126, 450, 24), "MODEL PROFILE", MUTED, fonts[0]);
    if !profile.has_vision {
        theme::rounded(dc, rect(52, 212, 96, 26), theme::INPUT, None, 10);
        text(dc, rect(64, 212, 80, 26), "Text only", MUTED, fonts[0]);
    }
    text(
        dc,
        rect(208, 212, width - 640, 26),
        &format!(
            "{} · {}",
            profile.settings.engine.name(),
            if let Some(devices) = &profile.settings.gpu {
                format!(
                    "GPU {}",
                    devices
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                "Profile GPU settings".into()
            }
        ),
        MUTED,
        fonts[0],
    );
    if stage > 0 {
        let segment = (width - 104 - 4 * 8) / 5;
        for i in 0..5 {
            theme::rounded(
                dc,
                rect(52 + i * (segment + 8), 242, segment, 2),
                if (i as usize) < stage {
                    launch_color
                } else {
                    theme::BORDER
                },
                None,
                2,
            );
        }
    }
    if section == 0 {
        for (x, title) in [(28, "Memory"), (48 + card, "Generation")] {
            theme::rounded(dc, rect(x, 320, card, 288), PANEL, Some(theme::BORDER), 12);
            text(dc, rect(x + 24, 334, card - 48, 28), title, WHITE, fonts[2]);
        }
        if profile.settings.engine == Engine::DeepSeek {
            text(
                dc,
                rect(52, 452, card - 48, 32),
                "DeepSeek manages its own memory cache.",
                MUTED,
                fonts[0],
            );
            text(
                dc,
                rect(72 + card, 390, card - 48, 32),
                "DSpark and CPU/GPU tuning: Advanced",
                MUTED,
                fonts[0],
            );
            text(
                dc,
                rect(72 + card, 434, card - 48, 32),
                "One request at a time; text only.",
                MUTED,
                fonts[0],
            );
        } else if unsafe { SendMessageW(control(hwnd, KV_STREAM), BM_GETCHECK, 0, 0) }
            == BST_CHECKED as isize
        {
            text(
                dc,
                rect(72 + card, 520, card - 48, 26),
                "Streaming keeps full context in RAM.",
                MUTED,
                fonts[0],
            );
            text(
                dc,
                rect(72 + card, 548, card - 48, 26),
                "VRAM reserve: at least 1000 MiB.",
                MUTED,
                fonts[0],
            );
        }
        if profile.settings.engine == Engine::Qwen {
            let cell = (card - 64) / 2;
            let x = 52 + card + 20 + cell + 16;
            text(dc, rect(x, 446, cell, 22), "Adapt async", MUTED, fonts[0]);
            theme::input_frame(hwnd, ASYNC, dc, rect(x, 474, cell, 38));
        }
        theme::rounded(
            dc,
            rect(28, 620, width - 56, 60),
            PANEL,
            Some(theme::BORDER),
            16,
        );
        text(dc, rect(52, 632, 140, 36), "Server", WHITE, fonts[2]);
        text(dc, rect(216, 634, 54, 34), "Port", MUTED, fonts[0]);
        theme::input_frame(hwnd, PORT, dc, rect(276, 632, 140, 38));
        if profile.settings.engine == Engine::Qwen {
            text(
                dc,
                rect(448, 634, 180, 34),
                "Concurrent requests",
                MUTED,
                fonts[0],
            );
            theme::input_frame(
                hwnd,
                PARALLEL,
                dc,
                rect(640, 632, (width - 690).min(264), 38),
            );
        }
    } else {
        theme::rounded(
            dc,
            rect(28, 320, width - 56, 348),
            PANEL,
            Some(theme::BORDER),
            16,
        );
        text(
            dc,
            rect(52, 336, width - 104, 32),
            match section {
                1 => "Default sampling",
                3 => "Hardware calibration",
                4 => "Performance tuning",
                5 => "Environment variables",
                6 => "GPU selection",
                _ => {
                    if profile.settings.engine == Engine::DeepSeek {
                        "Advanced: DeepSeek"
                    } else {
                        "Advanced: Qwen"
                    }
                }
            },
            WHITE,
            fonts[2],
        );
    }
    if section == 6 {
        crate::gpu_ui::paint(hwnd, dc, fonts, width);
    } else if section == 3 {
        paint_calibration(dc, profile, fonts, width, calibration, calibration_started);
    } else if section < 2 || section == 4 {
        for (i, field) in FIELDS.iter().enumerate() {
            if field_section(i) != section || !profile.settings.engine.supports_field(i) {
                continue;
            }
            let r = field_rect(i, width);
            text(
                dc,
                rect(r.left, r.top - 28, r.right - r.left, 22),
                field.label,
                MUTED,
                fonts[0],
            );
            if i == 0 {
                theme::input_frame(hwnd, CONTEXT_PRESET, dc, rect(r.left, r.top, 88, 38));
                theme::input_frame(
                    hwnd,
                    FIRST_FIELD,
                    dc,
                    rect(r.left + 92, r.top, r.right - r.left - 92, 38),
                );
            } else {
                theme::input_frame(hwnd, FIRST_FIELD + i, dc, r);
            }
        }
        if section == 4 {
            text(
                dc,
                rect(52, 556, width - 104, 32),
                "Blank fields use engine defaults. MTP minimum probability is in Engine.",
                MUTED,
                fonts[0],
            );
            text(
                dc,
                rect(52, 594, width - 104, 32),
                "Calibration fills these controls; saved changes apply on the next model launch.",
                MUTED,
                fonts[0],
            );
        }
    } else if section == 5 {
        theme::input_frame(hwnd, ENVIRONMENT_SCOPE, dc, rect(52, 378, 304, 38));
        text(
            dc,
            rect(52, 424, width - 104, 28),
            "One NAME=value per line. Empty values are allowed; remove a line to remove its override.",
            MUTED,
            fonts[0],
        );
        let scope = environment_scope(hwnd);
        theme::input_frame(
            hwnd,
            FIRST_ENVIRONMENT + scope,
            dc,
            rect(52, 456, width - 104, 128),
        );
        text(
            dc,
            rect(52, 594, width - 104, 24),
            if profile.has_experimental {
                "Experimental environments replace standard ones unless inheritance is enabled."
            } else {
                "Engine variables configure inference; server variables configure the Python server."
            },
            MUTED,
            fonts[0],
        );
        text(
            dc,
            rect(52, 622, width - 104, 24),
            if profile.settings.engine == Engine::Qwen {
                "Pin by need / Elastic KV use Engine switches. KV streaming forces STRATA_VRAM_BUDGET=0."
            } else {
                "Saved variables apply to this model on its next launch. Windows system settings stay unchanged."
            },
            MUTED,
            fonts[0],
        );
    } else {
        let option_width = (width - 128) * 2 / 3;
        text(
            dc,
            rect(52, 364, option_width, 24),
            "Parameter",
            MUTED,
            fonts[0],
        );
        text(
            dc,
            rect(76 + option_width, 364, width - option_width - 128, 24),
            "Value",
            MUTED,
            fonts[0],
        );
        theme::input_frame(hwnd, OPTION, dc, rect(52, 390, option_width, 38));
        if let Some(index) = selected_option(hwnd) {
            theme::input_frame(
                hwnd,
                FIRST_OPTION + index,
                dc,
                rect(76 + option_width, 390, width - option_width - 128, 38),
            );
            unsafe {
                let old = SelectObject(dc, fonts[0]);
                SetTextColor(dc, MUTED);
                SetBkMode(dc, TRANSPARENT as i32);
                DrawTextW(
                    dc,
                    wide(OPTIONS[index].help).as_ptr(),
                    -1,
                    &mut rect(52, 440, width - 104, 54),
                    DT_WORDBREAK | DT_NOPREFIX,
                );
                SelectObject(dc, old);
            }
        }
        text(
            dc,
            rect(52, 496, width - 104, 24),
            "Blank = engine default. Values stay in this form when you switch parameters.",
            MUTED,
            fonts[0],
        );
        text(
            dc,
            rect(52, 524, width - 104, 24),
            "Additional engine arguments (JSON array of strings)",
            MUTED,
            fonts[0],
        );
        theme::input_frame(hwnd, EXTRA, dc, rect(52, 546, width - 104, 96));
    }
    crate::fill(dc, rect(28, height - 112, width - 56, 1), theme::BORDER);
    let notice = if modified(hwnd) {
        "Unsaved changes"
    } else if hub.notice.is_empty() {
        "Settings apply on the next launch."
    } else {
        &hub.notice
    };
    text(
        dc,
        rect(156, height - 90, (width - 520).max(100), 40),
        notice,
        if modified(hwnd) { ACCENT } else { MUTED },
        fonts[0],
    );
}

fn paint_calibration(
    dc: HDC,
    profile: &crate::profiles::Profile,
    fonts: &[HFONT; 4],
    width: i32,
    status: &crate::jobs::Status,
    started: Option<f64>,
) {
    text(
        dc,
        rect(52, 379, width - 104, 28),
        "Measure CPU/GPU sharing, CPU workers, speculative decoding and expert caching.",
        MUTED,
        fonts[0],
    );
    let current = status.busy || status.data["profile"].as_str() == Some(profile.file.as_str());
    let message = if current {
        status.message.clone()
    } else if let Some(last) = &profile.calibration {
        format!(
            "Last calibration: {} · {:.1} token/s",
            last["date"].as_str().unwrap_or(""),
            last["tok_s"].as_f64().unwrap_or(0.0)
        )
    } else {
        "Ready to calibrate this model. Allow about 15–30 minutes; the engine reloads several times.".into()
    };
    unsafe {
        let old = SelectObject(dc, fonts[0]);
        SetTextColor(
            dc,
            if current && status.error {
                theme::ERROR
            } else if status.busy {
                theme::WARNING
            } else {
                ACCENT
            },
        );
        SetBkMode(dc, TRANSPARENT as i32);
        let mut bounds = rect(52, 428, width - 104, 72);
        DrawTextW(
            dc,
            wide(&message).as_ptr(),
            -1,
            &mut bounds,
            DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, old);
    }
    let detail = if let Some(started) = started {
        let elapsed = (crate::data::now() - started).max(0.0) as u64;
        format!(
            "{} · elapsed {}:{:02}",
            status.data["stage"].as_str().unwrap_or("Preparing"),
            elapsed / 60,
            elapsed % 60
        )
    } else if let Some(last) = &profile.calibration {
        format!(
            "Calibrated at {} tokens · {} · {} request slots",
            last["context"].as_str().unwrap_or("engine default"),
            if last["vision"].as_bool() == Some(true) {
                "Vision"
            } else {
                "Text"
            },
            last["parallel"]
        )
    } else {
        "Results are saved to this hub profile only after a successful run.".into()
    };
    text(dc, rect(52, 512, width - 104, 30), &detail, MUTED, fonts[0]);
    text(
        dc,
        rect(52, 548, width - 104, 28),
        "Requires Strata stopped and saved settings. The model stays stopped afterwards.",
        MUTED,
        fonts[0],
    );
}

pub fn calibration_controls(hwnd: HWND, hub: &Hub, busy: bool, can_start: bool, has_log: bool) {
    unsafe {
        let available = !hub.profiles.is_empty();
        for id in [
            PROFILE,
            SAVE,
            RESET,
            ASYNC,
            PIN,
            RESIDENT,
            KV_GROW,
            KV_STREAM,
            PARALLEL,
            PORT,
            CONTEXT_PRESET,
            EXTRA,
        ] {
            EnableWindow(control(hwnd, id), (available && !busy) as i32);
        }
        for index in 0..FIELDS.len() {
            EnableWindow(
                control(hwnd, FIRST_FIELD + index),
                (available && !busy) as i32,
            );
        }
        EnableWindow(control(hwnd, OPTION), (available && !busy) as i32);
        for index in 0..OPTIONS.len() {
            EnableWindow(
                control(hwnd, FIRST_OPTION + index),
                (available && !busy) as i32,
            );
        }
        EnableWindow(
            control(hwnd, VISION),
            (available && !busy && hub.profiles[hub.selected].has_vision) as i32,
        );
        let can_start =
            can_start && available && hub.profiles[hub.selected].settings.engine == Engine::Qwen;
        EnableWindow(control(hwnd, CALIBRATE), can_start as i32);
        EnableWindow(control(hwnd, CALIBRATION_LOG), has_log as i32);
        EnableWindow(
            control(hwnd, RESET_CALIBRATION),
            (can_start && available && hub.profiles[hub.selected].calibration.is_some()) as i32,
        );
    }
}

pub fn fill(hwnd: HWND, hub: &Hub) {
    unsafe {
        SetPropW(hwnd, wide("StrataFilling").as_ptr(), 1usize as HANDLE);
        SendMessageW(control(hwnd, PROFILE), CB_RESETCONTENT, 0, 0);
        for profile in &hub.profiles {
            SendMessageW(
                control(hwnd, PROFILE),
                CB_ADDSTRING,
                0,
                wide(&profile.label).as_ptr() as LPARAM,
            );
        }
        SendMessageW(control(hwnd, PROFILE), CB_SETCURSEL, hub.selected, 0);
        let Some(profile) = hub.profiles.get(hub.selected) else {
            SetPropW(hwnd, wide("StrataNoProfiles").as_ptr(), 1usize as HANDLE);
            RemovePropW(hwnd, wide("StrataFilling").as_ptr());
            saved(hwnd);
            visible(hwnd, false);
            return;
        };
        RemovePropW(hwnd, wide("StrataNoProfiles").as_ptr());
        for (name, enabled) in [
            (
                "StrataDeepSeek",
                profile.settings.engine == Engine::DeepSeek,
            ),
            ("StrataExperimentalAvailable", profile.has_experimental),
        ] {
            if enabled {
                SetPropW(hwnd, wide(name).as_ptr(), 1usize as HANDLE);
            } else {
                RemovePropW(hwnd, wide(name).as_ptr());
            }
        }
        if profile.settings.engine == Engine::DeepSeek && [3, 4].contains(&active_section(hwnd)) {
            SetWindowLongPtrW(control(hwnd, ENGINE), GWLP_USERDATA, 0);
        }
        if profile.has_vision {
            SetPropW(
                hwnd,
                wide("StrataVisionAvailable").as_ptr(),
                1usize as HANDLE,
            );
        } else {
            RemovePropW(hwnd, wide("StrataVisionAvailable").as_ptr());
        }
        let settings = &profile.settings;
        crate::gpu_ui::fill(hwnd, settings, &profile.gpu_description);
        let scopes = control(hwnd, ENVIRONMENT_SCOPE);
        SendMessageW(scopes, CB_RESETCONTENT, 0, 0);
        for (index, label) in ENVIRONMENT_SCOPES.iter().enumerate() {
            if index % 2 == 1 && !profile.has_experimental {
                continue;
            }
            let row = SendMessageW(scopes, CB_ADDSTRING, 0, wide(label).as_ptr() as LPARAM);
            SendMessageW(scopes, CB_SETITEMDATA, row as usize, index as isize);
        }
        set_text(
            hwnd,
            RUN,
            if profile.has_experimental {
                "Start standard"
            } else {
                "Start"
            },
        );
        SendMessageW(control(hwnd, ENVIRONMENT_SCOPE), CB_SETCURSEL, 0, 0);
        for (index, environment) in settings.environment.iter().enumerate() {
            let value = environment
                .as_ref()
                .map(|variables| {
                    variables
                        .iter()
                        .map(|(name, value)| format!("{name}={value}"))
                        .collect::<Vec<_>>()
                        .join("\r\n")
                })
                .unwrap_or_default();
            set_text(hwnd, FIRST_ENVIRONMENT + index, &value);
            if index % 2 == 1 {
                SendMessageW(
                    control(hwnd, FIRST_INHERIT + index / 2),
                    BM_SETCHECK,
                    if environment.is_none() {
                        BST_CHECKED
                    } else {
                        BST_UNCHECKED
                    } as WPARAM,
                    0,
                );
            }
        }
        SendMessageW(control(hwnd, OPTION), CB_RESETCONTENT, 0, 0);
        for (index, option) in OPTIONS
            .iter()
            .enumerate()
            .filter(|(_, o)| o.engine == settings.engine)
        {
            let label = format!("{} ({})", option.label, option.key);
            let row = SendMessageW(
                control(hwnd, OPTION),
                CB_ADDSTRING,
                0,
                wide(&label).as_ptr() as LPARAM,
            );
            SendMessageW(
                control(hwnd, OPTION),
                CB_SETITEMDATA,
                row as usize,
                index as isize,
            );
        }
        SendMessageW(control(hwnd, OPTION), CB_SETCURSEL, 0, 0);
        for (index, option) in OPTIONS.iter().enumerate() {
            set_text(
                hwnd,
                FIRST_OPTION + index,
                settings
                    .advanced
                    .get(option.key)
                    .map(String::as_str)
                    .unwrap_or(""),
            );
        }
        set_text(hwnd, PORT, &settings.port.to_string());
        SendMessageW(
            control(hwnd, PARALLEL),
            CB_SETCURSEL,
            (settings.parallel - 1) as WPARAM,
            0,
        );
        for (i, value) in settings.values.iter().enumerate() {
            set_text(hwnd, FIRST_FIELD + i, value);
            if !FIELDS[i].choices.is_empty() {
                SendMessageW(control(hwnd, FIRST_FIELD + i), CB_SETEDITSEL, 0, 0);
            }
        }
        sync_context(hwnd);
        for (id, checked) in [
            (PIN, settings.pin_by_need),
            (RESIDENT, settings.resident),
            (KV_GROW, settings.kv_grow),
            (KV_STREAM, settings.kv_stream),
            (VISION, settings.vision),
        ] {
            SendMessageW(
                control(hwnd, id),
                BM_SETCHECK,
                if checked { BST_CHECKED } else { BST_UNCHECKED } as WPARAM,
                0,
            );
        }
        SendMessageW(
            control(hwnd, ASYNC),
            CB_SETCURSEL,
            match settings.adapt_async {
                None => 0,
                Some(true) => 1,
                Some(false) => 2,
            },
            0,
        );
        EnableWindow(control(hwnd, VISION), profile.has_vision as i32);
        set_text(
            hwnd,
            EXTRA,
            &settings
                .extra_args
                .replace("\r\n", "\n")
                .replace('\n', "\r\n"),
        );
        RemovePropW(hwnd, wide("StrataFilling").as_ptr());
        resize(hwnd);
        visible(
            hwnd,
            GetPropW(hwnd, wide("StrataHubView").as_ptr()) as usize == 1,
        );
    }
}
pub fn read(hwnd: HWND) -> Result<Settings> {
    unsafe {
        let checked =
            |id| SendMessageW(control(hwnd, id), BM_GETCHECK, 0, 0) == BST_CHECKED as LRESULT;
        let (gpu, gpu_keep_order) = crate::gpu_ui::read(hwnd)?;
        let settings = Settings {
            engine: engine(hwnd),
            gpu,
            gpu_keep_order,
            environment: {
                let mut environment = [None, None, None, None];
                for (index, entry) in environment.iter_mut().enumerate() {
                    let experimental =
                        !GetPropW(hwnd, wide("StrataExperimentalAvailable").as_ptr()).is_null();
                    if index % 2 == 0 || (experimental && !checked(FIRST_INHERIT + index / 2)) {
                        *entry = Some(
                            parse_environment(&get_text(hwnd, FIRST_ENVIRONMENT + index)).map_err(
                                |error| format!("{}: {error}", ENVIRONMENT_SCOPES[index]),
                            )?,
                        );
                    }
                }
                environment
            },
            advanced: OPTIONS
                .iter()
                .enumerate()
                .filter(|(_, o)| o.engine == engine(hwnd))
                .filter_map(|(index, option)| {
                    let value = get_text(hwnd, FIRST_OPTION + index).trim().to_owned();
                    if value.is_empty() {
                        None
                    } else {
                        Some((option.key.to_owned(), value))
                    }
                })
                .collect(),
            port: get_text(hwnd, PORT)
                .trim()
                .parse()
                .map_err(|_| "Server port must be between 1 and 65535")?,
            parallel: u32::try_from(SendMessageW(control(hwnd, PARALLEL), CB_GETCURSEL, 0, 0) + 1)?,
            values: (0..FIELDS.len())
                .map(|i| get_text(hwnd, FIRST_FIELD + i).trim().to_owned())
                .collect(),
            adapt_async: match SendMessageW(control(hwnd, ASYNC), CB_GETCURSEL, 0, 0) {
                0 => None,
                1 => Some(true),
                2 => Some(false),
                _ => return Err("Select Auto, On or Off for Adapt async".into()),
            },
            pin_by_need: checked(PIN),
            resident: checked(RESIDENT),
            kv_grow: checked(KV_GROW),
            kv_stream: checked(KV_STREAM),
            vision: checked(VISION),
            extra_args: get_text(hwnd, EXTRA),
        };
        settings.validate()?;
        Ok(settings)
    }
}
pub fn selected(hwnd: HWND) -> Result<usize> {
    unsafe {
        let selected = SendMessageW(control(hwnd, PROFILE), CB_GETCURSEL, 0, 0);
        if selected < 0 {
            return Err("No profile selected".into());
        }
        Ok(selected as usize)
    }
}
pub fn restore_selection(hwnd: HWND, index: usize) {
    unsafe {
        SendMessageW(control(hwnd, PROFILE), CB_SETCURSEL, index, 0);
    }
}
