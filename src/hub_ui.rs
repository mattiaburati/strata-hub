use crate::{
    ACCENT, MUTED, PANEL, WHITE,
    data::Result,
    profiles::{FIELDS, Hub, Settings},
    rect, text, theme, wide,
};
use std::ptr::null;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{BST_CHECKED, BST_UNCHECKED, EM_SETLIMITTEXT},
        Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled},
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
const ASYNC: usize = 350;
const PIN: usize = 351;
const RESIDENT: usize = 352;
const VISION: usize = 353;
const PARALLEL: usize = 354;
const PORT: usize = 355;
const CONTEXT_PRESET: usize = 356;
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
const FIRST_FIELD: usize = 400;
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
        for (id, label) in [
            (ASYNC, "Adapt async"),
            (PIN, "Pin by need"),
            (RESIDENT, "Resident experts"),
            (VISION, "Vision support"),
        ] {
            let checkbox = add(hwnd, id, "BUTTON", label, BS_AUTOCHECKBOX as u32, fonts[0])?;
            theme::checkbox(checkbox)?;
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
        for (id, label) in [
            (RUN, "Start model"),
            (STOP, "Stop"),
            (SAVE, "Save changes"),
            (RESET, "Reset"),
            (LOG, "Open log"),
            (START_LOG, "Startup details"),
            (EXIT, "Exit hub"),
            (ENGINE, "Engine"),
            (SAMPLING, "Sampling"),
            (ADVANCED, "Advanced"),
        ] {
            add(hwnd, id, "BUTTON", label, BS_OWNERDRAW as u32, fonts[0])?;
        }
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
        for id in ENGINE..=ADVANCED {
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
    unsafe {
        let is_field = (FIRST_FIELD..FIRST_FIELD + FIELDS.len()).contains(&id);
        let checkbox = [ASYNC, PIN, RESIDENT, VISION].contains(&id);
        if !is_field
            && !checkbox
            && id != EXTRA
            && id != PARALLEL
            && id != PORT
            && id != CONTEXT_PRESET
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
        let combo = id == PARALLEL || (is_field && !FIELDS[id - FIRST_FIELD].choices.is_empty());
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
            400 + row * 82,
            cell,
            38,
        )
    } else {
        let i = (index - 7) as i32;
        let cell = (width - 152) / 3;
        rect(52 + (i % 3) * (cell + 24), 400 + (i / 3) * 82, cell, 38)
    }
}
pub fn visible(hwnd: HWND, show: bool) {
    let show = show && unsafe { GetPropW(hwnd, wide("StrataNoProfiles").as_ptr()).is_null() };
    unsafe {
        let section = active_section(hwnd);
        for id in [
            PROFILE, RUN, STOP, SAVE, RESET, LOG, START_LOG, ENGINE, SAMPLING, ADVANCED,
        ] {
            ShowWindow(control(hwnd, id), if show { SW_SHOW } else { SW_HIDE });
        }
        for i in 0..FIELDS.len() {
            ShowWindow(
                control(hwnd, FIRST_FIELD + i),
                if show && ((i < 7 && section == 0) || (i >= 7 && section == 1)) {
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
        for id in [ASYNC, PIN, RESIDENT, PARALLEL, PORT, CONTEXT_PRESET] {
            ShowWindow(
                control(hwnd, id),
                if show && section == 0 {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
        ShowWindow(
            control(hwnd, VISION),
            if show && IsWindowEnabled(control(hwnd, VISION)) != 0 {
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
        for i in 0..3 {
            move_control(hwnd, ENGINE + i, rect(28 + i as i32 * 128, 266, 120, 38));
        }
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
        move_control(hwnd, CONTEXT_PRESET, rect(54, 402, 82, 320));
        move_control(hwnd, PARALLEL, rect(642, 606, (w - 694).min(260), 240));
        move_control(hwnd, START_LOG, rect(w - 196, 207, 144, 34));
        move_control(hwnd, PORT, rect(288, 613, 114, 22));
        for (id, x, y) in [
            (PIN, 52, 536),
            (RESIDENT, 52 + cell + 16, 536),
            (ASYNC, 52 + card + 20 + cell + 16, 488),
            (VISION, 52, 212),
        ] {
            move_control(hwnd, id, rect(x, y, 18, 26));
        }
        move_control(hwnd, EXTRA, rect(64, 380, w - 128, 254));
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
    launch_detail: &str,
    launch_color: u32,
    stage: usize,
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
    let cell = (card - 64) / 2;
    theme::rounded(
        dc,
        rect(28, 112, width - 56, 136),
        PANEL,
        Some(theme::BORDER),
        16,
    );
    text(dc, rect(52, 126, 450, 24), "MODEL PROFILE", MUTED, fonts[0]);
    if profile.has_vision {
        text(
            dc,
            rect(82, 212, 100, 26),
            "Vision support",
            WHITE,
            fonts[0],
        );
    } else {
        theme::rounded(dc, rect(52, 212, 96, 26), theme::INPUT, None, 10);
        text(dc, rect(64, 212, 80, 26), "Text only", MUTED, fonts[0]);
    }
    text(
        dc,
        rect(208, 212, width - 432, 26),
        launch_detail,
        launch_color,
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
            theme::rounded(dc, rect(x, 320, card, 252), PANEL, Some(theme::BORDER), 16);
            text(dc, rect(x + 24, 336, card - 48, 32), title, WHITE, fonts[2]);
        }
        for (id, label, x, y) in [
            (PIN, "Pin by need", 52, 536),
            (RESIDENT, "Resident experts", 52 + cell + 16, 536),
            (ASYNC, "Adapt async", 52 + card + 20 + cell + 16, 488),
        ] {
            text(dc, rect(x + 30, y, cell - 22, 26), label, WHITE, fonts[0]);
            let _ = id;
        }
        text(
            dc,
            rect(52 + card + 20 + cell + 16, 454, cell, 22),
            "Adaptation",
            MUTED,
            fonts[0],
        );
        theme::rounded(
            dc,
            rect(28, 588, width - 56, 68),
            PANEL,
            Some(theme::BORDER),
            16,
        );
        text(dc, rect(52, 604, 140, 36), "Server", WHITE, fonts[2]);
        text(dc, rect(216, 606, 54, 34), "Port", MUTED, fonts[0]);
        theme::input_frame(hwnd, PORT, dc, rect(276, 604, 140, 38));
        text(
            dc,
            rect(448, 606, 180, 34),
            "Concurrent requests",
            MUTED,
            fonts[0],
        );
        theme::input_frame(
            hwnd,
            PARALLEL,
            dc,
            rect(640, 604, (width - 690).min(264), 38),
        );
    } else {
        theme::rounded(
            dc,
            rect(28, 320, width - 56, 336),
            PANEL,
            Some(theme::BORDER),
            16,
        );
        text(
            dc,
            rect(52, 336, width - 104, 32),
            if section == 1 {
                "Default sampling"
            } else {
                "Engine arguments"
            },
            WHITE,
            fonts[2],
        );
    }
    if section < 2 {
        for (i, field) in FIELDS.iter().enumerate() {
            if (i < 7) != (section == 0) {
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
    } else {
        theme::input_frame(hwnd, EXTRA, dc, rect(52, 372, width - 104, 270));
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
        let settings = &profile.settings;
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
            (ASYNC, settings.adapt_async),
            (PIN, settings.pin_by_need),
            (RESIDENT, settings.resident),
            (VISION, settings.vision),
        ] {
            SendMessageW(
                control(hwnd, id),
                BM_SETCHECK,
                if checked { BST_CHECKED } else { BST_UNCHECKED } as WPARAM,
                0,
            );
        }
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
        let settings = Settings {
            port: get_text(hwnd, PORT)
                .trim()
                .parse()
                .map_err(|_| "Server port must be between 1 and 65535")?,
            parallel: u32::try_from(SendMessageW(control(hwnd, PARALLEL), CB_GETCURSEL, 0, 0) + 1)?,
            values: (0..FIELDS.len())
                .map(|i| get_text(hwnd, FIRST_FIELD + i).trim().to_owned())
                .collect(),
            adapt_async: checked(ASYNC),
            pin_by_need: checked(PIN),
            resident: checked(RESIDENT),
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
