use crate::{
    ACCENT, MUTED, PANEL, WHITE, data::Result, hub_ui, jobs::Status, profiles::Hub, rect, text,
    theme, wide,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
};

pub const CHECK: usize = 600;
pub const INSTALL: usize = 601;
pub const RELEASE: usize = 602;
pub const UPDATE_LOG: usize = 603;
pub const SCAN: usize = 700;
pub const AGENT_LOG: usize = 701;
pub const AGENTS: &[&str] = &["pi", "opencode", "codex"];
pub const NAMES: &[&str] = &["Pi", "OpenCode", "Codex CLI"];
fn agent_step(height: i32) -> i32 {
    ((height - 360) / 3).clamp(140, 166)
}

fn button_ids() -> Vec<usize> {
    let mut ids = vec![CHECK, INSTALL, RELEASE, UPDATE_LOG, SCAN, AGENT_LOG];
    for i in 0..AGENTS.len() {
        ids.extend([710 + i * 10, 711 + i * 10, 712 + i * 10]);
    }
    ids
}
pub fn create(hwnd: HWND, font: HFONT) -> Result<()> {
    for (id, label) in [
        (CHECK, "Check for updates"),
        (INSTALL, "Install update"),
        (RELEASE, "Release notes"),
        (UPDATE_LOG, "Open update log"),
        (SCAN, "Scan agents"),
        (AGENT_LOG, "Open agent log"),
    ] {
        hub_ui::add(hwnd, id, "BUTTON", label, BS_OWNERDRAW as u32, font)?;
    }
    for i in 0..AGENTS.len() {
        for (action, label) in [(0, "Install"), (1, "Connect to Strata"), (2, "Launch")] {
            hub_ui::add(
                hwnd,
                710 + i * 10 + action,
                "BUTTON",
                label,
                BS_OWNERDRAW as u32,
                font,
            )?;
        }
    }
    Ok(())
}
pub fn visible(hwnd: HWND, page: usize) {
    unsafe {
        for id in button_ids() {
            let show = (page == 3 && id < 700) || (page == 4 && id >= 700);
            ShowWindow(
                GetDlgItem(hwnd, id as i32),
                if show { SW_SHOW } else { SW_HIDE },
            );
        }
    }
}
pub fn resize(hwnd: HWND) {
    unsafe {
        let mut client: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut client);
        let width = client.right - theme::SIDE;
        for (id, x, w) in [(CHECK, 52, 180), (INSTALL, 248, 164), (RELEASE, 428, 160)] {
            hub_ui::move_control(hwnd, id, rect(x, 290, w, 42));
        }
        hub_ui::move_control(hwnd, UPDATE_LOG, rect(28, client.bottom - 90, 168, 40));
        hub_ui::move_control(hwnd, SCAN, rect(width - 232, 142, 180, 42));
        hub_ui::move_control(hwnd, AGENT_LOG, rect(28, client.bottom - 90, 160, 40));
        for i in 0..AGENTS.len() {
            let step = agent_step(client.bottom);
            let y = 240 + i as i32 * step;
            for (action, x, w) in [
                (0, width - 500, 104),
                (1, width - 380, 200),
                (2, width - 164, 112),
            ] {
                hub_ui::move_control(hwnd, 710 + i * 10 + action, rect(x, y + step - 66, w, 36));
            }
        }
    }
}
pub fn sync(
    hwnd: HWND,
    updates: &Status,
    agents: &Status,
    model_running: bool,
    port: Option<u16>,
    context: Option<&str>,
    vision: Option<bool>,
) {
    unsafe {
        for (id, enabled) in [
            (CHECK, !updates.busy),
            (
                INSTALL,
                !updates.busy
                    && !updates.error
                    && updates.data["available"].as_bool() == Some(true)
                    && !model_running,
            ),
            (RELEASE, updates.data["url"].is_string()),
            (SCAN, !agents.busy && port.is_some()),
        ] {
            EnableWindow(GetDlgItem(hwnd, id as i32), enabled as i32);
        }
        for i in 0..AGENTS.len() {
            let row = &agents.data["agents"][i];
            let found = row["id"].as_str() == Some(AGENTS[i]);
            let installed = found && row["installed"].as_bool() == Some(true);
            let target_matches = port.is_some_and(|port| {
                agents.data["endpoint"].as_str()
                    == Some(format!("http://127.0.0.1:{port}/v1").as_str())
            });
            let connected = installed && target_matches && row["connected"].as_bool() == Some(true);
            let launch = connected && row["launcher"].as_bool() == Some(true);
            let label = if connected {
                if row["context"].as_u64() == context.and_then(|value| value.parse::<u64>().ok())
                    && row["context"].as_u64().is_some()
                    && row["vision"].as_bool() == vision
                {
                    "Reconnect"
                } else {
                    "Update connection"
                }
            } else {
                "Connect to Strata"
            };
            let button = GetDlgItem(hwnd, (711 + i * 10) as i32);
            let mut current = [0; 64];
            let len = GetWindowTextW(button, current.as_mut_ptr(), current.len() as i32);
            if String::from_utf16_lossy(&current[..len.max(0) as usize]) != label {
                SetWindowTextW(button, wide(label).as_ptr());
            }
            for (action, enabled) in [(0, found && !installed), (1, installed), (2, launch)] {
                EnableWindow(
                    GetDlgItem(hwnd, (710 + i * 10 + action) as i32),
                    (enabled && !agents.busy && port.is_some()) as i32,
                );
            }
        }
    }
}
fn wrapped(dc: HDC, bounds: RECT, value: &str, color: u32, font: HFONT) {
    unsafe {
        let old = SelectObject(dc, font);
        SetTextColor(dc, color);
        SetBkMode(dc, TRANSPARENT as i32);
        let mut bounds = bounds;
        DrawTextW(
            dc,
            wide(value).as_ptr(),
            -1,
            &mut bounds,
            DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, old);
    }
}
pub fn paint_updates(
    dc: HDC,
    fonts: &[HFONT; 4],
    width: i32,
    height: i32,
    status: &Status,
    installed: &str,
    model_running: bool,
) {
    theme::rounded(
        dc,
        rect(28, 112, width - 56, 244),
        PANEL,
        Some(theme::BORDER),
        16,
    );
    text(
        dc,
        rect(52, 128, width - 340, 34),
        "Strata releases",
        WHITE,
        fonts[2],
    );
    theme::badge(
        dc,
        rect(width - 228, 132, 176, 28),
        if model_running {
            "Model running"
        } else {
            "Model stopped"
        },
        if model_running {
            theme::WARNING
        } else {
            ACCENT
        },
        fonts[0],
    );
    for (x, label, value) in [
        (
            52,
            "INSTALLED",
            status.data["current"].as_str().unwrap_or(installed),
        ),
        (
            width / 2,
            "LATEST STABLE",
            status.data["latest"].as_str().unwrap_or("Not checked"),
        ),
    ] {
        text(dc, rect(x, 184, 300, 24), label, MUTED, fonts[0]);
        text(dc, rect(x, 218, 360, 34), value, WHITE, fonts[2]);
    }
    theme::rounded(
        dc,
        rect(28, 380, width - 56, 220),
        PANEL,
        Some(theme::BORDER),
        16,
    );
    text(
        dc,
        rect(52, 396, width - 104, 34),
        "Update status",
        WHITE,
        fonts[2],
    );
    wrapped(
        dc,
        rect(52, 450, width - 104, 64),
        &status.message,
        if status.error {
            theme::ERROR
        } else if status.busy {
            theme::WARNING
        } else {
            ACCENT
        },
        fonts[0],
    );
    wrapped(
        dc,
        rect(52, 536, width - 104, 44),
        if model_running {
            "Stop the running model to install an update. You can check releases while Strata is in use."
        } else {
            "Install when ready. Engine and configuration backups are retained; model files stay in place."
        },
        MUTED,
        fonts[0],
    );
    text(
        dc,
        rect(220, height - 90, width - 248, 40),
        "Stable releases · Manual installation",
        MUTED,
        fonts[0],
    );
}
fn context_label(context: u64) -> String {
    if context % 1024 == 0 {
        format!("{}K", context / 1024)
    } else {
        format!("{context} tokens")
    }
}
pub fn paint_agents(
    dc: HDC,
    fonts: &[HFONT; 4],
    width: i32,
    height: i32,
    hub: &Hub,
    status: &Status,
) {
    let Some(profile) = hub.profiles.get(hub.selected) else {
        text(
            dc,
            rect(52, 136, width - 104, 40),
            "Prepare a model profile before connecting coding agents.",
            MUTED,
            fonts[0],
        );
        return;
    };
    wrapped(
        dc,
        rect(204, height - 90, width - 232, 44),
        &status.message,
        if status.error { theme::ERROR } else { MUTED },
        fonts[0],
    );
    theme::rounded(
        dc,
        rect(28, 112, width - 56, 112),
        PANEL,
        Some(theme::BORDER),
        16,
    );
    text(
        dc,
        rect(52, 126, width - 340, 32),
        "Local connection",
        WHITE,
        fonts[2],
    );
    text(
        dc,
        rect(52, 162, width - 340, 24),
        &profile.label,
        MUTED,
        fonts[0],
    );
    let target_context = profile.settings.values[0].parse::<u64>().ok();
    theme::badge(
        dc,
        rect(width - 424, 188, 180, 26),
        &format!(
            "Context {}",
            target_context
                .map(context_label)
                .unwrap_or_else(|| "Invalid".into())
        ),
        WHITE,
        fonts[0],
    );
    text(
        dc,
        rect(52, 188, width - 500, 26),
        &format!("http://127.0.0.1:{}/v1 · strata", profile.settings.port),
        MUTED,
        fonts[0],
    );
    theme::badge(
        dc,
        rect(width - 232, 188, 180, 26),
        if profile.settings.vision {
            "Images enabled"
        } else {
            "Text only"
        },
        MUTED,
        fonts[0],
    );
    for (i, name) in NAMES.iter().enumerate() {
        let step = agent_step(height);
        let y = 240 + i as i32 * step;
        let row = &status.data["agents"][i];
        let found = row["id"].as_str() == Some(AGENTS[i]);
        let installed = found && row["installed"].as_bool() == Some(true);
        let target_matches = status.data["endpoint"].as_str()
            == Some(format!("http://127.0.0.1:{}/v1", profile.settings.port).as_str());
        let connected = found && target_matches && row["connected"].as_bool() == Some(true);
        let agent_context = row["context"].as_u64();
        let current = connected
            && agent_context.is_some()
            && agent_context == target_context
            && row["vision"].as_bool() == Some(profile.settings.vision);
        theme::rounded(
            dc,
            rect(28, y, width - 56, step - 16),
            PANEL,
            Some(theme::BORDER),
            16,
        );
        text(dc, rect(52, y + 12, 200, 32), name, WHITE, fonts[2]);
        theme::badge(
            dc,
            rect(width - 252, y + 14, 200, 28),
            if !found {
                "Scan to detect"
            } else if current && installed {
                "Connected"
            } else if connected && installed {
                "Update needed"
            } else if installed {
                "Installed"
            } else {
                "Not installed"
            },
            if current {
                ACCENT
            } else if connected {
                theme::WARNING
            } else {
                MUTED
            },
            fonts[0],
        );
        text(
            dc,
            rect(52, y + 49, width - 104, 24),
            row["path"]
                .as_str()
                .unwrap_or("Scan to detect installed clients"),
            MUTED,
            fonts[0],
        );
        let detail = if let Some(error) = row["config_error"]
            .as_str()
            .filter(|value| !value.is_empty())
        {
            error.to_string()
        } else if connected {
            match (agent_context, target_context) {
                (Some(actual), Some(target)) if actual != target => format!(
                    "Context {} → {}",
                    context_label(actual),
                    context_label(target)
                ),
                (Some(actual), _) => format!(
                    "Client {} · {}",
                    context_label(actual),
                    match row["vision"].as_bool() {
                        Some(true) => "Images enabled",
                        Some(false) => "Text only",
                        None => "Images not configured",
                    }
                ),
                _ => "Client context not configured".into(),
            }
        } else if installed {
            "Ready to connect".into()
        } else {
            String::new()
        };
        text(
            dc,
            rect(52, y + step - 66, (width - 584).max(100), 36),
            &detail,
            if connected && !current || !row["config_error"].as_str().unwrap_or("").is_empty() {
                theme::WARNING
            } else {
                MUTED
            },
            fonts[0],
        );
    }
}
