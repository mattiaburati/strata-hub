use crate::{ACCENT, BG, MUTED, PANEL, WHITE, data::Result, fill, rect, text, wide};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

pub const SIDE: i32 = 200;
pub fn icon(size: i32) -> HICON {
    // Shared Win32 resources are cached by the OS and do not require destruction.
    unsafe {
        LoadImageW(
            GetModuleHandleW(std::ptr::null()),
            1 as *const u16,
            IMAGE_ICON,
            size,
            size,
            LR_SHARED,
        ) as HICON
    }
}
pub const INPUT: u32 = 0x00231911;
pub const BORDER: u32 = 0x004e3c2b;
const SIDE_BG: u32 = 0x00201812;
const ACTIVE: u32 = 0x00353c20;

pub fn rounded(dc: HDC, r: RECT, color: u32, border: Option<u32>, radius: i32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID, 1, border.unwrap_or(color));
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);
        RoundRect(dc, r.left, r.top, r.right, r.bottom, radius, radius);
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}
pub fn init(hwnd: HWND, font: HFONT) -> Result<()> {
    unsafe {
        for (name, value) in [
            ("StrataFont", font as HANDLE),
            ("StrataInputBrush", CreateSolidBrush(INPUT) as HANDLE),
            ("StrataPanelBrush", CreateSolidBrush(PANEL) as HANDLE),
        ] {
            if value.is_null() || SetPropW(hwnd, wide(name).as_ptr(), value) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        view(hwnd, 1);
        metric_tab(hwnd, 0);
        Ok(())
    }
}
pub fn dark_title(hwnd: HWND) -> Result<()> {
    unsafe {
        let dark = 1i32;
        let result = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&dark as *const i32).cast(),
            4,
        );
        if result < 0 {
            return Err(format!("Cannot apply dark title bar: {result:#x}").into());
        }
        Ok(())
    }
}
pub fn dispose(hwnd: HWND) {
    unsafe {
        for name in ["StrataInputBrush", "StrataPanelBrush"] {
            let brush = RemovePropW(hwnd, wide(name).as_ptr());
            DeleteObject(brush);
        }
        for name in [
            "StrataFont",
            "StrataHubView",
            "StrataMetricTab",
            "StrataNoProfiles",
            "StrataDeepSeek",
            "StrataExperimentalAvailable",
        ] {
            RemovePropW(hwnd, wide(name).as_ptr());
        }
    }
}
pub fn view(hwnd: HWND, page: usize) {
    unsafe {
        SetPropW(hwnd, wide("StrataHubView").as_ptr(), page as HANDLE);
    }
}
pub fn metric_tab(hwnd: HWND, tab: usize) {
    unsafe {
        SetPropW(hwnd, wide("StrataMetricTab").as_ptr(), (tab + 1) as HANDLE);
    }
}
pub fn color(hwnd: HWND, dc: HDC, checkbox: bool) -> LRESULT {
    unsafe {
        SetTextColor(dc, WHITE);
        SetBkColor(dc, if checkbox { PANEL } else { INPUT });
        GetPropW(
            hwnd,
            wide(if checkbox {
                "StrataPanelBrush"
            } else {
                "StrataInputBrush"
            })
            .as_ptr(),
        ) as LRESULT
    }
}
pub fn sidebar(dc: HDC, height: i32, fonts: &[HFONT; 4]) {
    fill(dc, rect(0, 0, SIDE, height), SIDE_BG);
    fill(dc, rect(SIDE - 1, 0, 1, height), BORDER);
    unsafe {
        DrawIconEx(
            dc,
            22,
            26,
            icon(40),
            40,
            40,
            0,
            std::ptr::null_mut(),
            DI_NORMAL,
        );
    }
    text(dc, rect(72, 27, 115, 32), "Strata", WHITE, fonts[2]);
    text(dc, rect(73, 56, 105, 20), "LOCAL HUB", MUTED, fonts[0]);
    text(dc, rect(24, 110, 150, 20), "WORKSPACE", MUTED, fonts[0]);
    fill(dc, rect(24, 304, SIDE - 48, 1), BORDER);
    text(dc, rect(24, 308, 150, 20), "TOOLS", MUTED, fonts[0]);
}
pub const WARNING: u32 = 0x006ccbf2;
pub const ERROR: u32 = 0x008d8df4;
pub fn badge(dc: HDC, bounds: RECT, label: &str, color: u32, font: HFONT) {
    rounded(dc, bounds, INPUT, Some(BORDER), 12);
    text(
        dc,
        rect(
            bounds.left + 12,
            bounds.top,
            bounds.right - bounds.left - 24,
            bounds.bottom - bounds.top,
        ),
        label,
        color,
        font,
    );
}
pub fn status(
    dc: HDC,
    width: i32,
    label: &str,
    color: u32,
    rates: &str,
    prefill: &str,
    font: HFONT,
) {
    badge(dc, rect(width - 710, 29, 194, 30), prefill, WHITE, font);
    let speed_rect = rect(width - 506, 29, 338, 30);
    rounded(dc, speed_rect, PANEL, Some(BORDER), 14);
    text(
        dc,
        rect(speed_rect.left + 12, speed_rect.top, 314, 30),
        rates,
        WHITE,
        font,
    );
    let r = rect(width - 158, 29, 130, 30);
    rounded(dc, r, PANEL, None, 14);
    rounded(dc, rect(r.left + 14, r.top + 11, 8, 8), color, None, 8);
    text(dc, rect(r.left + 30, r.top, 95, 30), label, color, font);
}
pub fn input_frame(hwnd: HWND, id: usize, dc: HDC, r: RECT) {
    unsafe {
        let control = GetDlgItem(hwnd, id as i32);
        let focus = GetFocus();
        rounded(
            dc,
            r,
            INPUT,
            Some(if focus == control || IsChild(control, focus) != 0 {
                ACCENT
            } else {
                BORDER
            }),
            8,
        );
    }
}

pub fn combo(hwnd: HWND) -> Result<()> {
    unsafe {
        if SetWindowSubclass(hwnd, Some(combo_proc), 1, 0) == 0 {
            return Err("Cannot style the model/settings selector".into());
        }
        if GetWindowLongPtrW(hwnd, GWL_STYLE) & 3 == CBS_DROPDOWN as isize {
            let mut info: COMBOBOXINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of_val(&info) as u32;
            if GetComboBoxInfo(hwnd, &mut info) == 0 || info.hwndItem.is_null() {
                return Err("Cannot access the settings selector's edit control".into());
            }
            let style = GetWindowLongPtrW(info.hwndItem, GWL_STYLE);
            SetWindowLongPtrW(info.hwndItem, GWL_STYLE, style & !(ES_NOHIDESEL as isize));
        }
    }
    Ok(())
}

pub fn checkbox(hwnd: HWND) -> Result<()> {
    unsafe {
        if SetWindowSubclass(hwnd, Some(checkbox_proc), 1, 0) == 0 {
            return Err("Cannot style the model setting checkbox".into());
        }
    }
    Ok(())
}
// Keep the native checkbox's state, keyboard handling and accessibility provider.
unsafe extern "system" fn checkbox_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_PAINT {
            let mut paint: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut bounds: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut bounds);
            fill(dc, bounds, PANEL);
            let checked = SendMessageW(hwnd, BM_GETCHECK, 0, 0) == BST_CHECKED as LRESULT;
            let focused = GetFocus() == hwnd;
            rounded(
                dc,
                rect(1, 5, 16, 16),
                if checked { ACCENT } else { INPUT },
                Some(if checked || focused { ACCENT } else { BORDER }),
                4,
            );
            if checked {
                let pen = CreatePen(PS_SOLID, 2, BG);
                let old = SelectObject(dc, pen);
                MoveToEx(dc, 4, 12, std::ptr::null_mut());
                LineTo(dc, 8, 16);
                LineTo(dc, 14, 9);
                SelectObject(dc, old);
                DeleteObject(pen);
            }
            if focused {
                DrawFocusRect(dc, &rect(0, 2, bounds.right, bounds.bottom - 4));
            }
            let mut label = vec![0u16; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
            GetWindowTextW(hwnd, label.as_mut_ptr(), label.len() as i32);
            let font = GetPropW(GetParent(hwnd), wide("StrataFont").as_ptr()) as HFONT;
            let old = SelectObject(dc, font);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(
                dc,
                if IsWindowEnabled(hwnd) != 0 {
                    WHITE
                } else {
                    MUTED
                },
            );
            DrawTextW(
                dc,
                label.as_ptr(),
                -1,
                &mut rect(28, 0, bounds.right - 28, bounds.bottom),
                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            SelectObject(dc, old);
            EndPaint(hwnd, &paint);
            return 0;
        }
        if msg == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(checkbox_proc), 1);
        }
        let result = DefSubclassProc(hwnd, msg, w, l);
        if matches!(msg, BM_SETCHECK | WM_SETFOCUS | WM_KILLFOCUS | WM_ENABLE) {
            InvalidateRect(hwnd, std::ptr::null(), 0);
        }
        result
    }
}

// Retain the native combo's keyboard, dropdown and accessibility behavior.
// Only its classic border and arrow are repainted in the application palette.
unsafe extern "system" fn combo_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(combo_proc), 1);
        }
        let result = DefSubclassProc(hwnd, msg, w, l);
        if msg == WM_WINDOWPOSCHANGED
            && GetFocus() != hwnd
            && IsChild(hwnd, GetFocus()) == 0
            && GetWindowLongPtrW(hwnd, GWL_STYLE) & 3 == CBS_DROPDOWN as isize
        {
            // Resizing an editable native combo can select its text even without focus.
            SendMessageW(hwnd, CB_SETEDITSEL, 0, 0);
        }
        if msg == WM_PAINT {
            let dc = GetDC(hwnd);
            let mut bounds: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut bounds);
            let focus = GetFocus();
            let border = CreateSolidBrush(if focus == hwnd || IsChild(hwnd, focus) != 0 {
                ACCENT
            } else {
                BORDER
            });
            let inside = CreateSolidBrush(INPUT);
            FrameRect(dc, &bounds, border);
            let inner = rect(1, 1, bounds.right - 2, bounds.bottom - 2);
            FrameRect(dc, &inner, inside);
            let inner = rect(2, 2, bounds.right - 4, bounds.bottom - 4);
            FrameRect(dc, &inner, inside);
            fill(dc, rect(bounds.right - 21, 3, 18, bounds.bottom - 6), INPUT);
            let pen = CreatePen(PS_SOLID, 1, MUTED);
            let old = SelectObject(dc, pen);
            MoveToEx(
                dc,
                bounds.right - 15,
                bounds.bottom / 2 - 2,
                std::ptr::null_mut(),
            );
            LineTo(dc, bounds.right - 11, bounds.bottom / 2 + 2);
            LineTo(dc, bounds.right - 7, bounds.bottom / 2 - 2);
            SelectObject(dc, old);
            DeleteObject(pen);
            DeleteObject(border);
            DeleteObject(inside);
            ReleaseDC(hwnd, dc);
        } else if msg == WM_SETFOCUS || msg == WM_KILLFOCUS {
            InvalidateRect(hwnd, std::ptr::null(), 0);
        }
        result
    }
}
pub fn draw_item(parent: HWND, item: &DRAWITEMSTRUCT) -> LRESULT {
    unsafe {
        let font = GetPropW(parent, wide("StrataFont").as_ptr()) as HFONT;
        let r = item.rcItem;
        let dc = item.hDC;
        if item.CtlType == ODT_COMBOBOX {
            fill(
                dc,
                r,
                if item.itemState & ODS_SELECTED != 0 && item.itemState & ODS_COMBOBOXEDIT == 0 {
                    ACTIVE
                } else {
                    INPUT
                },
            );
            let mut label = String::new();
            if item.itemID != u32::MAX {
                let length = SendMessageW(item.hwndItem, CB_GETLBTEXTLEN, item.itemID as usize, 0);
                if length >= 0 {
                    let mut buf = vec![0u16; length as usize + 1];
                    SendMessageW(
                        item.hwndItem,
                        CB_GETLBTEXT,
                        item.itemID as usize,
                        buf.as_mut_ptr() as LPARAM,
                    );
                    label = String::from_utf16_lossy(&buf[..length as usize]);
                }
            }
            text(
                dc,
                rect(r.left + 10, r.top, r.right - r.left - 18, r.bottom - r.top),
                &label,
                if item.itemState & ODS_DISABLED != 0 {
                    MUTED
                } else {
                    WHITE
                },
                font,
            );
            return 1;
        }
        if item.CtlType != ODT_BUTTON {
            return 0;
        }
        let id = item.CtlID as usize;
        let page = GetPropW(parent, wide("StrataHubView").as_ptr()) as usize;
        let hub = page == 1;
        let selected = match id {
            110 => hub,
            111 => page == 2,
            112 => page == 3,
            113 => page == 4,
            115 => page == 5,
            101..=104 => GetPropW(parent, wide("StrataMetricTab").as_ptr()) as usize == id - 100,
            310..=312 => crate::hub_ui::active_section(parent) == id - 310,
            314 => crate::hub_ui::active_section(parent) == 3,
            318 => crate::hub_ui::active_section(parent) == 4,
            320 => crate::hub_ui::active_section(parent) == 5,
            322 => crate::hub_ui::active_section(parent) == 6,
            _ => false,
        };
        let tab = (101..=104).contains(&id)
            || (310..=312).contains(&id)
            || matches!(id, 314 | 318 | 320 | 322);
        let nav = (110..=115).contains(&id) || id == 306;
        let primary = id == 301
            || (id == 303 && crate::hub_ui::modified(parent))
            || id == 601
            || id == 811
            || id == 814
            || id == 903
            || (711..=731).contains(&id) && id % 10 == 1;
        let danger = id == 302 || id == 908;
        let pressed = item.itemState & ODS_SELECTED != 0;
        let disabled = item.itemState & ODS_DISABLED != 0;
        let color = if disabled && !nav && !tab {
            PANEL
        } else if primary {
            if pressed { 0x009bc45e } else { ACCENT }
        } else if danger {
            0x002e253b
        } else if selected {
            ACTIVE
        } else if pressed {
            INPUT
        } else if nav || tab {
            if nav { SIDE_BG } else { BG }
        } else {
            PANEL
        };
        fill(dc, r, if nav { SIDE_BG } else { BG });
        rounded(
            dc,
            rect(
                r.left + 1,
                r.top + 1,
                r.right - r.left - 2,
                r.bottom - r.top - 2,
            ),
            color,
            if (disabled || !primary) && !tab && !nav {
                Some(BORDER)
            } else {
                None
            },
            10,
        );
        if selected && nav {
            rounded(
                dc,
                rect(r.left + 1, r.top + 10, 3, r.bottom - r.top - 20),
                ACCENT,
                None,
                3,
            );
        }
        if selected && tab {
            rounded(
                dc,
                rect(r.left + 16, r.bottom - 3, r.right - r.left - 32, 2),
                ACCENT,
                None,
                2,
            );
        }
        let length = GetWindowTextLengthW(item.hwndItem).max(0) as usize;
        let mut buffer = vec![0u16; length + 1];
        GetWindowTextW(item.hwndItem, buffer.as_mut_ptr(), buffer.len() as i32);
        let label = String::from_utf16_lossy(&buffer[..length]);
        let foreground = if item.itemState & ODS_DISABLED != 0 {
            MUTED
        } else if primary {
            BG
        } else if danger {
            0x00b3a7f2
        } else if selected {
            ACCENT
        } else {
            WHITE
        };
        if nav {
            nav_icon(
                dc,
                r.left + 18,
                r.top + (r.bottom - r.top - 16) / 2,
                id,
                if selected { ACCENT } else { MUTED },
            );
        }
        let old = SelectObject(dc, font);
        SetTextColor(dc, foreground);
        SetBkMode(dc, TRANSPARENT as i32);
        let mut label_rect = rect(
            r.left + if nav { 44 } else { 8 },
            r.top + pressed as i32,
            r.right - r.left - if nav { 52 } else { 16 },
            r.bottom - r.top,
        );
        DrawTextW(
            dc,
            wide(&label).as_ptr(),
            -1,
            &mut label_rect,
            DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | if nav { DT_LEFT } else { DT_CENTER },
        );
        SelectObject(dc, old);
        if item.itemState & ODS_FOCUS != 0 {
            let focus = rect(
                r.left + 5,
                r.top + 5,
                r.right - r.left - 10,
                r.bottom - r.top - 10,
            );
            DrawFocusRect(dc, &focus);
        }
        1
    }
}
fn nav_icon(dc: HDC, x: i32, y: i32, id: usize, color: u32) {
    unsafe {
        let pen = CreatePen(PS_SOLID, 1, color);
        let old_pen = SelectObject(dc, pen);
        let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
        let line = |ax, ay, bx, by| {
            MoveToEx(dc, x + ax, y + ay, std::ptr::null_mut());
            LineTo(dc, x + bx, y + by);
        };
        match id {
            110 => {
                for row in [2, 7, 12] {
                    RoundRect(dc, x, y + row, x + 16, y + row + 4, 2, 2);
                }
            }
            115 => {
                for col in [0, 6, 12] {
                    Rectangle(dc, x + col, y + 2, x + col + 4, y + 15);
                }
            }
            111 => {
                line(0, 0, 0, 15);
                line(0, 15, 16, 15);
                line(3, 11, 7, 7);
                line(7, 7, 10, 9);
                line(10, 9, 16, 2);
            }
            112 => {
                line(8, 0, 8, 11);
                line(3, 7, 8, 12);
                line(8, 12, 13, 7);
                line(1, 12, 1, 16);
                line(1, 16, 15, 16);
                line(15, 16, 15, 12);
            }
            113 => {
                RoundRect(dc, x + 2, y + 3, x + 15, y + 14, 4, 4);
                line(8, 0, 8, 3);
                line(5, 7, 5, 9);
                line(11, 7, 11, 9);
                line(5, 11, 11, 11);
            }
            114 => {
                for (row, col) in [(3, 5), (8, 11), (13, 7)] {
                    line(0, row, 16, row);
                    Rectangle(dc, x + col - 2, y + row - 2, x + col + 2, y + row + 2);
                }
            }
            306 => {
                line(8, 0, 8, 8);
                Arc(
                    dc,
                    x + 1,
                    y + 2,
                    x + 15,
                    y + 16,
                    x + 3,
                    y + 3,
                    x + 13,
                    y + 3,
                );
            }
            _ => (),
        }
        SelectObject(dc, old_brush);
        SelectObject(dc, old_pen);
        DeleteObject(pen);
    }
}
