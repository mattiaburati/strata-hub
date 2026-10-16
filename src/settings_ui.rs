use crate::{
    BG, MUTED, WHITE,
    config::{self, FIELDS, Paths},
    data::Result,
    fill, hub_ui, installer,
    jobs::Job,
    message, rect, text, theme, wide,
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT},
        Input::KeyboardAndMouse::EnableWindow,
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "StrataHubSettings";
const FIRST: usize = 800;
const DEFAULTS: usize = 810;
const SAVE: usize = 811;
const CANCEL: usize = 812;
const INSTALL: usize = 814;
const LOG: usize = 815;
const MODEL: usize = 816;
struct Editor {
    file: PathBuf,
    values: Vec<String>,
    fonts: [HFONT; 3],
    saved: bool,
    installation: Option<Job>,
    started: Option<Instant>,
    paths_applied: bool,
}
fn read_paths(hwnd: HWND, file: &Path) -> Result<Paths> {
    let mut value = json!({"version":1});
    for (i, (key, _)) in FIELDS.iter().enumerate() {
        value[key] = json!(get_text(hwnd, FIRST + i));
    }
    Paths::from_json(&value, file)
}
fn busy(editor: &Editor) -> bool {
    editor
        .installation
        .as_ref()
        .is_some_and(|job| job.status().busy)
}
fn enabled(hwnd: HWND, editor: &Editor) {
    unsafe {
        let running = busy(editor);
        for id in (FIRST..FIRST + FIELDS.len()).chain([DEFAULTS, SAVE, CANCEL, MODEL]) {
            EnableWindow(GetDlgItem(hwnd, id as i32), (!running) as i32);
        }
        let present = read_paths(hwnd, &editor.file)
            .is_ok_and(|paths| paths.strata_dir.join("engine/BUILD.json").is_file());
        EnableWindow(
            GetDlgItem(hwnd, INSTALL as i32),
            (!running && !present) as i32,
        );
        if !running {
            KillTimer(hwnd, 1);
        }
        InvalidateRect(hwnd, null(), 0);
    }
}
fn get_text(hwnd: HWND, id: usize) -> String {
    unsafe {
        let item = GetDlgItem(hwnd, id as i32);
        let mut buffer = vec![0; GetWindowTextLengthW(item).max(0) as usize + 1];
        let copied = GetWindowTextW(item, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }
}
fn write_text(hwnd: HWND, id: usize, value: &str) {
    unsafe {
        SetWindowTextW(GetDlgItem(hwnd, id as i32), wide(value).as_ptr());
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Editor;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, msg, w, l);
        }
        match msg {
            WM_CREATE => {
                let editor = &*pointer;
                let create = || -> Result<()> {
                    theme::init(hwnd, editor.fonts[0])?;
                    for i in 0..FIELDS.len() {
                        let item = hub_ui::add(
                            hwnd,
                            FIRST + i,
                            "EDIT",
                            &editor.values[i],
                            WS_VISIBLE | ES_AUTOHSCROLL as u32,
                            editor.fonts[0],
                        )?;
                        SendMessageW(
                            item,
                            windows_sys::Win32::UI::Controls::EM_SETLIMITTEXT,
                            32767,
                            0,
                        );
                        MoveWindow(item, 312, 108 + i as i32 * 44, 710, 24, 0);
                    }
                    for (id, label, x, width) in [
                        (DEFAULTS, "Use Strata defaults", 32, 220),
                        (SAVE, "Save settings", 674, 174),
                        (CANCEL, "Cancel", 864, 168),
                    ] {
                        let item = hub_ui::add(
                            hwnd,
                            id,
                            "BUTTON",
                            label,
                            WS_VISIBLE | BS_OWNERDRAW as u32,
                            editor.fonts[0],
                        )?;
                        MoveWindow(item, x, 580, width, 42, 0);
                    }
                    let combo = hub_ui::add(
                        hwnd,
                        MODEL,
                        "COMBOBOX",
                        "",
                        WS_VISIBLE
                            | CBS_DROPDOWNLIST as u32
                            | CBS_OWNERDRAWFIXED as u32
                            | CBS_HASSTRINGS as u32
                            | WS_VSCROLL,
                        editor.fonts[0],
                    )?;
                    for (_, label) in installer::MODELS {
                        SendMessageW(combo, CB_ADDSTRING, 0, wide(label).as_ptr() as LPARAM);
                    }
                    SendMessageW(combo, CB_SETCURSEL, 0, 0);
                    MoveWindow(combo, 50, 470, 430, 240, 0);
                    for (id, label, x, width) in [
                        (INSTALL, "Install Strata", 504, 214),
                        (LOG, "Open install log", 738, 274),
                    ] {
                        let item = hub_ui::add(
                            hwnd,
                            id,
                            "BUTTON",
                            label,
                            WS_VISIBLE | BS_OWNERDRAW as u32,
                            editor.fonts[0],
                        )?;
                        MoveWindow(item, x, 468, width, 42, 0);
                    }
                    enabled(hwnd, editor);
                    Ok(())
                };
                if let Err(error) = create() {
                    message(hwnd, &error.to_string());
                    return -1;
                }
                0
            }
            WM_PAINT => {
                let editor = &*pointer;
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut bounds: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut bounds);
                fill(dc, bounds, BG);
                text(
                    dc,
                    rect(32, 22, 990, 42),
                    "Strata Hub settings",
                    WHITE,
                    editor.fonts[1],
                );
                text(
                    dc,
                    rect(32, 76, 990, 28),
                    "Choose folders for your installation, models and usage data. Relative paths use the settings file folder.",
                    MUTED,
                    editor.fonts[0],
                );
                for (i, (_, label)) in FIELDS.iter().enumerate() {
                    let y = 102 + i as i32 * 44;
                    text(dc, rect(32, y, 252, 34), label, WHITE, editor.fonts[0]);
                    theme::input_frame(hwnd, FIRST + i, dc, rect(300, y, 734, 36));
                }
                theme::rounded(
                    dc,
                    rect(32, 376, 1000, 180),
                    crate::PANEL,
                    Some(theme::BORDER),
                    12,
                );
                text(
                    dc,
                    rect(50, 390, 700, 32),
                    "Engine setup",
                    WHITE,
                    editor.fonts[2],
                );
                theme::badge(
                    dc,
                    rect(836, 392, 176, 28),
                    "Official stable release",
                    MUTED,
                    editor.fonts[0],
                );
                text(
                    dc,
                    rect(50, 424, 950, 24),
                    "Git required · New or empty folder · Python and build tools installed by setup",
                    MUTED,
                    editor.fonts[0],
                );
                text(
                    dc,
                    rect(50, 446, 950, 24),
                    "70–120 GB download · Text only · Model stays stopped after setup",
                    MUTED,
                    editor.fonts[0],
                );
                let (notice, color) = if let Some(job) = &editor.installation {
                    let status = job.status();
                    let elapsed = editor
                        .started
                        .map(|time| time.elapsed().as_secs())
                        .unwrap_or(0);
                    (
                        if status.busy {
                            format!(
                                "{}  |  {}m {:02}s",
                                status.message,
                                elapsed / 60,
                                elapsed % 60
                            )
                        } else {
                            status.message
                        },
                        if status.error {
                            theme::ERROR
                        } else {
                            crate::ACCENT
                        },
                    )
                } else {
                    match read_paths(hwnd, &editor.file).and_then(|paths| paths.validate()) {
                        Ok(()) => (
                            "Strata detected. All required paths are valid.".into(),
                            crate::ACCENT,
                        ),
                        Err(error) => (error.to_string(), theme::WARNING),
                    }
                };
                text(dc, rect(50, 520, 950, 30), &notice, color, editor.fonts[0]);
                text(
                    dc,
                    rect(32, 636, 990, 28),
                    &format!("Settings file: {}", editor.file.display()),
                    MUTED,
                    editor.fonts[0],
                );
                EndPaint(hwnd, &ps);
                0
            }
            WM_DRAWITEM => theme::draw_item(hwnd, &*(l as *const DRAWITEMSTRUCT)),
            WM_MEASUREITEM => {
                (*(l as *mut MEASUREITEMSTRUCT)).itemHeight = 30;
                1
            }
            WM_CTLCOLOREDIT => theme::color(hwnd, w as HDC, false),
            WM_ERASEBKGND => 1,
            WM_COMMAND => {
                let id = (w & 0xffff) as usize;
                if busy(&*pointer) && id != LOG {
                    return 0;
                }
                match id {
                    DEFAULTS => {
                        let defaults = || -> Result<()> {
                            let base = (*pointer)
                                .file
                                .parent()
                                .ok_or("Invalid settings location")?;
                            let strata = config::resolve(base, &get_text(hwnd, FIRST))?;
                            write_text(
                                hwnd,
                                FIRST + 1,
                                &strata.join(".venv/Scripts/python.exe").to_string_lossy(),
                            );
                            write_text(
                                hwnd,
                                FIRST + 2,
                                &strata
                                    .parent()
                                    .ok_or("Strata folder has no parent")?
                                    .join("Strata-data")
                                    .to_string_lossy(),
                            );
                            write_text(hwnd, FIRST + 3, &strata.to_string_lossy());
                            let data = config::resolve(base, &get_text(hwnd, FIRST + 4))?;
                            write_text(
                                hwnd,
                                FIRST + 5,
                                &data.join("qwen_pricing.json").to_string_lossy(),
                            );
                            Ok(())
                        };
                        if let Err(error) = defaults() {
                            message(hwnd, &error.to_string());
                        }
                    }
                    SAVE => {
                        let save = || -> Result<()> {
                            let paths = read_paths(hwnd, &(*pointer).file)?;
                            config::save(&(*pointer).file, &paths)?;
                            Ok(())
                        };
                        match save() {
                            Ok(()) => {
                                (*pointer).saved = true;
                                DestroyWindow(hwnd);
                            }
                            Err(error) => message(hwnd, &error.to_string()),
                        }
                    }
                    CANCEL => {
                        DestroyWindow(hwnd);
                    }
                    INSTALL => {
                        let install = || -> Result<Job> {
                            let paths = read_paths(hwnd, &(*pointer).file)?;
                            let model =
                                SendMessageW(GetDlgItem(hwnd, MODEL as i32), CB_GETCURSEL, 0, 0);
                            installer::start(
                                hwnd,
                                &(*pointer).file,
                                &paths,
                                usize::try_from(model)?,
                            )
                        };
                        match install() {
                            Ok(job) => {
                                (*pointer).installation = Some(job);
                                (*pointer).started = Some(Instant::now());
                                (*pointer).paths_applied = false;
                                if SetTimer(hwnd, 1, 1000, None) == 0 {
                                    message(
                                        hwnd,
                                        "Could not start the installation display timer. Progress remains available in the log.",
                                    );
                                }
                            }
                            Err(error) => message(hwnd, &error.to_string()),
                        }
                    }
                    LOG => {
                        let log = (*pointer)
                            .file
                            .parent()
                            .expect("Settings have a parent")
                            .join("strata-install.log");
                        if let Err(error) = crate::open_log(&log) {
                            message(hwnd, &error.to_string());
                        }
                    }
                    _ => {
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
                enabled(hwnd, &*pointer);
                0
            }
            crate::UPDATE | WM_TIMER => {
                let completed = (*pointer).installation.as_ref().is_some_and(|job| {
                    let status = job.status();
                    !status.busy && !status.error
                });
                if completed && !(*pointer).paths_applied {
                    (*pointer).paths_applied = true;
                    match read_paths(hwnd, &(*pointer).file) {
                        Ok(paths) => {
                            write_text(
                                hwnd,
                                FIRST + 1,
                                &paths
                                    .strata_dir
                                    .join(".venv/Scripts/python.exe")
                                    .to_string_lossy(),
                            );
                            write_text(hwnd, FIRST + 3, &paths.strata_dir.to_string_lossy());
                        }
                        Err(error) => message(hwnd, &error.to_string()),
                    }
                }
                enabled(hwnd, &*pointer);
                0
            }
            WM_CLOSE => {
                if busy(&*pointer) {
                    message(
                        hwnd,
                        "Wait for Strata installation to finish before closing settings. Progress and errors are available in the install log.",
                    );
                } else {
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_DESTROY => {
                KillTimer(hwnd, 1);
                theme::dispose(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
pub fn edit(parent: HWND, file: &Path, paths: Option<&Paths>) -> Result<bool> {
    unsafe {
        let base = file.parent().ok_or("Invalid settings location")?;
        let data = base.join("data");
        let strata = base.join("Strata");
        let values = paths.map(Paths::values).unwrap_or_else(|| {
            vec![
                strata.to_string_lossy().into_owned(),
                strata
                    .join(".venv/Scripts/python.exe")
                    .to_string_lossy()
                    .into_owned(),
                base.join("Strata-data").to_string_lossy().into_owned(),
                strata.to_string_lossy().into_owned(),
                data.to_string_lossy().into_owned(),
                data.join("qwen_pricing.json")
                    .to_string_lossy()
                    .into_owned(),
            ]
        });
        let font = |height, weight| {
            CreateFontW(
                height,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide("Segoe UI").as_ptr(),
            )
        };
        let mut editor = Editor {
            file: file.into(),
            values,
            fonts: [
                font(-15, FW_NORMAL as i32),
                font(-27, FW_SEMIBOLD as i32),
                font(-23, FW_SEMIBOLD as i32),
            ],
            saved: false,
            installation: None,
            started: None,
            paths_applied: false,
        };
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
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("Strata Hub settings").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1080,
            710,
            parent,
            null_mut(),
            instance,
            (&mut editor as *mut Editor).cast(),
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        theme::dark_title(hwnd)?;
        if !parent.is_null() {
            EnableWindow(parent, 0);
        }
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        let mut msg: MSG = std::mem::zeroed();
        while IsWindow(hwnd) != 0 {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            if result == 0 {
                break;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if !parent.is_null() {
            EnableWindow(parent, 1);
            SetForegroundWindow(parent);
        }
        for font in editor.fonts {
            DeleteObject(font);
        }
        Ok(editor.saved)
    }
}
