use crate::{
    CREATE_NO_WINDOW, UPDATE,
    data::{self, Result},
};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};
use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::PostMessageW};

#[derive(Clone)]
pub struct Status {
    pub busy: bool,
    pub mutating: bool,
    pub blocks_model: bool,
    pub message: String,
    pub error: bool,
    pub data: Value,
}
pub struct Job {
    state: Arc<Mutex<Status>>,
    pub log: PathBuf,
}
impl Job {
    pub fn new(log_name: &str, message: &str) -> Self {
        Self::at(data::root().join(log_name), message)
    }
    pub fn at(log: PathBuf, message: &str) -> Self {
        Self {
            state: Arc::new(Mutex::new(Status {
                busy: false,
                mutating: false,
                blocks_model: false,
                message: message.into(),
                error: false,
                data: json!({}),
            })),
            log,
        }
    }
    pub fn status(&self) -> Status {
        self.state.lock().expect("Job mutex poisoned").clone()
    }
    pub fn fail(&self, message: String) {
        let mut state = self.state.lock().expect("Job mutex poisoned");
        state.error = true;
        state.message = message;
    }
    pub fn finish(&self, message: String) {
        let mut state = self.state.lock().expect("Job mutex poisoned");
        state.error = false;
        state.message = message;
    }
    pub fn tag(&self, key: &str, value: Value) {
        self.state.lock().expect("Job mutex poisoned").data[key] = value;
    }
    pub fn start(
        &self,
        hwnd: HWND,
        script: &'static str,
        arguments: Vec<String>,
        mutating: bool,
        blocks_model: bool,
        message: &str,
    ) -> Result<()> {
        let mut command = Command::new(data::python());
        command
            .args(["-u", "-c", script])
            .args(arguments)
            .env("PYTHONIOENCODING", "utf-8");
        self.start_command(hwnd, command, mutating, blocks_model, message)
    }
    pub fn start_command(
        &self,
        hwnd: HWND,
        mut command: Command,
        mutating: bool,
        blocks_model: bool,
        message: &str,
    ) -> Result<()> {
        {
            let mut state = self.state.lock().expect("Job mutex poisoned");
            if state.busy {
                return Err("An operation is already in progress. Please wait.".into());
            }
            state.busy = true;
            state.mutating = mutating;
            state.blocks_model = blocks_model;
            state.error = false;
            state.message = message.into();
        }
        let state = self.state.clone();
        let path = self.log.clone();
        let window = hwnd as isize;
        thread::spawn(move || {
            let run = || -> Result<()> {
                let mut log = OpenOptions::new().create(true).append(true).open(path)?;
                writeln!(
                    log,
                    "\n{} Starting hub operation",
                    data::local_date(data::now(), true)
                )?;
                let mut child = command
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(log.try_clone()?)
                    .spawn()?;
                let output = child.stdout.take().ok_or("Missing operation output")?;
                let read_result = (|| -> Result<()> {
                    for line in BufReader::new(output).lines() {
                        let line = line?;
                        if line.len() > 65536 {
                            return Err("Operation output line exceeds 64 KiB".into());
                        }
                        writeln!(log, "{line}")?;
                        if let Some(payload) = line.strip_prefix("HUB_EVENT ") {
                            let event: Value = serde_json::from_str(payload)?;
                            let mut status = state.lock().expect("Job mutex poisoned");
                            let kind = event["kind"]
                                .as_str()
                                .ok_or("Missing operation event kind")?;
                            if let Some(message) = event["message"].as_str() {
                                status.message = message.into();
                            }
                            status.error = kind == "error";
                            for (key, value) in
                                event.as_object().ok_or("Invalid operation event")?
                            {
                                if key != "kind" && key != "message" {
                                    status.data[key] = value.clone();
                                }
                            }
                            if kind == "complete" {
                                status.data["available"] = json!(false);
                            }
                            unsafe {
                                PostMessageW(window as HWND, UPDATE, 0, 0);
                            }
                        }
                    }
                    Ok(())
                })();
                let waited = child.wait();
                read_result?;
                let result = waited?;
                if !result.success() {
                    let message = state.lock().expect("Job mutex poisoned").message.clone();
                    return Err(format!("{message} (exit {result})").into());
                }
                Ok(())
            };
            let result = run();
            let mut status = state.lock().expect("Job mutex poisoned");
            if let Err(error) = result {
                status.error = true;
                status.message = error.to_string();
            }
            status.busy = false;
            status.mutating = false;
            status.blocks_model = false;
            unsafe {
                PostMessageW(window as HWND, UPDATE, 0, 0);
            }
        });
        Ok(())
    }
}
