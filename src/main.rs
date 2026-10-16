#![windows_subsystem = "windows"]
mod config;
mod data;
mod hub_ui;
mod installer;
mod jobs;
mod launch;
mod models_ui;
mod profiles;
mod resources;
mod settings_ui;
mod theme;
mod tools_ui;

use data::{Result, Snapshot};
use std::{
    fs::OpenOptions,
    io::Write,
    net::TcpStream,
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr::{null, null_mut},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{
        LibraryLoader::GetModuleHandleW,
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::*,
    },
    UI::{
        Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, SetScrollInfo, ShowScrollBar},
        Shell::*,
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "StrataNativeDashboard";
const UPDATE: u32 = WM_APP + 1;
const TRAY: u32 = WM_APP + 2;
const START: u32 = WM_APP + 3;
const CREATE_NO_WINDOW: u32 = 0x08000000;
const BG: u32 = 0x0018120f;
const PANEL: u32 = 0x002a1f17;
const ACCENT: u32 = 0x00b6df76;
const WHITE: u32 = 0x00f7f1ea;
const MUTED: u32 = 0x00bfafa3;

#[derive(Clone, Copy)]
struct AgentSettings {
    port: u16,
    context: Option<u32>,
    vision: bool,
}
struct App {
    snapshot: Arc<Mutex<Snapshot>>,
    refresh: mpsc::Sender<()>,
    child: Option<Child>,
    tab: usize,
    scroll: usize,
    visible_rows: usize,
    fonts: [HFONT; 4],
    tray: NOTIFYICONDATAW,
    hub: profiles::Hub,
    show_hub: bool,
    tools_page: usize,
    updates: jobs::Job,
    agents: jobs::Job,
    pending_agent_settings: Option<AgentSettings>,
    syncing_agent_settings: bool,
    models: jobs::Job,
    library: models_ui::Library,
    installed_version: String,
    auth_profile: Arc<Mutex<Option<PathBuf>>>,
    launch: Option<launch::Launch>,
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
fn log_error(message: &str) {
    if config::loaded().is_none() {
        return;
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(data::root().join("native.log"))
    {
        let _ = writeln!(file, "{} {message}", data::local_date(data::now(), true));
    }
}
fn message(hwnd: HWND, text: &str) {
    unsafe {
        MessageBoxW(
            hwnd,
            wide(text).as_ptr(),
            wide("Strata Hub").as_ptr(),
            MB_OK,
        );
    }
}
fn grouped(value: i64) -> String {
    let digits = value.to_string();
    let mut text = String::new();
    for (index, c) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            text.push(',')
        }
        text.push(c)
    }
    text
}
fn money(value: f64, euro: bool) -> String {
    let text = format!("{value:.2}");
    if euro {
        format!("{text} €")
    } else {
        format!("{text} USD")
    }
}
fn elapsed(milliseconds: f64) -> String {
    let seconds = (milliseconds / 1000.0).round() as i64;
    if seconds < 60 {
        format!("{seconds} s")
    } else if seconds < 3600 {
        format!("{} min {} s", seconds / 60, seconds % 60)
    } else {
        format!("{} h {} min", seconds / 3600, seconds % 3600 / 60)
    }
}
fn own_memory() -> f64 {
    unsafe {
        let mut info: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        info.cb = std::mem::size_of_val(&info) as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut info, info.cb) != 0 {
            info.WorkingSetSize as f64 / 1048576.0
        } else {
            0.0
        }
    }
}
fn fill(dc: HDC, rect: RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, &rect, brush);
        DeleteObject(brush);
    }
}
fn text(dc: HDC, rect: RECT, value: &str, color: u32, font: HFONT) {
    unsafe {
        let old = SelectObject(dc, font);
        SetTextColor(dc, color);
        SetBkMode(dc, TRANSPARENT as i32);
        let mut r = rect;
        DrawTextW(
            dc,
            wide(value).as_ptr(),
            -1,
            &mut r,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(dc, old);
    }
}
fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
fn card(dc: HDC, r: RECT, label: &str, value: &str, detail: &str, fonts: &[HFONT; 4]) {
    theme::rounded(dc, r, PANEL, Some(theme::BORDER), 14);
    text(
        dc,
        rect(r.left + 16, r.top + 8, r.right - r.left - 30, 24),
        label,
        MUTED,
        fonts[0],
    );
    text(
        dc,
        rect(r.left + 16, r.top + 33, r.right - r.left - 30, 38),
        value,
        WHITE,
        fonts[1],
    );
    text(
        dc,
        rect(r.left + 16, r.top + 74, r.right - r.left - 30, 21),
        detail,
        MUTED,
        fonts[0],
    );
}
fn rows(snapshot: &Snapshot, tab: usize) -> (Vec<&'static str>, Vec<Vec<String>>, Vec<f64>) {
    match tab {
        0 => (
            vec![
                "Day",
                "Requests",
                "Output",
                "Fresh input",
                "Cached input",
                "Engine time",
                "Qwen € / USD",
            ],
            snapshot
                .daily
                .iter()
                .rev()
                .map(|g| {
                    let c = &g.counts;
                    let cost = snapshot.prices.costs(c)[0];
                    vec![
                        g.name.clone(),
                        grouped(c.requests),
                        grouped(c.output),
                        grouped(c.prompt - c.cached),
                        grouped(c.cached),
                        elapsed(c.prompt_ms + c.decode_ms),
                        format!(
                            "{} / {}",
                            money(cost / snapshot.prices.usd_per_eur, true),
                            money(cost, false)
                        ),
                    ]
                })
                .collect(),
            vec![0.13, 0.10, 0.12, 0.12, 0.12, 0.14, 0.27],
        ),
        1 => (
            vec![
                "Model",
                "Requests",
                "Output",
                "Engine time",
                "Token/s",
                "Qwen € / USD",
            ],
            snapshot
                .models
                .iter()
                .map(|g| {
                    let c = &g.counts;
                    let cost = snapshot.prices.costs(c)[0];
                    vec![
                        g.name.clone(),
                        grouped(c.requests),
                        grouped(c.output),
                        elapsed(c.prompt_ms + c.decode_ms),
                        if c.decode_ms > 0.0 {
                            format!("{:.1}", c.output as f64 * 1000.0 / c.decode_ms)
                        } else {
                            "-".into()
                        },
                        format!(
                            "{} / {}",
                            money(cost / snapshot.prices.usd_per_eur, true),
                            money(cost, false)
                        ),
                    ]
                })
                .collect(),
            vec![0.40, 0.10, 0.12, 0.12, 0.08, 0.18],
        ),
        2 => (
            vec!["Day / period", "Requests", "Output", "Engine time", "File"],
            snapshot
                .logs
                .iter()
                .map(|g| {
                    vec![
                        g.period.clone(),
                        grouped(g.counts.requests),
                        grouped(g.counts.output),
                        elapsed(g.counts.prompt_ms + g.counts.decode_ms),
                        g.source.clone(),
                    ]
                })
                .collect(),
            vec![0.27, 0.12, 0.13, 0.17, 0.31],
        ),
        _ => (
            vec!["Time", "Output", "Fresh input", "Duration", "Result"],
            snapshot
                .recent
                .iter()
                .map(|r| {
                    vec![
                        data::local_date(r.at, true),
                        grouped(r.output),
                        grouped(r.fresh),
                        elapsed(r.seconds * 1000.0),
                        r.finish.clone(),
                    ]
                })
                .collect(),
            vec![0.24, 0.18, 0.20, 0.18, 0.20],
        ),
    }
}
fn paint(hwnd: HWND, app: &mut App) {
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let target = BeginPaint(hwnd, &mut ps);
        let mut bounds: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut bounds);
        let width = bounds.right - theme::SIDE;
        let height = bounds.bottom;
        // One GDI bitmap per repaint prevents flicker; no retained image or GPU surface.
        let dc = CreateCompatibleDC(target);
        let bitmap = CreateCompatibleBitmap(target, bounds.right.max(1), height.max(1));
        let old = SelectObject(dc, bitmap);
        fill(dc, bounds, BG);
        theme::sidebar(dc, height, &app.fonts);
        SetViewportOrgEx(dc, theme::SIDE, 0, null_mut());
        let snapshot = app
            .snapshot
            .lock()
            .expect("Snapshot mutex poisoned")
            .clone();
        text(
            dc,
            rect(28, 22, 450, 42),
            match app.tools_page {
                3 => "Updates",
                4 => "Agents",
                5 => "Model library",
                _ if app.show_hub => "Models",
                _ => "Usage",
            },
            WHITE,
            app.fonts[1],
        );
        let online =
            snapshot.online && snapshot.error.is_empty() && data::now() - snapshot.updated < 10.0;
        let loaded = online && snapshot.phase != "unloaded";
        let failed = app.launch.as_ref().is_some_and(|launch| launch.failed());
        let starting = app
            .launch
            .as_ref()
            .is_some_and(|launch| !launch.failed() && launch.phase != launch::Phase::Ready);
        let color = if failed {
            theme::ERROR
        } else if loaded {
            ACCENT
        } else if app.launch.is_some() {
            theme::WARNING
        } else {
            MUTED
        };
        let label = if failed {
            "Error"
        } else if loaded {
            "Online"
        } else if starting {
            "Starting"
        } else if online {
            "Unloaded"
        } else if app.launch.is_some() {
            "Disconnected"
        } else {
            "Stopped"
        };
        theme::status(
            dc,
            width,
            label,
            color,
            if loaded { snapshot.speed } else { None },
            app.fonts[0],
        );
        let launch_detail = app
            .launch
            .as_ref()
            .map(|launch| launch.summary(loaded, snapshot.slots.max(1)))
            .unwrap_or_else(|| {
                if loaded {
                    format!("Ready · {} slots", snapshot.slots.max(1))
                } else {
                    "No model running".into()
                }
            });
        let detail = if loaded && !failed {
            let phase = match snapshot.phase.as_str() {
                "idle" => "Idle",
                "reading" => "Reading prompt",
                "thinking" => "Thinking",
                "generating" | "answering" => "Generation",
                "tool_call" => "Tool call",
                other => other,
            };
            format!(
                "{}  ·  {}{}",
                friendly_model(&snapshot.model),
                phase,
                snapshot
                    .generated
                    .map(|n| format!("  ·  {} token", grouped(n)))
                    .unwrap_or_default()
            )
        } else {
            launch_detail.clone()
        };
        text(
            dc,
            rect(28, 66, width - 428, 26),
            &detail,
            if failed { theme::ERROR } else { MUTED },
            app.fonts[0],
        );
        for (offset, label, bytes) in [
            (374, "RAM", snapshot.memory.ram),
            (196, "VRAM", snapshot.memory.vram),
        ] {
            let bounds = rect(width - offset, 62, 168, 28);
            theme::rounded(dc, bounds, PANEL, Some(theme::BORDER), 14);
            text(
                dc,
                rect(bounds.left + 12, bounds.top, 146, 28),
                &bytes
                    .map(|bytes| format!("{label} {:.1} GiB", bytes as f64 / 1073741824.0))
                    .unwrap_or_else(|| format!("{label} N/A")),
                if bytes.is_some() { WHITE } else { MUTED },
                app.fonts[0],
            );
        }
        if app.tools_page == 3 {
            tools_ui::paint_updates(
                dc,
                &app.fonts,
                width,
                height,
                &app.updates.status(),
                &app.installed_version,
                snapshot.online || app.launch.as_ref().is_some_and(|l| l.running),
            );
        } else if app.tools_page == 4 {
            tools_ui::paint_agents(
                dc,
                &app.fonts,
                width,
                height,
                &app.hub,
                &app.agents.status(),
            );
        } else if app.tools_page == 5 {
            models_ui::paint(
                hwnd,
                dc,
                &app.fonts,
                width,
                height,
                &app.library,
                &app.models.status(),
            );
        } else if app.show_hub {
            hub_ui::paint(
                hwnd,
                dc,
                &app.hub,
                &app.fonts,
                width,
                height,
                &launch_detail,
                color,
                app.launch
                    .as_ref()
                    .map(|l| l.phase.step())
                    .unwrap_or(if loaded { 5 } else { 0 }),
            );
        } else {
            let gap = 16;
            let cw = (width - 56 - gap * 3) / 4;
            let c = &snapshot.totals;
            let today = data::local_date(data::now(), false);
            let today_count = snapshot
                .daily
                .iter()
                .find(|g| g.name == today)
                .map(|g| g.counts.requests)
                .unwrap_or(0);
            let cache = if c.prompt > 0 {
                100.0 * c.cached as f64 / c.prompt as f64
            } else {
                0.0
            };
            let cards = [
                ("Generated tokens", grouped(c.output), String::new()),
                (
                    "Requests",
                    grouped(c.requests),
                    format!("Today: {}", grouped(today_count)),
                ),
                (
                    "Engine time",
                    elapsed(c.prompt_ms + c.decode_ms),
                    String::new(),
                ),
                (
                    "Prompt cache",
                    format!("{cache:.1}%"),
                    format!("{} fresh tokens", grouped(c.prompt - c.cached)),
                ),
            ];
            for (i, (label, value, detail)) in cards.iter().enumerate() {
                card(
                    dc,
                    rect(28 + i as i32 * (cw + gap), 112, cw, 98),
                    label,
                    value,
                    detail,
                    &app.fonts,
                )
            }
            let costs = snapshot.prices.costs(c);
            for (i, label) in [
                "Qwen without cache",
                "Uncached input",
                "Output",
                "Estimated cached cost",
            ]
            .iter()
            .enumerate()
            {
                card(
                    dc,
                    rect(28 + i as i32 * (cw + gap), 222, cw, 98),
                    label,
                    &money(costs[i] / snapshot.prices.usd_per_eur, true),
                    &money(costs[i], false),
                    &app.fonts,
                )
            }
            text(
                dc,
                rect(28, 326, width - 56, 24),
                &format!(
                    "qwen3.8-flash · Rates {} · ECB {} · 1 EUR = {} USD",
                    snapshot.prices.date, snapshot.prices.fx_date, snapshot.prices.usd_per_eur
                ),
                MUTED,
                app.fonts[0],
            );
            let chart = rect(28, 360, width - 56, 107);
            theme::rounded(dc, chart, PANEL, Some(theme::BORDER), 14);
            let days: Vec<_> = snapshot
                .daily
                .iter()
                .rev()
                .take(14)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let max = days
                .iter()
                .map(|g| g.counts.output)
                .max()
                .unwrap_or(1)
                .max(1) as f64;
            let step = (width - 84) as f64 / days.len().max(1) as f64;
            for (i, g) in days.iter().enumerate() {
                let bh = (64.0 * g.counts.output as f64 / max) as i32;
                let x = 42 + (i as f64 * step) as i32;
                fill(
                    dc,
                    rect(x, 438 - bh, (step * 0.65).min(120.0) as i32, bh),
                    ACCENT,
                );
                text(
                    dc,
                    rect(x, 439, step as i32, 24),
                    &g.name[5..],
                    MUTED,
                    app.fonts[0],
                );
            }
            let top = 514;
            let row_height = 31;
            let (headers, data, ratios) = rows(&snapshot, app.tab);
            app.visible_rows = ((height - top - 45).max(row_height) / row_height) as usize;
            app.scroll = app.scroll.min(data.len().saturating_sub(app.visible_rows));
            let mut scroll: SCROLLINFO = std::mem::zeroed();
            scroll.cbSize = std::mem::size_of_val(&scroll) as u32;
            scroll.fMask = SIF_RANGE | SIF_PAGE | SIF_POS;
            scroll.nMax = data.len().saturating_sub(1) as i32;
            scroll.nPage = app.visible_rows as u32;
            scroll.nPos = app.scroll as i32;
            SetScrollInfo(hwnd, SB_VERT, &scroll, 1);
            let mut x = 22;
            for (i, header) in headers.iter().enumerate() {
                let column = ((width - 44) as f64 * ratios[i]) as i32;
                text(
                    dc,
                    rect(x + 8, top, column - 12, row_height),
                    header,
                    ACCENT,
                    app.fonts[0],
                );
                x += column;
            }
            for (index, row) in data
                .iter()
                .skip(app.scroll)
                .take(app.visible_rows)
                .enumerate()
            {
                let y = top + row_height * (index as i32 + 1);
                if y + row_height > height - 36 {
                    break;
                }
                fill(
                    dc,
                    rect(22, y, width - 44, row_height),
                    if index % 2 == 0 { PANEL } else { BG },
                );
                let mut x = 22;
                for (i, cell) in row.iter().enumerate() {
                    let column = ((width - 44) as f64 * ratios[i]) as i32;
                    text(
                        dc,
                        rect(x + 8, y, column - 12, row_height),
                        cell,
                        WHITE,
                        app.fonts[0],
                    );
                    x += column;
                }
            }
        }
        if app.tools_page >= 3 {
            fill(dc, rect(28, height - 112, width - 56, 1), theme::BORDER);
        }
        let footer = if !snapshot.memory.error.is_empty() {
            format!(
                "{} · Hub RAM {:.1} MiB",
                snapshot.memory.error,
                own_memory()
            )
        } else if snapshot.error.is_empty() || snapshot.error == "Strata offline" {
            format!(
                "Local  ·  {}  ·  Live 2 s · Totals 15 s  ·  Hub RAM {:.1} MiB",
                data::local_date(snapshot.updated, true),
                own_memory()
            )
        } else {
            format!("{} · Hub RAM {:.1} MiB", snapshot.error, own_memory())
        };
        text(
            dc,
            rect(22, height - 30, width - 44, 25),
            &footer,
            MUTED,
            app.fonts[0],
        );
        SetViewportOrgEx(dc, 0, 0, null_mut());
        BitBlt(target, 0, 0, bounds.right, height, dc, 0, 0, SRCCOPY);
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
        EndPaint(hwnd, &ps);
    }
}
fn start_strata(hwnd: HWND, app: &mut App) -> Result<()> {
    if app.hub.profiles.is_empty() {
        return Err("Prepare a model profile before starting Strata.".into());
    }
    if app.updates.status().blocks_model || app.models.status().blocks_model {
        return Err(
            "Wait for the current Strata update or model file operation before starting a model."
                .into(),
        );
    }
    if let Some(child) = app.child.as_mut() {
        if child.try_wait()?.is_none() {
            return Err(
                "Strata is already running. Stop it before changing the model or settings.".into(),
            );
        }
    }
    if recorded_child()?.is_some() {
        return Err("A Strata process owned by the hub is running or starting. Stop it before launching another.".into());
    }
    app.hub.profiles[app.hub.selected].settings = hub_ui::read(hwnd)?;
    let port = app.hub.profiles[app.hub.selected].settings.port;
    if TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse()?,
        Duration::from_millis(250),
    )
    .is_ok()
    {
        return Err(format!(
            "Port {port} is already in use. Stop Strata before launching another model."
        )
        .into());
    }
    app.hub.profiles[app.hub.selected].settings = hub_ui::read(hwnd)?;
    let agent_settings = agent_settings_target(&app.hub.profiles[app.hub.selected].settings)?;
    let config = app.hub.prepare()?;
    app.hub.save()?;
    hub_ui::saved(hwnd);
    app.pending_agent_settings = Some(agent_settings);
    poll_agent_settings(hwnd, app);
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(data::root().join("strata-native-stdout.log"))?;
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(data::root().join("strata-native-stderr.log"))?;
    let stdout_start = stdout.metadata()?.len();
    let stderr_start = stderr.metadata()?.len();
    let engine_log = app.hub.log_path()?;
    let engine_start = match std::fs::metadata(&engine_log) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    let child = Command::new(data::python())
        .args([
            "-u",
            "-m",
            "serve.server",
            "--engine",
            "strata",
            "--config",
            config
                .to_str()
                .ok_or("Invalid runtime configuration path")?,
            "--port",
            &port.to_string(),
        ])
        .current_dir(data::strata_home())
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()?;
    app.child = Some(child);
    let child = app.child.as_ref().expect("Child just assigned");
    let created = process_created(child.as_raw_handle())?;
    let record = serde_json::json!({"pid":child.id(),"created":created,"config":config,"profile":app.hub.profiles[app.hub.selected].file,
        "label":app.hub.profiles[app.hub.selected].label,"parallel":app.hub.profiles[app.hub.selected].settings.parallel,
        "stdout_start":stdout_start,"stderr_start":stderr_start,"engine_start":engine_start,"engine_log":engine_log,"port":port});
    std::fs::write(
        data::root().join("owned-strata.json"),
        serde_json::to_vec(&record)?,
    )?;
    app.launch = Some(launch::Launch::from_record(&record)?);
    *app.auth_profile.lock().expect("Profile mutex poisoned") = Some(config);
    app.hub.notice = format!("Starting {}", app.hub.profiles[app.hub.selected].label);
    app.refresh.send(())?;
    unsafe {
        InvalidateRect(hwnd, null(), 0);
    }
    Ok(())
}
fn process_created(handle: HANDLE) -> Result<u64> {
    unsafe {
        let mut created: FILETIME = std::mem::zeroed();
        let mut exited: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        if GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok((created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64)
    }
}
fn recorded_child() -> Result<Option<(u32, OwnedHandle)>> {
    let bytes = match std::fs::read(data::root().join("owned-strata.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let record: serde_json::Value = serde_json::from_slice(&bytes)?;
    let pid = u32::try_from(record["pid"].as_u64().ok_or("Invalid owned Strata PID")?)?;
    let created = record["created"]
        .as_u64()
        .ok_or("Invalid owned Strata creation time")?;
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            if GetLastError() == ERROR_INVALID_PARAMETER {
                return Ok(None);
            }
            return Err(std::io::Error::last_os_error().into());
        }
        let owned = OwnedHandle::from_raw_handle(handle);
        let mut code = 0;
        if GetExitCodeProcess(handle, &mut code) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if code != STILL_ACTIVE as u32 || process_created(handle)? != created {
            return Ok(None);
        }
        Ok(Some((pid, owned)))
    }
}
fn stop_strata(hwnd: HWND, app: &mut App) -> Result<()> {
    // Keep a process handle open to prevent PID reuse while stopping the owned tree.
    let recorded = recorded_child()?;
    let pid = match app.child.as_mut() {
        Some(child) => {
            if child.try_wait()?.is_none() {
                Some(child.id())
            } else {
                recorded.as_ref().map(|(pid, _)| *pid)
            }
        }
        _ => recorded.as_ref().map(|(pid, _)| *pid),
    };
    if let Some(pid) = pid {
        let status = Command::new("taskkill.exe")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            return Err("Could not stop the Strata process".into());
        }
        app.child = None;
        app.launch = None;
        app.hub.notice = "Strata stopped.".into();
        std::fs::remove_file(data::root().join("owned-strata.json"))?;
        app.refresh.send(())?;
    } else {
        message(
            hwnd,
            "No server started by this app is running. Stop externally launched servers from their original launcher.",
        );
    }
    Ok(())
}
fn worker(
    state: Arc<Mutex<Snapshot>>,
    receiver: mpsc::Receiver<()>,
    hwnd: isize,
    auth_profile: Arc<Mutex<Option<PathBuf>>>,
) {
    let run = || -> Result<()> {
        let mut db = data::connect()?;
        let mut snapshot = data::load(&db)?;
        let mut stored_at = Instant::now();
        let mut full = true;
        let mut resources = resources::Monitor::default();
        loop {
            let profile = auth_profile.lock().expect("Profile mutex poisoned").clone();
            full |= stored_at.elapsed() >= Duration::from_secs(15);
            if let Some(profile) = profile.as_deref() {
                match data::fetch_metrics(profile, full) {
                    Ok(value) => {
                        if full {
                            data::store(&mut db, &value)?;
                            snapshot = data::load(&db)?;
                            stored_at = Instant::now();
                        }
                        snapshot.online = true;
                        snapshot.error.clear();
                        snapshot.updated = data::now();
                        snapshot.model = value["engine"]["model"]
                            .as_str()
                            .ok_or("Missing loaded model")?
                            .into();
                        snapshot.phase = value["live"]["phase"]
                            .as_str()
                            .unwrap_or(value["live"]["state"].as_str().unwrap_or(""))
                            .into();
                        snapshot.speed = value["live"]["tok_s"].as_f64();
                        snapshot.generated = value["live"]["generated"].as_i64();
                        snapshot.slots =
                            u32::try_from(value["live"]["parallel"].as_u64().unwrap_or(1))?.max(1);
                    }
                    Err(error) => {
                        let network = error.downcast_ref::<std::io::Error>().is_some_and(|e| {
                            matches!(
                                e.kind(),
                                std::io::ErrorKind::ConnectionRefused
                                    | std::io::ErrorKind::ConnectionReset
                                    | std::io::ErrorKind::ConnectionAborted
                                    | std::io::ErrorKind::TimedOut
                                    | std::io::ErrorKind::WouldBlock
                                    | std::io::ErrorKind::UnexpectedEof
                                    | std::io::ErrorKind::NotConnected
                            ) || (e.kind() == std::io::ErrorKind::NotFound
                                && auth_profile
                                    .lock()
                                    .expect("Profile mutex poisoned")
                                    .is_none())
                        });
                        if !network {
                            return Err(error);
                        }
                        snapshot.online = false;
                        snapshot.speed = None;
                        snapshot.generated = None;
                        snapshot.error = "Strata offline".into();
                        snapshot.updated = data::now();
                    }
                }
            } else {
                if full {
                    snapshot = data::load(&db)?;
                    stored_at = Instant::now();
                }
                snapshot.online = false;
                snapshot.speed = None;
                snapshot.generated = None;
                snapshot.error.clear();
                snapshot.updated = data::now();
            }
            snapshot.memory = resources.sample(profile.as_deref());
            *state.lock().expect("Snapshot mutex poisoned") = snapshot.clone();
            unsafe {
                PostMessageW(hwnd as HWND, UPDATE, 0, 0);
            }
            let visible =
                unsafe { IsWindowVisible(hwnd as HWND) != 0 && IsIconic(hwnd as HWND) == 0 };
            full = match receiver.recv_timeout(Duration::from_secs(if visible { 2 } else { 15 })) {
                Ok(()) => true,
                Err(mpsc::RecvTimeoutError::Timeout) => false,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
        }
        Ok(())
    };
    if let Err(error) = run() {
        log_error(&error.to_string());
        let mut snapshot = state.lock().expect("Snapshot mutex poisoned");
        snapshot.online = false;
        snapshot.speed = None;
        snapshot.memory = Default::default();
        snapshot.error = format!("Metrics error: {error}");
        unsafe {
            PostMessageW(hwnd as HWND, UPDATE, 0, 0);
        }
    }
}

fn monitor_launch(hwnd: HWND, app: &mut App) {
    let snapshot = app
        .snapshot
        .lock()
        .expect("Snapshot mutex poisoned")
        .clone();
    let online = snapshot.online
        && snapshot.error.is_empty()
        && snapshot.phase != "unloaded"
        && data::now() - snapshot.updated < 10.0;
    let mut changed = false;
    if let Some(launch) = app.launch.as_mut() {
        let previous = launch.phase;
        if let Err(error) = launch.poll(online && snapshot.updated >= launch.started) {
            log_error(&error.to_string());
            launch.phase = launch::Phase::Failed;
            launch.error = format!("Startup monitor error: {error}");
        }
        changed =
            previous != launch.phase || (!launch.failed() && launch.phase != launch::Phase::Ready);
    }
    tools_ui::sync(
        hwnd,
        &app.updates.status(),
        &app.agents.status(),
        online || app.launch.as_ref().is_some_and(|l| l.running),
        app.hub
            .profiles
            .get(app.hub.selected)
            .map(|profile| profile.settings.port),
        app.hub
            .profiles
            .get(app.hub.selected)
            .map(|profile| profile.settings.values[0].as_str()),
        app.hub
            .profiles
            .get(app.hub.selected)
            .map(|profile| profile.settings.vision),
    );
    models_ui::sync_controls(
        hwnd,
        &app.library,
        &app.models.status(),
        snapshot.online || app.launch.as_ref().is_some_and(|l| l.running),
        app.updates.status().busy,
    );
    // Controls reflect process ownership; an external server still blocks another launch.
    let owned = app.launch.as_ref().is_some_and(|l| l.running);
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
            GetDlgItem(hwnd, hub_ui::RUN as i32),
            (!app.hub.profiles.is_empty()
                && !owned
                && !snapshot.online
                && !app.updates.status().blocks_model
                && !app.models.status().blocks_model) as i32,
        );
        windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow(
            GetDlgItem(hwnd, hub_ui::STOP as i32),
            owned as i32,
        );
        if changed && IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 {
            InvalidateRect(hwnd, null(), 0);
        }
    }
}

fn friendly_model(model: &str) -> &str {
    match model {
        "huihui-swift-qwen3.8-flash-next-iq3_s-abliterated-v2" => {
            "Huihui Swift IQ3_S · Abliterated"
        }
        "alesha-qwen3.8-flash-next-gsq-rco-iq3_s-abliterated" => "Qwen IQ3_S · Abliterated",
        "alesha-qwen3.8-flash-next-gsq-rco-iq3_s-original" => "Qwen IQ3_S · Original",
        _ => model,
    }
}
fn layout_controls(hwnd: HWND) {
    unsafe {
        hub_ui::resize(hwnd);
        let mut client: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut client);
        tools_ui::resize(hwnd);
        models_ui::resize(hwnd);
        for (id, y) in [
            (110, 128),
            (115, 180),
            (111, 232),
            (112, 284),
            (113, 336),
            (114, 388),
        ] {
            MoveWindow(GetDlgItem(hwnd, id), 16, y, 168, 44, 1);
        }
        for id in 101..=104 {
            MoveWindow(
                GetDlgItem(hwnd, id),
                theme::SIDE + 28 + (id - 101) * 104,
                478,
                96,
                32,
                1,
            );
        }
        MoveWindow(GetDlgItem(hwnd, 105), client.right - 280, 478, 108, 32, 1);
        MoveWindow(GetDlgItem(hwnd, 106), client.right - 156, 478, 128, 32, 1);
    }
}
fn set_page(hwnd: HWND, app: &mut App, hub: bool) {
    unsafe {
        app.show_hub = hub;
        app.tools_page = 0;
        tools_ui::visible(hwnd, 0);
        models_ui::visible(hwnd, false);
        theme::view(hwnd, if hub { 1 } else { 2 });
        for id in [110, 111, 112, 113, 114, 115] {
            InvalidateRect(GetDlgItem(hwnd, id), null(), 0);
        }
        for id in 101..=106 {
            ShowWindow(GetDlgItem(hwnd, id), if hub { SW_HIDE } else { SW_SHOW });
        }
        hub_ui::visible(hwnd, hub);
        ShowScrollBar(hwnd, SB_VERT, (!hub) as i32);
        InvalidateRect(hwnd, null(), 0);
    }
}
fn set_tools_page(hwnd: HWND, app: &mut App, page: usize) -> Result<()> {
    if let Some(profile) = app.hub.profiles.get_mut(app.hub.selected) {
        profile.settings = hub_ui::read(hwnd)?;
    }
    set_page(hwnd, app, false);
    app.tools_page = page;
    theme::view(hwnd, page);
    tools_ui::visible(hwnd, page);
    models_ui::visible(hwnd, page == 5);
    unsafe {
        for id in 101..=106 {
            ShowWindow(GetDlgItem(hwnd, id), SW_HIDE);
        }
        ShowScrollBar(hwnd, SB_VERT, 0);
        InvalidateRect(hwnd, null(), 0);
    }
    Ok(())
}
fn open_log(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Err("No operation log is available yet.".into());
    }
    Command::new("notepad.exe").arg(path).spawn()?;
    Ok(())
}
fn tools_command(hwnd: HWND, app: &mut App, id: usize) -> Result<()> {
    poll_agent_settings(hwnd, app);
    if id == tools_ui::UPDATE_LOG {
        return open_log(&app.updates.log);
    }
    if id == tools_ui::AGENT_LOG {
        return open_log(&app.agents.log);
    }
    if id == tools_ui::RELEASE {
        let status = app.updates.status();
        let url = status.data["url"]
            .as_str()
            .ok_or("Check for updates first.")?;
        if !url.starts_with("https://github.com/Niko1221/Strata/releases/tag/v") {
            return Err("Invalid release URL".into());
        }
        unsafe {
            let result = ShellExecuteW(
                hwnd,
                wide("open").as_ptr(),
                wide(url).as_ptr(),
                null(),
                null(),
                SW_SHOWNORMAL,
            );
            if result as usize <= 32 {
                return Err("Could not open the release notes".into());
            }
        }
        return Ok(());
    }
    if [tools_ui::CHECK, tools_ui::INSTALL].contains(&id) {
        let install = id == tools_ui::INSTALL;
        if install
            && (recorded_child()?.is_some()
                || app.snapshot.lock().expect("Snapshot mutex poisoned").online)
        {
            return Err("Stop the running model before installing a Strata update.".into());
        }
        if install && (app.agents.status().busy || app.models.status().busy) {
            return Err(
                "Wait for the agent or model library operation to finish before updating Strata."
                    .into(),
            );
        }
        let mut arguments = vec![
            if install { "install" } else { "check" }.into(),
            "--root".into(),
            data::strata_home().to_string_lossy().into_owned(),
            "--data".into(),
            data::model_root().to_string_lossy().into_owned(),
            "--profiles".into(),
            data::profiles_dir().to_string_lossy().into_owned(),
            "--report".into(),
            data::root().to_string_lossy().into_owned(),
        ];
        if install {
            let state = app.updates.status();
            if state.error || state.data["available"].as_bool() != Some(true) {
                return Err("Check for an available stable update first.".into());
            }
            arguments.extend([
                "--tag".into(),
                state.data["latest"]
                    .as_str()
                    .ok_or("Missing release tag")?
                    .into(),
            ]);
        }
        app.updates.start(
            hwnd,
            include_str!("../scripts/update_strata.py"),
            arguments,
            install,
            install,
            if install {
                "Preparing Strata update"
            } else {
                "Checking official stable releases"
            },
        )?;
    } else {
        if app.updates.status().blocks_model {
            return Err("Wait for the Strata update to finish before changing agents.".into());
        }
        if id != tools_ui::SCAN && id < 710 {
            return Err("Invalid agent action".into());
        }
        let (mode, agent) = if id == tools_ui::SCAN {
            ("scan", None)
        } else {
            let index = (id - 710) / 10;
            let mode = match (id - 710) % 10 {
                0 => "install",
                1 => "connect",
                2 => "launch",
                _ => return Err("Invalid agent action".into()),
            };
            (
                mode,
                Some(*tools_ui::AGENTS.get(index).ok_or("Unknown agent")?),
            )
        };
        let profile = app
            .hub
            .profiles
            .get(app.hub.selected)
            .ok_or("Prepare a model profile before connecting agents")?;
        let settings = hub_ui::read(hwnd)?;
        let mut arguments = vec![
            mode.into(),
            "--report".into(),
            data::root().to_string_lossy().into_owned(),
            "--profile".into(),
            profile.path.to_string_lossy().into_owned(),
            "--port".into(),
            settings.port.to_string(),
            "--context".into(),
            settings.values[0].clone(),
        ];
        if settings.vision {
            arguments.push("--vision".into());
        }
        if let Some(agent) = agent {
            arguments.extend(["--agent".into(), agent.into()]);
        }
        app.agents.start(
            hwnd,
            include_str!("../scripts/agents.py"),
            arguments,
            matches!(mode, "install" | "connect"),
            false,
            match mode {
                "scan" => "Scanning installed agents",
                "install" => "Installing agent",
                "connect" => "Connecting agent to Strata",
                _ => "Launching agent",
            },
        )?;
    }
    monitor_launch(hwnd, app);
    unsafe {
        InvalidateRect(hwnd, null(), 0);
    }
    Ok(())
}
fn agent_settings_target(settings: &profiles::Settings) -> Result<AgentSettings> {
    let value = settings.values[0].trim();
    let context = if value.is_empty() {
        None
    } else {
        let tokens = value
            .parse::<i32>()
            .map_err(|_| "Context must be a positive integer.")?;
        if tokens <= 0 {
            return Err("Context must be a positive integer.".into());
        }
        Some(tokens as u32)
    };
    Ok(AgentSettings {
        port: settings.port,
        context,
        vision: settings.vision,
    })
}
fn poll_agent_settings(hwnd: HWND, app: &mut App) {
    let status = app.agents.status();
    if app.syncing_agent_settings && !status.busy {
        app.syncing_agent_settings = false;
        if status.error {
            log_error(&status.message);
            message(
                hwnd,
                &format!(
                    "Model settings were saved, but agent connection synchronization failed:\n{}\nSee the agent log for details.",
                    status.message
                ),
            );
        }
        if !app.hub.notice.starts_with("Starting ") {
            app.hub.notice = status.message;
        }
    }
    if status.busy || app.updates.status().busy {
        return;
    }
    let Some(target) = app.pending_agent_settings.take() else {
        return;
    };
    let mut arguments = vec![
        "sync-settings".into(),
        "--report".into(),
        data::root().to_string_lossy().into_owned(),
        "--port".into(),
        target.port.to_string(),
    ];
    if let Some(context) = target.context {
        arguments.extend(["--context".into(), context.to_string()]);
    }
    if target.vision {
        arguments.push("--vision".into());
    }
    match app.agents.start(
        hwnd,
        include_str!("../scripts/agents.py"),
        arguments,
        true,
        false,
        "Synchronizing connected agent settings",
    ) {
        Ok(()) => app.syncing_agent_settings = true,
        Err(error) => {
            log_error(&error.to_string());
            message(
                hwnd,
                &format!("Agent connection synchronization could not start: {error}"),
            );
        }
    }
}
fn hub_command(hwnd: HWND, app: &mut App, id: usize, notification: u32) -> Result<()> {
    if app.hub.profiles.is_empty() {
        return Err("No prepared model profiles. Use Model library and Strata setup first.".into());
    }
    if app.models.status().blocks_model {
        return Err("Wait for the model file operation before changing model settings.".into());
    }
    match id {
        hub_ui::PROFILE if notification == CBN_SELCHANGE => {
            let selected = hub_ui::selected(hwnd)?;
            match hub_ui::read(hwnd) {
                Ok(settings) => app.hub.profiles[app.hub.selected].settings = settings,
                Err(error) => {
                    hub_ui::restore_selection(hwnd, app.hub.selected);
                    return Err(error);
                }
            }
            if selected >= app.hub.profiles.len() {
                return Err("Invalid selected profile".into());
            }
            app.hub.selected = selected;
            app.hub.notice.clear();
            hub_ui::fill(hwnd, &app.hub);
        }
        hub_ui::RUN => start_strata(hwnd, app)?,
        hub_ui::STOP => stop_strata(hwnd, app)?,
        hub_ui::SAVE => {
            app.hub.profiles[app.hub.selected].settings = hub_ui::read(hwnd)?;
            let target = agent_settings_target(&app.hub.profiles[app.hub.selected].settings)?;
            app.hub.save()?;
            hub_ui::saved(hwnd);
            app.pending_agent_settings = Some(target);
            app.hub.notice = "Settings saved. Synchronizing connected agent settings.".into();
            poll_agent_settings(hwnd, app);
        }
        hub_ui::ENGINE..=hub_ui::ADVANCED => hub_ui::select_section(hwnd, id - hub_ui::ENGINE),
        hub_ui::RESET => {
            app.hub.reset()?;
            hub_ui::fill(hwnd, &app.hub);
            hub_ui::mark_dirty(hwnd);
        }
        hub_ui::START_LOG => {
            let snapshot = app
                .snapshot
                .lock()
                .expect("Snapshot mutex poisoned")
                .clone();
            if let Some(launch) = &app.launch {
                message(
                    hwnd,
                    &launch.details(snapshot.online, snapshot.slots.max(1)),
                );
            } else {
                message(
                    hwnd,
                    "No monitored launch in this session. Start a model from the hub to see startup stages, elapsed time and errors.",
                );
            }
        }
        hub_ui::LOG => {
            let path = app.hub.log_path()?;
            if !path.is_file() {
                return Err(format!("Log is not available yet: {}", path.display()).into());
            }
            Command::new("notepad.exe").arg(path).spawn()?;
        }
        _ => (),
    }
    unsafe {
        InvalidateRect(hwnd, null(), 0);
    }
    Ok(())
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            let app = create.lpCreateParams as *const App;
            if let Err(error) = theme::init(hwnd, (*app).fonts[0]) {
                log_error(&error.to_string());
                return 0;
            }
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, msg, w, l);
        }
        // Control text changes send synchronous notifications to this window.
        // Borrow application state only in handlers that actually need it.
        match msg {
            WM_CREATE => {
                let app = &mut *pointer;
                for (id, title, x, width) in [
                    (101, "Days", 22, 92),
                    (102, "Models", 124, 92),
                    (103, "History", 226, 92),
                    (104, "Requests", 328, 92),
                    (105, "Refresh", 440, 100),
                    (106, "Export CSV", 550, 112),
                    (110, "Models", 760, 120),
                    (111, "Usage", 900, 120),
                    (112, "Updates", 900, 120),
                    (113, "Agents", 900, 120),
                    (114, "Settings", 900, 120),
                    (115, "Model library", 900, 120),
                ] {
                    let button = CreateWindowExW(
                        0,
                        wide("BUTTON").as_ptr(),
                        wide(title).as_ptr(),
                        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
                        x,
                        if id >= 110 { 18 } else { 478 },
                        width,
                        28,
                        hwnd,
                        id as HMENU,
                        GetModuleHandleW(null()),
                        null(),
                    );
                    SendMessageW(button, WM_SETFONT, app.fonts[0] as WPARAM, 1);
                }
                if let Err(error) = hub_ui::create(hwnd, &app.hub, &app.fonts) {
                    message(hwnd, &error.to_string());
                    return -1;
                }
                if let Err(error) = tools_ui::create(hwnd, app.fonts[0]) {
                    message(hwnd, &error.to_string());
                    return -1;
                }
                if let Err(error) = models_ui::create(hwnd, app.fonts[0]) {
                    message(hwnd, &error.to_string());
                    return -1;
                }
                set_page(hwnd, app, app.show_hub);
                layout_controls(hwnd);
                app.tray.cbSize = std::mem::size_of_val(&app.tray) as u32;
                app.tray.hWnd = hwnd;
                app.tray.uID = 1;
                app.tray.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
                app.tray.uCallbackMessage = TRAY;
                app.tray.hIcon = theme::icon(16);
                let tip = wide("Strata Hub · double-click to open");
                app.tray.szTip[..tip.len()].copy_from_slice(&tip);
                Shell_NotifyIconW(NIM_ADD, &app.tray);
                if SetTimer(hwnd, 1, 1000, None) == 0 {
                    log_error("Cannot start the launch status timer");
                    return -1;
                }
                0
            }
            WM_PAINT => {
                let app = &mut *pointer;
                paint(hwnd, app);
                0
            }
            WM_DRAWITEM => theme::draw_item(hwnd, &*(l as *const DRAWITEMSTRUCT)),
            WM_MEASUREITEM => {
                let item = &mut *(l as *mut MEASUREITEMSTRUCT);
                item.itemHeight = 32;
                1
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC => {
                theme::color(hwnd, w as HDC, false)
            }
            WM_CTLCOLORBTN => theme::color(hwnd, w as HDC, true),
            WM_ERASEBKGND => 1,
            WM_SIZE => {
                layout_controls(hwnd);
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(l as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT { x: 1280, y: 820 };
                0
            }
            WM_TIMER if w == 1 => {
                poll_agent_settings(hwnd, &mut *pointer);
                monitor_launch(hwnd, &mut *pointer);
                0
            }
            UPDATE => {
                let plan_result = {
                    let app = &mut *pointer;
                    poll_agent_settings(hwnd, app);
                    if app.snapshot.lock().expect("Snapshot mutex poisoned").online
                        && app.hub.notice.starts_with("Starting ")
                    {
                        app.hub.notice = "Model ready. Changes apply on the next launch.".into();
                    }
                    monitor_launch(hwnd, app);
                    if IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 {
                        InvalidateRect(hwnd, null(), 0);
                    }
                    let status = app.models.status();
                    let mut refresh_profiles = || -> Result<()> {
                        let generation = status.data["profiles_generation"].as_u64().unwrap_or(0);
                        let inventory = status.data["library_generation"].as_u64().unwrap_or(0);
                        if !status.busy
                            && (generation != app.library.profiles_generation
                                || app.library.reload_after_operation
                                || (app.hub.profiles.is_empty()
                                    && inventory != app.library.empty_inventory_generation))
                        {
                            app.library.profiles_generation = generation;
                            app.library.empty_inventory_generation = inventory;
                            app.library.reload_after_operation = false;
                            let selected = app
                                .hub
                                .profiles
                                .get(app.hub.selected)
                                .map(|profile| profile.file.clone());
                            let mut hub = profiles::Hub::load()?;
                            if let Some(index) = hub
                                .profiles
                                .iter()
                                .position(|profile| Some(&profile.file) == selected.as_ref())
                            {
                                hub.selected = index;
                            }
                            app.hub = hub;
                            *app.auth_profile.lock().expect("Profile mutex poisoned") = app
                                .hub
                                .profiles
                                .get(app.hub.selected)
                                .map(|profile| profile.path.clone());
                            if app.hub.profiles.is_empty() {
                                app.launch = None;
                                app.child = None;
                            }
                            hub_ui::fill(hwnd, &app.hub);
                            app.library.profiles_generation = generation;
                        }
                        Ok(())
                    };
                    refresh_profiles().and_then(|()| app.library.sync(hwnd, &status))
                };
                let plan = match plan_result {
                    Ok(plan) => plan,
                    Err(error) => {
                        log_error(&error.to_string());
                        message(hwnd, &error.to_string());
                        None
                    }
                };
                if let Some(plan) = plan {
                    match models_ui::confirmation(&plan) {
                        Ok(prompt) => {
                            if MessageBoxW(
                                hwnd,
                                wide(&prompt).as_ptr(),
                                wide("Confirm model operation").as_ptr(),
                                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                            ) == IDYES
                            {
                                let app = &mut *pointer;
                                match models_ui::confirm(hwnd, &app.models, &plan) {
                                    Ok(()) => {
                                        app.library.reload_after_operation = true;
                                        *app.auth_profile.lock().expect("Profile mutex poisoned") =
                                            None;
                                    }
                                    Err(error) => message(hwnd, &error.to_string()),
                                }
                            }
                        }
                        Err(error) => message(hwnd, &error.to_string()),
                    }
                }
                0
            }
            WM_SHOWWINDOW if w != 0 => {
                if let Err(error) = (*pointer).refresh.send(()) {
                    log_error(&error.to_string());
                }
                DefWindowProcW(hwnd, msg, w, l)
            }
            START => {
                if (*pointer).models.status().blocks_model {
                    message(
                        hwnd,
                        "Wait for the model file operation before opening the model controls.",
                    );
                    return 0;
                }
                let app = &mut *pointer;
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
                set_page(hwnd, app, w != 2);
                if w == 1 {
                    if app.hub.profiles.is_empty() {
                        message(hwnd, "No prepared model profiles are available.");
                        return 0;
                    }
                    let mut launch = || -> Result<()> {
                        app.hub.profiles[app.hub.selected].settings = hub_ui::read(hwnd)?;
                        app.hub.selected = app
                            .hub
                            .profiles
                            .iter()
                            .position(|p| p.file == "strata-swift-iq3_s-abliterated-no-images.json")
                            .ok_or("Huihui profile is missing")?;
                        hub_ui::fill(hwnd, &app.hub);
                        start_strata(hwnd, app)
                    };
                    if let Err(error) = launch() {
                        message(hwnd, &error.to_string())
                    }
                }
                0
            }
            WM_MOUSEWHEEL => {
                let app = &mut *pointer;
                let delta = ((w >> 16) as u16) as i16;
                if delta > 0 {
                    app.scroll = app.scroll.saturating_sub(3)
                } else {
                    app.scroll = app.scroll.saturating_add(3)
                }
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_VSCROLL => {
                let app = &mut *pointer;
                match (w & 0xffff) as i32 {
                    SB_LINEUP => app.scroll = app.scroll.saturating_sub(1),
                    SB_LINEDOWN => app.scroll += 1,
                    SB_PAGEUP => app.scroll = app.scroll.saturating_sub(app.visible_rows),
                    SB_PAGEDOWN => app.scroll += app.visible_rows,
                    SB_THUMBTRACK | SB_THUMBPOSITION => {
                        let mut s: SCROLLINFO = std::mem::zeroed();
                        s.cbSize = std::mem::size_of_val(&s) as u32;
                        s.fMask = SIF_TRACKPOS;
                        GetScrollInfo(hwnd, SB_VERT, &mut s);
                        app.scroll = s.nTrackPos.max(0) as usize;
                    }
                    _ => (),
                };
                InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_COMMAND => {
                let id = (w & 0xffff) as usize;
                if id == 109 || id == hub_ui::EXIT {
                    let app = &*pointer;
                    if app.updates.status().mutating
                        || app.agents.status().mutating
                        || app.pending_agent_settings.is_some()
                        || app.models.status().busy
                    {
                        message(
                            hwnd,
                            "Wait for the current installation or configuration to finish before exiting. You can close the window to hide the hub.",
                        );
                        return 0;
                    }
                    DestroyWindow(hwnd);
                    return 0;
                }
                if id == 114 {
                    let busy = {
                        let app = &*pointer;
                        app.snapshot.lock().expect("Snapshot mutex poisoned").online
                            || app.updates.status().busy
                            || app.agents.status().busy
                            || app.pending_agent_settings.is_some()
                            || app.models.status().busy
                    };
                    let change = || -> Result<()> {
                        if recorded_child()?.is_some() || busy {
                            return Err("Stop Strata and wait for current operations before changing installation paths.".into());
                        }
                        if hub_ui::modified(hwnd) {
                            return Err(
                                "Save your model settings before changing installation paths."
                                    .into(),
                            );
                        }
                        if settings_ui::edit(hwnd, &config::get().file, Some(&config::get().paths))?
                        {
                            Command::new(std::env::current_exe()?)
                                .arg("--config")
                                .arg(&config::get().file)
                                .args(["--wait-for-parent", &std::process::id().to_string()])
                                .spawn()?;
                            DestroyWindow(hwnd);
                        }
                        Ok(())
                    };
                    if let Err(error) = change() {
                        message(hwnd, &error.to_string());
                    }
                    return 0;
                }
                let notification = (w >> 16) as u32;
                if (models_ui::REPO..=models_ui::PREPARE).contains(&id) {
                    if [models_ui::REMOTE, models_ui::LOCAL].contains(&id)
                        && notification == CBN_SELCHANGE
                    {
                        (*pointer).library.selected_index(hwnd, id);
                        monitor_launch(hwnd, &mut *pointer);
                        InvalidateRect(hwnd, null(), 0);
                    } else if notification == BN_CLICKED
                        && ![
                            models_ui::REPO,
                            models_ui::DIRECTORY,
                            models_ui::DESTINATION,
                            models_ui::REMOTE,
                            models_ui::LOCAL,
                        ]
                        .contains(&id)
                    {
                        let app = &*pointer;
                        let running = app.snapshot.lock().expect("Snapshot mutex poisoned").online
                            || app.launch.as_ref().is_some_and(|l| l.running);
                        let result = if [
                            models_ui::MOVE,
                            models_ui::DELETE,
                            models_ui::DOWNLOAD,
                            models_ui::PREPARE,
                        ]
                        .contains(&id)
                            && hub_ui::modified(hwnd)
                        {
                            Err("Save model settings before changing or preparing models.".into())
                        } else if app.updates.status().busy {
                            Err("Wait for the Strata update operation to finish".into())
                        } else {
                            let template = if matches!(id, models_ui::DOWNLOAD | models_ui::PREPARE)
                                && !app.hub.profiles.is_empty()
                            {
                                app.hub.configuration().map(Some)
                            } else {
                                Ok(None)
                            };
                            template.and_then(|template| {
                                app.library
                                    .command(hwnd, &app.models, id, running, template)
                            })
                        };
                        if let Err(error) = result {
                            message(hwnd, &error.to_string());
                        }
                        monitor_launch(hwnd, &mut *pointer);
                    }
                    return 0;
                }
                if hub_ui::edit_notification(hwnd, id, notification) {
                    return 0;
                }
                if (350..600).contains(&id)
                    || (id == hub_ui::PROFILE && notification != CBN_SELCHANGE)
                    || (id != hub_ui::PROFILE && notification != BN_CLICKED)
                {
                    return 0;
                }
                let app = &mut *pointer;
                match id {
                    101..=104 => {
                        app.tab = id - 101;
                        theme::metric_tab(hwnd, app.tab);
                        for tab in 101..=104 {
                            InvalidateRect(GetDlgItem(hwnd, tab), null(), 0);
                        }
                        app.scroll = 0;
                        InvalidateRect(hwnd, null(), 0);
                    }
                    105 => {
                        if let Err(error) = app.refresh.send(()) {
                            message(hwnd, &error.to_string())
                        }
                    }
                    106 => {
                        let snapshot = app
                            .snapshot
                            .lock()
                            .expect("Snapshot mutex poisoned")
                            .clone();
                        match data::export(&snapshot) {
                            Ok(path) => {
                                message(hwnd, &format!("CSV files saved to {}", path.display()))
                            }
                            Err(error) => message(hwnd, &error.to_string()),
                        }
                    }
                    107 | 110 => {
                        if app.models.status().blocks_model {
                            message(
                                hwnd,
                                "Wait for the model file operation before opening model controls.",
                            );
                        } else {
                            set_page(hwnd, app, true);
                        }
                    }
                    111 => set_page(hwnd, app, false),
                    112 | 113 => {
                        if let Err(error) = set_tools_page(hwnd, app, id - 109) {
                            message(hwnd, &error.to_string());
                        }
                    }
                    115 => {
                        if let Err(error) = set_tools_page(hwnd, app, 5).and_then(|()| {
                            if app.models.status().busy {
                                Ok(())
                            } else {
                                app.library.command(
                                    hwnd,
                                    &app.models,
                                    models_ui::REFRESH,
                                    false,
                                    None,
                                )
                            }
                        }) {
                            message(hwnd, &error.to_string());
                        }
                    }
                    600..=732 => {
                        if let Err(error) = tools_command(hwnd, app, id) {
                            message(hwnd, &error.to_string());
                        }
                    }
                    300..=313 => {
                        if let Err(error) = hub_command(hwnd, app, id, (w >> 16) as u32) {
                            message(hwnd, &error.to_string());
                        }
                    }
                    108 => {
                        if let Err(error) = stop_strata(hwnd, app) {
                            message(hwnd, &error.to_string())
                        }
                    }
                    _ => (),
                };
                0
            }
            TRAY => {
                match l as u32 {
                    WM_LBUTTONDBLCLK => {
                        ShowWindow(hwnd, SW_RESTORE);
                        SetForegroundWindow(hwnd);
                    }
                    WM_RBUTTONUP => {
                        let menu = CreatePopupMenu();
                        AppendMenuW(menu, MF_STRING, 201, wide("Open dashboard").as_ptr());
                        AppendMenuW(
                            menu,
                            MF_STRING,
                            202,
                            wide("Exit (keep Strata running)").as_ptr(),
                        );
                        let mut point: POINT = std::mem::zeroed();
                        GetCursorPos(&mut point);
                        SetForegroundWindow(hwnd);
                        let chosen =
                            TrackPopupMenu(menu, TPM_RETURNCMD, point.x, point.y, 0, hwnd, null());
                        DestroyMenu(menu);
                        if chosen == 201 {
                            ShowWindow(hwnd, SW_RESTORE);
                            SetForegroundWindow(hwnd);
                        } else if chosen == 202 {
                            PostMessageW(hwnd, WM_COMMAND, hub_ui::EXIT, 0);
                        }
                    }
                    _ => (),
                };
                0
            }
            WM_CLOSE => {
                ShowWindow(hwnd, SW_HIDE);
                0
            }
            WM_DESTROY => {
                let app = &mut *pointer;
                Shell_NotifyIconW(NIM_DELETE, &app.tray);
                theme::dispose(hwnd);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

fn run() -> Result<()> {
    unsafe {
        let args: Vec<String> = std::env::args().collect();
        let value = |flag: &str| -> Result<Option<&str>> {
            args.iter()
                .position(|argument| argument == flag)
                .map(|index| {
                    args.get(index + 1)
                        .filter(|value| !value.starts_with("--"))
                        .map(String::as_str)
                        .ok_or_else(|| format!("Missing value for {flag}").into())
                })
                .transpose()
        };
        if let Some(pid) = value("--wait-for-parent")? {
            let handle = OpenProcess(0x00100000, 0, pid.parse()?);
            if !handle.is_null() {
                let result = WaitForSingleObject(handle, 10000);
                CloseHandle(handle);
                if result != WAIT_OBJECT_0 {
                    return Err("The previous hub process did not exit".into());
                }
            } else if GetLastError() != ERROR_INVALID_PARAMETER {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        let settings_file = match value("--config")? {
            Some(file) => std::path::absolute(file)?,
            None => config::default_file()?,
        };
        let existing = config::read_if_present(&settings_file)?;
        if args.iter().any(|argument| argument == "--settings") {
            settings_ui::edit(null_mut(), &settings_file, existing.as_ref())?;
            return Ok(());
        }
        let paths = if let Some(paths) = existing {
            paths
        } else {
            if value("--config")?.is_some() {
                return Err(format!("Saved hub settings not found: {}. Restore this file, or open --settings to choose the installation paths. No setup or download was started.", settings_file.display()).into());
            }
            if args
                .iter()
                .any(|argument| matches!(argument.as_str(), "--check" | "--background"))
            {
                return Err("No hub settings found. Run Strata Hub normally to complete first-run setup, or pass --config <file>.".into());
            }
            if !settings_ui::edit(null_mut(), &settings_file, None)? {
                return Ok(());
            }
            config::read(&settings_file)?
        };
        paths.validate().map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("{error}. Saved paths in {} have been retained. Check that the configured drives are available, or use --settings to edit the paths. No setup or download was started.", settings_file.display()).into()
        })?;
        config::set(settings_file, paths)?;
        data::initialize(&config::get().paths)?;
        if args.iter().any(|s| s == "--check") {
            data::check()?;
            let mut hub = profiles::Hub::load()?;
            let mut checked = Vec::new();
            for selected in 0..hub.profiles.len() {
                hub.selected = selected;
                let config = hub.configuration()?;
                checked.push(serde_json::json!({"profile":hub.profiles[selected].file,"model":config["model_name"],"vision":hub.profiles[selected].settings.vision,"engine_arguments":config["args"].as_array().ok_or("Invalid arguments")?.len()}));
            }
            std::fs::write(
                data::root().join("hub-check.json"),
                serde_json::to_vec_pretty(&checked)?,
            )?;
            return Ok(());
        }
        let report_only = args.iter().any(|s| s == "--report-only");
        let background = args.iter().any(|s| s == "--background");
        let auto_huihui = args.iter().any(|s| s == "--start-huihui");
        let name = wide("Local\\StrataNativeDashboard");
        let mutex = CreateMutexW(null(), 0, name.as_ptr());
        if mutex.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let existing = FindWindowW(wide(CLASS).as_ptr(), null());
            if !existing.is_null() {
                if !background {
                    PostMessageW(
                        existing,
                        START,
                        if auto_huihui {
                            1
                        } else if report_only {
                            2
                        } else {
                            0
                        },
                        0,
                    );
                }
            }
            CloseHandle(mutex);
            return Ok(());
        }
        let initial = data::load(&data::connect()?)?;
        let state = Arc::new(Mutex::new(initial));
        let (sender, receiver) = mpsc::channel();
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
        let fonts = [
            font(-15, FW_NORMAL as i32),
            font(-27, FW_SEMIBOLD as i32),
            font(-23, FW_SEMIBOLD as i32),
            CreateFontW(
                -14,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide("Consolas").as_ptr(),
            ),
        ];
        let hub = profiles::Hub::load()?;
        let initial_profile = if recorded_child()?.is_some() {
            let record: serde_json::Value =
                serde_json::from_slice(&std::fs::read(data::root().join("owned-strata.json"))?)?;
            // The first dashboard version only launched Huihui and stored no config path.
            record
                .get("config")
                .map(|v| {
                    v.as_str()
                        .map(PathBuf::from)
                        .ok_or("Invalid owned profile path")
                })
                .transpose()?
                .or_else(|| {
                    hub.profiles
                        .get(hub.selected)
                        .map(|profile| profile.path.clone())
                })
        } else {
            hub.profiles
                .get(hub.selected)
                .map(|profile| profile.path.clone())
        };
        let auth_profile = Arc::new(Mutex::new(initial_profile));
        let worker_profile = auth_profile.clone();
        let app = Box::new(App {
            snapshot: state.clone(),
            refresh: sender,
            child: None,
            tab: 0,
            scroll: 0,
            visible_rows: 8,
            fonts,
            tray: std::mem::zeroed(),
            hub,
            show_hub: !report_only,
            tools_page: 0,
            updates: jobs::Job::new(
                "strata-update.log",
                "Check for a stable Strata release when you are ready.",
            ),
            agents: jobs::Job::new(
                "strata-agents.log",
                "Scan to find installed agents and available installers.",
            ),
            pending_agent_settings: None,
            syncing_agent_settings: false,
            models: jobs::Job::new(
                "strata-models.log",
                "Analyze a Hugging Face repository or scan local model files.",
            ),
            library: models_ui::Library::default(),
            installed_version: serde_json::from_slice::<serde_json::Value>(&std::fs::read(
                data::strata_home().join("engine/BUILD.json"),
            )?)?["version"]
                .as_str()
                .ok_or("Missing installed engine version")?
                .into(),
            auth_profile,
            launch: match std::fs::read(data::root().join("owned-strata.json")) {
                Ok(bytes) => Some(launch::Launch::from_record(&serde_json::from_slice(
                    &bytes,
                )?)?),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            },
        });
        let pointer = Box::into_raw(app);
        let instance = GetModuleHandleW(null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: null(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hIcon: theme::icon(32),
            ..std::mem::zeroed()
        };
        // Keep the class-name allocation alive through RegisterClassW.
        let class_name = wide(CLASS);
        let class = WNDCLASSW {
            lpszClassName: class_name.as_ptr(),
            ..class
        };
        if RegisterClassW(&class) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("Strata Hub").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1420,
            910,
            null_mut(),
            null_mut(),
            instance,
            pointer.cast(),
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        theme::dark_title(hwnd)?;
        let window_id = hwnd as isize;
        thread::spawn(move || worker(state, receiver, window_id, worker_profile));
        ShowWindow(hwnd, if background { SW_HIDE } else { SW_SHOW });
        if !background {
            UpdateWindow(hwnd);
        }
        if auto_huihui {
            PostMessageW(hwnd, START, 1, 0);
        }
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result == 0 {
                break;
            }
            if result < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let app = Box::from_raw(pointer);
        for font in app.fonts {
            DeleteObject(font);
        }
        CloseHandle(mutex);
        Ok(())
    }
}
fn main() {
    if let Err(error) = run() {
        log_error(&error.to_string());
        if !std::env::args().any(|arg| arg == "--check") {
            message(
                null_mut(),
                &format!("Could not start the dashboard: {error}"),
            );
        }
        std::process::exit(1);
    }
}
