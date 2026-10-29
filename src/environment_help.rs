use crate::{
    BG, MUTED, WHITE, data::Result, engine_options::Engine, hub_ui, rect, text, theme, wide,
};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, WindowsAndMessaging::*},
};

const CLASS: &str = "StrataEnvironmentReference";
const SEARCH: i32 = 1;
const CONTENT: i32 = 2;
const CATALOG: &str = include_str!("../assets/environment-variables.tsv");
const GROUPS: &[(&str, &str)] = &[
    ("qwen", "Qwen standard engine"),
    ("qwen-experimental", "Qwen experimental engine"),
    ("deepseek", "DeepSeek engine"),
    ("server", "Standard Python server"),
    ("server-experimental", "Experimental Python server"),
];

fn reference(group: usize, query: &str) -> (String, usize) {
    let query = query.trim().to_ascii_lowercase();
    let mut result = String::new();
    let mut count = 0;
    for line in CATALOG.lines().filter(|line| !line.starts_with('#')) {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 3, "Invalid bundled environment reference");
        if fields[0] != GROUPS[group].0 || !line.to_ascii_lowercase().contains(&query) {
            continue;
        }
        count += 1;
        result.push_str(&format!("{}\r\n  Source: {}\r\n\r\n", fields[1], fields[2]));
    }
    if count == 0 {
        result.push_str("No matching variables.");
    }
    (result, count)
}

fn populate(hwnd: HWND) {
    unsafe {
        let search = GetDlgItem(hwnd, SEARCH);
        let mut query = vec![0; GetWindowTextLengthW(search).max(0) as usize + 1];
        let copied = GetWindowTextW(search, query.as_mut_ptr(), query.len() as i32);
        let group = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
        let (body, count) = reference(
            group,
            &String::from_utf16_lossy(&query[..copied.max(0) as usize]),
        );
        SetWindowTextW(GetDlgItem(hwnd, CONTENT), wide(&body).as_ptr());
        SetWindowTextW(
            hwnd,
            wide(&format!(
                "Available variables: {} ({count})",
                GROUPS[group].1
            ))
            .as_ptr(),
        );
    }
}

pub fn tooltip(parent: HWND, target: HWND) -> Result<()> {
    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES,
        };
        if InitCommonControlsEx(&controls) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let tip = CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            null(),
            WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            parent,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if tip.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let tool = TTTOOLINFOW {
            // TTTOOLINFOW_V2_SIZE from the Windows SDK: fields through lParam.
            // The Hub uses Comctl32 v5 without a v6 activation manifest.
            cbSize: (std::mem::offset_of!(TTTOOLINFOW, lParam) + std::mem::size_of::<LPARAM>()) as u32,
            uFlags: TTF_IDISHWND | TTF_SUBCLASS,
            hwnd: parent,
            uId: target as usize,
            // The tooltip retains this pointer. The static literal outlives the control.
            lpszText: windows_sys::core::w!("Browse all known variable names for the selected engine/server scope.\nClick for the complete searchable list and source references.\nThe reference includes diagnostic and GPU-specific switches; availability depends on the installed version and backend.\nQwen: STRATA_PIN_BY_NEED and STRATA_KV_GROW use the Engine switches.").cast_mut(),
            ..std::mem::zeroed()
        };
        if SendMessageW(
            tip,
            TTM_ADDTOOLW,
            0,
            (&tool as *const TTTOOLINFOW) as LPARAM,
        ) == 0
        {
            DestroyWindow(tip);
            return Err("Could not attach the environment help tooltip".into());
        }
        SendMessageW(tip, TTM_SETMAXTIPWIDTH, 0, 520);
        SendMessageW(tip, TTM_SETDELAYTIME, TTDT_AUTOPOP as usize, 30000);
        Ok(())
    }
}

struct Request {
    group: usize,
    fonts: [HFONT; 4],
}

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let request = &*(*(l as *const CREATESTRUCTW))
                    .lpCreateParams
                    .cast::<Request>();
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, request.group as isize);
                let create = || -> Result<()> {
                    theme::init(hwnd, request.fonts[0])?;
                    hub_ui::add(
                        hwnd,
                        SEARCH as usize,
                        "EDIT",
                        "",
                        WS_VISIBLE | ES_AUTOHSCROLL as u32,
                        request.fonts[0],
                    )?;
                    let content = hub_ui::add(
                        hwnd,
                        CONTENT as usize,
                        "EDIT",
                        "",
                        WS_VISIBLE
                            | WS_VSCROLL
                            | WS_HSCROLL
                            | ES_MULTILINE as u32
                            | ES_READONLY as u32
                            | ES_AUTOHSCROLL as u32
                            | ES_AUTOVSCROLL as u32,
                        request.fonts[3],
                    )?;
                    SendMessageW(content, EM_SETLIMITTEXT, 1024 * 1024, 0);
                    populate(hwnd);
                    Ok(())
                };
                if let Err(error) = create() {
                    crate::message(hwnd, &error.to_string());
                    return -1;
                }
                0
            }
            WM_SIZE => {
                let mut bounds: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut bounds);
                MoveWindow(GetDlgItem(hwnd, SEARCH), 32, 122, bounds.right - 64, 24, 1);
                MoveWindow(
                    GetDlgItem(hwnd, CONTENT),
                    32,
                    170,
                    bounds.right - 64,
                    bounds.bottom - 202,
                    1,
                );
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_GETMINMAXINFO => {
                (*(l as *mut MINMAXINFO)).ptMinTrackSize = POINT { x: 760, y: 480 };
                0
            }
            WM_COMMAND if (w & 0xffff) as i32 == SEARCH && (w >> 16) as u32 == EN_CHANGE => {
                populate(hwnd);
                0
            }
            WM_COMMAND if w == IDCANCEL as usize => {
                DestroyWindow(hwnd);
                0
            }
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut bounds: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut bounds);
                crate::fill(dc, bounds, BG);
                let font = GetPropW(hwnd, wide("StrataFont").as_ptr()) as HFONT;
                text(
                    dc,
                    rect(24, 16, bounds.right - 48, 26),
                    "Variable reference · Source snapshots: 27 October 2026",
                    WHITE,
                    font,
                );
                text(
                    dc,
                    rect(24, 48, bounds.right - 48, 24),
                    "Variable names from Qwen, DeepSeek and Python server sources.",
                    MUTED,
                    font,
                );
                text(
                    dc,
                    rect(24, 74, bounds.right - 48, 24),
                    "Includes diagnostics and GPU-specific switches. Installed versions/backends may differ.",
                    MUTED,
                    font,
                );
                theme::input_frame(
                    hwnd,
                    SEARCH as usize,
                    dc,
                    rect(24, 114, bounds.right - 48, 40),
                );
                theme::input_frame(
                    hwnd,
                    CONTENT as usize,
                    dc,
                    rect(24, 162, bounds.right - 48, bounds.bottom - 186),
                );
                EndPaint(hwnd, &ps);
                0
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => theme::color(hwnd, w as HDC, false),
            WM_ERASEBKGND => 1,
            WM_DESTROY => {
                RemovePropW(GetWindow(hwnd, GW_OWNER), wide(CLASS).as_ptr());
                theme::dispose(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

pub fn show(parent: HWND, fonts: [HFONT; 4], engine: Engine, scope: usize) -> Result<()> {
    let group = if scope >= 2 {
        scope + 1
    } else if engine == Engine::DeepSeek {
        2
    } else {
        scope
    };
    unsafe {
        let existing = GetPropW(parent, wide(CLASS).as_ptr()) as HWND;
        if !existing.is_null() {
            SetWindowLongPtrW(existing, GWLP_USERDATA, group as isize);
            populate(existing);
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
            return Ok(());
        }
        let class_name = wide(CLASS);
        let instance = GetModuleHandleW(null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hIcon: theme::icon(32),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(std::io::Error::last_os_error().into());
        }
        let request = Request { group, fonts };
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class_name.as_ptr(),
            wide("Available variables").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1040,
            740,
            parent,
            null_mut(),
            instance,
            (&request as *const Request).cast(),
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        if let Err(error) = theme::dark_title(hwnd) {
            DestroyWindow(hwnd);
            return Err(error);
        }
        if SetPropW(parent, class_name.as_ptr(), hwnd as HANDLE) == 0 {
            let error = std::io::Error::last_os_error();
            DestroyWindow(hwnd);
            return Err(error.into());
        }
        SendMessageW(
            GetDlgItem(hwnd, SEARCH),
            EM_SETCUEBANNER,
            0,
            wide("Search variable names or source paths...").as_ptr() as LPARAM,
        );
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(GetDlgItem(hwnd, SEARCH));
        Ok(())
    }
}

pub fn translate(parent: HWND, msg: &MSG) -> bool {
    unsafe {
        let hwnd = GetPropW(parent, wide(CLASS).as_ptr()) as HWND;
        !hwnd.is_null() && IsDialogMessageW(hwnd, msg) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_tooltip_attaches_to_native_control() -> Result<()> {
        unsafe {
            let parent = CreateWindowExW(
                0,
                windows_sys::core::w!("STATIC"),
                null(),
                WS_POPUP,
                0,
                0,
                320,
                200,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            if parent.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            let result = (|| -> Result<()> {
                let target = hub_ui::add(
                    parent,
                    321,
                    "BUTTON",
                    "Available variables (?)",
                    0,
                    null_mut(),
                )?;
                tooltip(parent, target)
            })();
            DestroyWindow(parent);
            result
        }
    }

    #[test]
    fn environment_reference_covers_scopes_and_filters_names() {
        for group in 0..GROUPS.len() {
            assert!(reference(group, "").1 > 0);
            assert_eq!(reference(group, "not-a-variable-name").1, 0);
        }
        assert_eq!(reference(0, "strata_stager_sleep").1, 1);
        assert_eq!(reference(1, "STRATA_VRAM_BUDGET").1, 1);
        assert!(reference(2, "DS4_").1 > 0);
        assert_eq!(reference(3, "STRATA_HTTP_BACKLOG").1, 1);
        assert_eq!(reference(4, "STRATA_ENGINE_READY_S").1, 1);
    }
}
