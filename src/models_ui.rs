use crate::{
    ACCENT, MUTED, PANEL, WHITE,
    data::{self, Result},
    hub_ui,
    jobs::{Job, Status},
    rect, text, theme, wide,
};
use serde_json::Value;
use std::{fs, path::Path, process::Command};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
};

pub const REPO: usize = 900;
pub const ANALYZE: usize = 901;
pub const REMOTE: usize = 902;
pub const DOWNLOAD: usize = 903;
pub const DIRECTORY: usize = 904;
pub const LOCAL: usize = 905;
pub const REFRESH: usize = 906;
pub const MOVE: usize = 907;
pub const DELETE: usize = 908;
pub const DESTINATION: usize = 909;
pub const LOG: usize = 910;
pub const PREPARE: usize = 911;

#[derive(Default)]
pub struct Library {
    pub catalog: Value,
    pub local: Vec<Value>,
    remote_index: usize,
    local_index: usize,
    catalog_generation: u64,
    library_generation: u64,
    plan_generation: u64,
    pub profiles_generation: u64,
    pub empty_inventory_generation: u64,
    pub reload_after_operation: bool,
}
fn read(path: &Path) -> Result<Value> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(
        bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes),
    )?)
}
fn input(hwnd: HWND, id: usize) -> String {
    unsafe {
        let item = GetDlgItem(hwnd, id as i32);
        let mut buffer = vec![0; GetWindowTextLengthW(item).max(0) as usize + 1];
        let n = GetWindowTextW(item, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..n.max(0) as usize])
    }
}
fn combo(hwnd: HWND, id: usize, rows: impl Iterator<Item = String>) {
    unsafe {
        let item = GetDlgItem(hwnd, id as i32);
        SendMessageW(item, CB_RESETCONTENT, 0, 0);
        for row in rows {
            SendMessageW(item, CB_ADDSTRING, 0, wide(&row).as_ptr() as LPARAM);
        }
        SendMessageW(item, CB_SETCURSEL, 0, 0);
    }
}
fn gigabytes(row: &Value) -> String {
    format!("{:.2} GB", row["size"].as_f64().unwrap_or(0.0) / 1e9)
}
fn label(kind: &str) -> &str {
    match kind {
        "known" => "Known Strata layout",
        "layout" => "Compatible layout; fine-tune untested",
        "no" => "Incompatible",
        "untested" => "Untested layout",
        "family" => "Separate engine path",
        _ => "Inspection failed",
    }
}
impl Library {
    fn remote(&self) -> Option<&Value> {
        self.catalog["variants"].as_array()?.get(self.remote_index)
    }
    fn selected(&self) -> Option<&Value> {
        self.local.get(self.local_index)
    }
    pub fn sync(&mut self, hwnd: HWND, status: &Status) -> Result<Option<Value>> {
        let generation = status.data["catalog_generation"].as_u64().unwrap_or(0);
        if generation != self.catalog_generation {
            self.catalog = read(&data::root().join("model-catalog.json"))?;
            self.catalog_generation = generation;
            self.remote_index = 0;
            combo(
                hwnd,
                REMOTE,
                self.catalog["variants"]
                    .as_array()
                    .ok_or("Invalid model catalog")?
                    .iter()
                    .map(|row| {
                        format!(
                            "{}  |  {}  |  {}",
                            row["name"].as_str().unwrap_or(""),
                            gigabytes(row),
                            label(row["verdict"].as_str().unwrap_or(""))
                        )
                    }),
            );
        }
        let generation = status.data["library_generation"].as_u64().unwrap_or(0);
        if generation != self.library_generation {
            self.library_generation = generation;
            self.local.clear();
            let registry = data::root().join("model-library.json");
            if registry.is_file() {
                self.local.extend(
                    read(&registry)?["models"]
                        .as_array()
                        .ok_or("Invalid model library")?
                        .clone(),
                );
            }
            let inventory = data::root().join("model-inventory.json");
            if inventory.is_file() {
                for row in read(&inventory)?["existing"]
                    .as_array()
                    .ok_or("Invalid local inventory")?
                {
                    if !self.local.iter().any(|entry| entry["path"] == row["path"]) {
                        self.local.push(row.clone());
                    }
                }
            }
            self.local_index = self.local_index.min(self.local.len().saturating_sub(1));
            combo(
                hwnd,
                LOCAL,
                self.local.iter().map(|row| {
                    format!(
                        "{}  |  {}  |  {}",
                        row["name"].as_str().unwrap_or(""),
                        gigabytes(row),
                        row["state"].as_str().unwrap_or("")
                    )
                }),
            );
            unsafe {
                SendMessageW(
                    GetDlgItem(hwnd, LOCAL as i32),
                    CB_SETCURSEL,
                    self.local_index,
                    0,
                );
            }
        }
        let generation = status.data["plan_generation"].as_u64().unwrap_or(0);
        if generation != self.plan_generation && !status.busy {
            self.plan_generation = generation;
            return Ok(Some(read(&data::root().join("model-operation.json"))?));
        }
        Ok(None)
    }
    pub fn selected_index(&mut self, hwnd: HWND, id: usize) {
        unsafe {
            let index = SendMessageW(GetDlgItem(hwnd, id as i32), CB_GETCURSEL, 0, 0);
            if index >= 0 {
                if id == REMOTE {
                    self.remote_index = index as usize;
                } else {
                    self.local_index = index as usize;
                }
            }
        }
    }
    pub fn command(
        &self,
        hwnd: HWND,
        job: &Job,
        id: usize,
        model_running: bool,
        template: Option<Value>,
    ) -> Result<()> {
        if id == LOG {
            return crate::open_log(&job.log);
        }
        let mut args = common();
        if matches!(id, DOWNLOAD | PREPARE) {
            if model_running {
                return Err("Stop Strata before downloading and preparing a model".into());
            }
            if let Some(template) = template {
                let path = data::root().join("model-prepare-template.json");
                fs::write(&path, serde_json::to_vec_pretty(&template)?)?;
                args.extend(["--template".into(), path.to_string_lossy().into_owned()]);
            }
        }
        let (mode, message, mutating, blocks) = match id {
            ANALYZE => {
                args.extend(["--repo".into(), input(hwnd, REPO)]);
                ("scan", "Reading Hugging Face repository", false, false)
            }
            DOWNLOAD => {
                let row = self
                    .remote()
                    .ok_or("Analyze a repository and select a GGUF first")?;
                if row["downloadable"].as_bool() != Some(true) {
                    return Err("This GGUF has no supported Strata layout".into());
                }
                args.extend([
                    "--id".into(),
                    row["id"].as_str().ok_or("Invalid GGUF identifier")?.into(),
                    "--destination".into(),
                    input(hwnd, DIRECTORY),
                ]);
                (
                    "download",
                    "Downloading and preparing selected model",
                    true,
                    true,
                )
            }
            PREPARE => {
                let row = self.selected().ok_or("Select a local model")?;
                args.extend([
                    "--id".into(),
                    row["id"]
                        .as_str()
                        .ok_or("Invalid library identifier")?
                        .into(),
                ]);
                ("prepare", "Preparing local model for Strata", true, true)
            }
            REFRESH => ("inventory", "Reading local model files", false, false),
            MOVE | DELETE => {
                if model_running {
                    return Err("Stop Strata before moving or deleting model files".into());
                }
                let row = self.selected().ok_or("Select a local model")?;
                args.extend([
                    "--id".into(),
                    row["id"]
                        .as_str()
                        .ok_or("Invalid library identifier")?
                        .into(),
                ]);
                if id == MOVE {
                    args.extend(["--destination".into(), input(hwnd, DESTINATION)]);
                }
                (
                    if id == MOVE {
                        "plan-move"
                    } else {
                        "plan-delete"
                    },
                    "Checking model files and profile references",
                    false,
                    true,
                )
            }
            _ => return Err("Invalid model library action".into()),
        };
        args.insert(0, mode.into());
        start(hwnd, job, args, mutating, blocks, message)
    }
}
fn common() -> Vec<String> {
    vec![
        "--root".into(),
        data::strata_home().to_string_lossy().into_owned(),
        "--report".into(),
        data::root().to_string_lossy().into_owned(),
        "--models".into(),
        data::model_root().to_string_lossy().into_owned(),
        "--profiles".into(),
        data::profiles_dir().to_string_lossy().into_owned(),
    ]
}
fn start(
    hwnd: HWND,
    job: &Job,
    mut args: Vec<String>,
    mutating: bool,
    blocks: bool,
    message: &str,
) -> Result<()> {
    let directory = data::root().join("helpers");
    fs::create_dir_all(&directory)?;
    let helper = directory.join("model_idle_check.py");
    fs::write(&helper, include_str!("../scripts/update_strata.py"))?;
    args.extend([
        "--idle-helper".into(),
        helper.to_string_lossy().into_owned(),
    ]);
    let script = directory.join("model_library.py");
    fs::write(&script, include_str!("../scripts/model_library.py"))?;
    let mut command = Command::new(data::python());
    command
        .arg("-u")
        .arg(script)
        .args(args)
        .env("PYTHONIOENCODING", "utf-8");
    job.start_command(hwnd, command, mutating, blocks, message)
}
pub fn confirm(hwnd: HWND, job: &Job, plan: &Value) -> Result<()> {
    let action = plan["action"].as_str().ok_or("Invalid model operation")?;
    if !matches!(action, "move" | "delete") {
        return Err("Invalid model operation".into());
    }
    let mut args = common();
    args.insert(0, action.into());
    args.extend([
        "--id".into(),
        plan["id"].as_str().ok_or("Missing model ID")?.into(),
        "--confirmation".into(),
        plan["digest"]
            .as_str()
            .ok_or("Missing operation digest")?
            .into(),
    ]);
    if let Some(target) = plan["target"].as_str() {
        let parent = Path::new(target)
            .parent()
            .ok_or("Invalid model destination")?;
        args.extend([
            "--destination".into(),
            parent.to_string_lossy().into_owned(),
        ]);
    }
    start(
        hwnd,
        job,
        args,
        true,
        true,
        if action == "move" {
            "Moving model files"
        } else {
            "Deleting model files"
        },
    )
}
pub fn confirmation(plan: &Value) -> Result<String> {
    let action = plan["action"].as_str().ok_or("Invalid model operation")?;
    let files = plan["files"].as_array().ok_or("Invalid model file list")?;
    let mut lines = vec![
        if action == "delete" {
            "Permanently delete these model files?".to_string()
        } else {
            "Move these model files?".to_string()
        },
        String::new(),
        plan["source"]
            .as_str()
            .ok_or("Missing model folder")?
            .into(),
    ];
    if let Some(target) = plan["target"].as_str() {
        lines.push(format!("Destination: {target}"));
    }
    lines.push(String::new());
    lines.extend(files.iter().map(|file| {
        format!(
            "{} ({:.2} GB)",
            file["path"].as_str().unwrap_or(""),
            file["size"].as_f64().unwrap_or(0.0) / 1e9
        )
    }));
    if let Some(changes) = plan["changes"].as_array().filter(|rows| !rows.is_empty()) {
        lines.push("\nProfile/pack references to update (backups are retained):".into());
        lines.extend(
            changes
                .iter()
                .map(|row| row["path"].as_str().unwrap_or("").into()),
        );
    }
    if let Some(deletions) = plan["deletions"].as_array().filter(|rows| !rows.is_empty()) {
        lines.push("\nProfiles and launchers to remove (backups are retained):".into());
        lines.extend(
            deletions
                .iter()
                .map(|row| row["path"].as_str().unwrap_or("").into()),
        );
    }
    if action == "delete" {
        lines.push("\nDeletion is permanent and does not use the Recycle Bin.".into());
    }
    let prompt = lines.join("\n");
    if prompt.len() > 24000 {
        return Err("The file list is too large for a complete confirmation dialog. Use a smaller model folder.".into());
    }
    Ok(prompt)
}
pub fn create(hwnd: HWND, font: HFONT) -> Result<()> {
    for (id, value) in [
        (REPO, "".into()),
        (
            DIRECTORY,
            data::model_root()
                .join("models")
                .to_string_lossy()
                .into_owned(),
        ),
        (DESTINATION, String::new()),
    ] {
        hub_ui::add(hwnd, id, "EDIT", &value, ES_AUTOHSCROLL as u32, font)?;
    }
    for id in [REMOTE, LOCAL] {
        hub_ui::add(
            hwnd,
            id,
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32
                | CBS_OWNERDRAWFIXED as u32
                | CBS_HASSTRINGS as u32
                | WS_VSCROLL,
            font,
        )?;
    }
    for (id, title) in [
        (ANALYZE, "Analyze repo"),
        (DOWNLOAD, "Download & prepare"),
        (PREPARE, "Prepare model"),
        (REFRESH, "Scan local"),
        (MOVE, "Move"),
        (DELETE, "Delete"),
        (LOG, "Open model log"),
    ] {
        hub_ui::add(hwnd, id, "BUTTON", title, BS_OWNERDRAW as u32, font)?;
    }
    Ok(())
}
pub fn visible(hwnd: HWND, visible: bool) {
    unsafe {
        for id in REPO..=PREPARE {
            ShowWindow(
                GetDlgItem(hwnd, id as i32),
                if visible { SW_SHOW } else { SW_HIDE },
            );
        }
    }
}
pub fn resize(hwnd: HWND) {
    unsafe {
        let mut client: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut client);
        let w = client.right - theme::SIDE;
        for (id, bounds) in [
            (REPO, rect(52, 190, w - 324, 24)),
            (ANALYZE, rect(w - 232, 184, 180, 36)),
            (REMOTE, rect(52, 232, w - 104, 260)),
            (DIRECTORY, rect(52, 376, w - 358, 24)),
            (DOWNLOAD, rect(w - 266, 370, 214, 36)),
            (LOCAL, rect(52, 502, w - 452, 260)),
            (PREPARE, rect(w - 382, 498, 174, 40)),
            (REFRESH, rect(w - 196, 498, 144, 40)),
            (DESTINATION, rect(52, client.bottom - 166, w - 390, 24)),
            (MOVE, rect(w - 298, client.bottom - 172, 112, 36)),
            (DELETE, rect(w - 170, client.bottom - 172, 118, 36)),
            (LOG, rect(28, client.bottom - 90, 160, 40)),
        ] {
            hub_ui::move_control(hwnd, id, bounds);
        }
    }
}
pub fn sync_controls(
    hwnd: HWND,
    library: &Library,
    status: &Status,
    running: bool,
    updating: bool,
) {
    unsafe {
        let idle = !status.busy && !updating;
        for id in REPO..=DESTINATION {
            EnableWindow(GetDlgItem(hwnd, id as i32), idle as i32);
        }
        EnableWindow(
            GetDlgItem(hwnd, DOWNLOAD as i32),
            (idle
                && !running
                && library
                    .remote()
                    .is_some_and(|row| row["downloadable"].as_bool() == Some(true)))
                as i32,
        );
        for id in [MOVE, DELETE] {
            EnableWindow(
                GetDlgItem(hwnd, id as i32),
                (idle && !running && library.selected().is_some()) as i32,
            );
        }
        EnableWindow(
            GetDlgItem(hwnd, PREPARE as i32),
            (idle
                && !running
                && library.selected().is_some_and(|row| {
                    row["state"].as_str() != Some("partial")
                        && row["state"].as_str() != Some("ready")
                        && row["references"].as_array().is_none_or(Vec::is_empty)
                })) as i32,
        );
    }
}
fn wrapped(dc: HDC, bounds: RECT, value: &str, color: u32, font: HFONT) {
    unsafe {
        let previous = SelectObject(dc, font);
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
        SelectObject(dc, previous);
    }
}
pub fn paint(
    hwnd: HWND,
    dc: HDC,
    fonts: &[HFONT; 4],
    width: i32,
    height: i32,
    library: &Library,
    status: &Status,
) {
    theme::rounded(
        dc,
        rect(28, 112, width - 56, 312),
        PANEL,
        Some(theme::BORDER),
        16,
    );
    text(
        dc,
        rect(52, 126, width - 300, 34),
        "Discover models",
        WHITE,
        fonts[2],
    );
    theme::badge(
        dc,
        rect(width - 236, 130, 184, 28),
        "Hugging Face · GGUF",
        MUTED,
        fonts[0],
    );
    for (id, bounds) in [
        (REPO, rect(42, 184, width - 304, 36)),
        (DIRECTORY, rect(42, 370, width - 338, 36)),
    ] {
        theme::input_frame(hwnd, id, dc, bounds);
    }
    text(
        dc,
        rect(52, 342, width - 104, 24),
        "DOWNLOAD DIRECTORY",
        MUTED,
        fonts[0],
    );
    text(
        dc,
        rect(52, 158, width - 104, 24),
        "REPOSITORY URL OR OWNER/MODEL",
        MUTED,
        fonts[0],
    );
    if let Some(row) = library.remote() {
        text(
            dc,
            rect(52, 270, width - 104, 26),
            &format!(
                "{}  |  {}  |  {} shards  |  Strata {}",
                row["quant"].as_str().unwrap_or(""),
                gigabytes(row),
                row["files"].as_array().map_or(0, Vec::len),
                library.catalog["engine"].as_str().unwrap_or("")
            ),
            ACCENT,
            fonts[0],
        );
        wrapped(
            dc,
            rect(52, 300, width - 104, 38),
            row["detail"].as_str().unwrap_or(""),
            if row["downloadable"].as_bool() == Some(true) {
                MUTED
            } else {
                theme::WARNING
            },
            fonts[0],
        );
    } else {
        text(
            dc,
            rect(52, 276, width - 104, 32),
            "Analyze a repository to see quantizations, sizes and Strata compatibility.",
            MUTED,
            fonts[0],
        );
    }
    theme::rounded(
        dc,
        rect(28, 448, width - 56, height - 566),
        PANEL,
        Some(theme::BORDER),
        16,
    );
    text(
        dc,
        rect(52, 458, width - 300, 34),
        "Local library",
        WHITE,
        fonts[2],
    );
    theme::badge(
        dc,
        rect(width - 236, 462, 184, 28),
        &format!("{} models", library.local.len()),
        MUTED,
        fonts[0],
    );
    if let Some(row) = library.selected() {
        text(
            dc,
            rect(52, 542, width - 104, 26),
            row["path"].as_str().unwrap_or(""),
            MUTED,
            fonts[0],
        );
    } else {
        text(
            dc,
            rect(52, 542, width - 104, 26),
            "Scan your configured model directories to populate this library.",
            MUTED,
            fonts[0],
        );
    }
    text(
        dc,
        rect(52, height - 200, width - 104, 24),
        "MOVE TO DIRECTORY",
        MUTED,
        fonts[0],
    );
    theme::input_frame(
        hwnd,
        DESTINATION,
        dc,
        rect(42, height - 172, width - 370, 36),
    );
    wrapped(
        dc,
        rect(204, height - 90, width - 232, 44),
        if status.message.is_empty() {
            "Stop Strata to prepare or manage files. Shared MTP setup may download ~5 GB."
        } else {
            &status.message
        },
        if status.error { theme::ERROR } else { MUTED },
        fonts[0],
    );
}
