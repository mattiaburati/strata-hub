use crate::{config::Paths, data::Result, jobs::Job};
use std::{fs, path::Path, process::Command};
use windows_sys::Win32::Foundation::HWND;

pub const MODELS: &[(&str, &str)] = &[
    ("recommended", "Qwen - recommended size"),
    ("qwen-q2", "Qwen Q2_0"),
    ("qwen-iq2", "Qwen IQ2_XS"),
    ("qwen-iq3xxs", "Qwen IQ3_XXS"),
    ("qwen-iq3s", "Qwen IQ3_S"),
    ("swift", "Swift - recommended size"),
    ("coder", "Coder - recommended size"),
];

pub fn start(hwnd: HWND, file: &Path, paths: &Paths, model: usize) -> Result<Job> {
    let model = MODELS.get(model).ok_or("Choose an installation model")?.0;
    let parent = file.parent().ok_or("Invalid settings file location")?;
    let helpers = parent.join("helpers");
    fs::create_dir_all(&helpers)?;
    let script = helpers.join("install_strata.ps1");
    fs::write(&script, include_bytes!("../scripts/install_strata.ps1"))?;
    let powershell = std::env::var_os("SystemRoot")
        .ok_or("SystemRoot is unavailable; Windows PowerShell is required")?;
    let mut command =
        Command::new(Path::new(&powershell).join("System32/WindowsPowerShell/v1.0/powershell.exe"));
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .arg("-Root")
        .arg(&paths.strata_dir)
        .arg("-Data")
        .arg(&paths.models_dir)
        .args(["-Model", model]);
    let job = Job::at(
        parent.join("strata-install.log"),
        "Preparing Strata installation",
    );
    job.start_command(hwnd, command, true, true, "Preparing Strata installation")?;
    Ok(job)
}
