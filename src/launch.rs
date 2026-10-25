use crate::{
    data::{self, Result},
    process_created,
};
use serde_json::Value;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::PathBuf,
};
use windows_sys::Win32::{Foundation::*, System::Threading::*};

const TAIL_BYTES: u64 = 8192;

struct LogTail {
    path: PathBuf,
    start: u64,
    position: u64,
    text: String,
}
impl LogTail {
    fn new(path: PathBuf, start: u64) -> Self {
        Self {
            path,
            start,
            position: start,
            text: String::new(),
        }
    }
    fn poll(&mut self) -> Result<String> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
            Err(error) => return Err(error.into()),
        };
        let length = file.metadata()?.len();
        if length < self.position {
            return Err(format!(
                "The log was truncated during startup: {}",
                self.path.display()
            )
            .into());
        }
        file.seek(SeekFrom::Start(
            self.position
                .max(length.saturating_sub(TAIL_BYTES))
                .max(self.start),
        ))?;
        let mut bytes = Vec::new();
        file.take(TAIL_BYTES).read_to_end(&mut bytes)?;
        self.position = length;
        let chunk = String::from_utf8_lossy(&bytes).replace('\0', "");
        self.text.push_str(&chunk);
        if self.text.len() > TAIL_BYTES as usize {
            let mut cut = self.text.len() - TAIL_BYTES as usize;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.text.drain(..cut);
        }
        Ok(chunk)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    Starting,
    Weights,
    Experts,
    Gpu,
    Finishing,
    Ready,
    Failed,
}
impl Phase {
    pub fn step(self) -> usize {
        match self {
            Self::Starting => 0,
            Self::Weights => 1,
            Self::Experts => 2,
            Self::Gpu => 3,
            Self::Finishing => 4,
            Self::Ready => 5,
            Self::Failed => 0,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Starting => "Starting the server",
            Self::Weights => "Reading model weights",
            Self::Experts => "Loading experts into RAM",
            Self::Gpu => "Filling the GPU cache",
            Self::Finishing => "Verifying the service",
            Self::Ready => "Model ready",
            Self::Failed => "Strata exited",
        }
    }
    fn read(self, line: &str) -> Self {
        let next = if line.contains("ready: http") || line.contains("almost ready") {
            Self::Finishing
        } else if line.contains("filling the GPU") || line.contains("experts loaded:") {
            Self::Gpu
        } else if line.contains("loading the experts")
            || line.contains("loading the most-used experts")
        {
            Self::Experts
        } else if line.contains("reading the model's weights") {
            Self::Weights
        } else {
            self
        };
        if next.step() > self.step() {
            next
        } else {
            self
        }
    }
}

pub struct Launch {
    handle: Option<OwnedHandle>,
    pub started: f64,
    pub phase: Phase,
    pub error: String,
    pub label: String,
    pub requested: u32,
    pub running: bool,
    stdout: LogTail,
    stderr: LogTail,
    engine: LogTail,
}
impl Launch {
    pub fn from_record(record: &Value) -> Result<Self> {
        let pid = u32::try_from(record["pid"].as_u64().ok_or("Invalid owned PID")?)?;
        let created = record["created"]
            .as_u64()
            .ok_or("Invalid owned creation time")?;
        let handle = unsafe {
            let raw = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if raw.is_null() {
                if GetLastError() != ERROR_INVALID_PARAMETER {
                    return Err(std::io::Error::last_os_error().into());
                }
                None
            } else {
                let owned = OwnedHandle::from_raw_handle(raw);
                if process_created(raw)? == created {
                    Some(owned)
                } else {
                    None
                }
            }
        };
        Self::from_handle(record, handle)
    }
    pub fn from_owned(record: &Value, handle: OwnedHandle) -> Result<Self> {
        Self::from_handle(record, Some(handle))
    }
    fn from_handle(record: &Value, handle: Option<OwnedHandle>) -> Result<Self> {
        let created = record["created"]
            .as_u64()
            .ok_or("Invalid owned creation time")?;
        let log = |key: &str, file: PathBuf| -> Result<LogTail> {
            // Older ownership records predate log offsets: start observing at the current end.
            let offset = match record.get(key) {
                Some(value) => value.as_u64().ok_or("Invalid saved log offset")?,
                None => match std::fs::metadata(&file) {
                    Ok(m) => m.len(),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
                    Err(e) => return Err(e.into()),
                },
            };
            Ok(LogTail::new(file, offset))
        };
        Ok(Self {
            running: handle.is_some(),
            handle,
            started: created as f64 / 10_000_000.0 - 11644473600.0,
            phase: Phase::Starting,
            error: String::new(),
            label: record["label"].as_str().unwrap_or("Strata").into(),
            requested: record["parallel"].as_u64().unwrap_or(1) as u32,
            stdout: log(
                "stdout_start",
                data::root().to_path_buf().join("strata-native-stdout.log"),
            )?,
            stderr: log(
                "stderr_start",
                data::root().to_path_buf().join("strata-native-stderr.log"),
            )?,
            engine: log(
                "engine_start",
                record["engine_log"]
                    .as_str()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| data::root().to_path_buf().join("strata-native-stderr.log")),
            )?,
        })
    }
    pub fn failed(&self) -> bool {
        self.phase == Phase::Failed
    }
    pub fn poll(&mut self, online: bool) -> Result<()> {
        let missing = self.handle.is_none();
        let exit = if let Some(handle) = &self.handle {
            let mut code = 0;
            if unsafe { GetExitCodeProcess(handle.as_raw_handle(), &mut code) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            (code != STILL_ACTIVE as u32).then_some(code)
        } else {
            None
        };
        self.running = !missing && exit.is_none();
        if self.failed() {
            return Ok(());
        }
        if self.phase != Phase::Ready || !self.running || !online {
            self.stdout.poll()?;
            for line in self.stdout.text.lines() {
                self.phase = self.phase.read(line);
            }
            self.stderr.poll()?;
        }
        if !self.running {
            self.engine.poll()?;
            self.phase = Phase::Failed;
            let reason = self
                .stderr
                .text
                .lines()
                .rev()
                .chain(self.engine.text.lines().rev())
                .find(|s| !s.trim().is_empty())
                .unwrap_or("No details were written to the logs");
            self.error = match exit {
                Some(code) => format!("Process exited (code {code}). {reason}"),
                None => format!("Process is no longer running. {reason}"),
            };
        } else if online {
            self.phase = Phase::Ready;
        }
        Ok(())
    }
    pub fn summary(&self, online: bool, slots: u32) -> String {
        if self.failed() {
            return self.error.clone();
        }
        if self.phase == Phase::Ready {
            if !online {
                return "Service unreachable · process still running".into();
            }
            if self.requested > slots {
                return format!("Ready · {slots}/{} slots available", self.requested);
            }
            return format!(
                "Ready · {slots} {}",
                if slots == 1 {
                    "request at a time"
                } else {
                    "concurrent requests"
                }
            );
        }
        let seconds = (data::now() - self.started).max(0.0) as u64;
        format!(
            "{} · {}:{:02}",
            self.phase.label(),
            seconds / 60,
            seconds % 60
        )
    }
    pub fn details(&self, online: bool, slots: u32) -> String {
        format!(
            "{}\n{}\n\n{}\n\nServer:\n{}\n\nErrors:\n{}\n\nEngine:\n{}",
            self.label,
            data::local_date(self.started, true),
            self.summary(online, slots),
            self.stdout.text,
            self.stderr.text,
            self.engine.text
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::windows::process::CommandExt, process::Command};
    #[test]
    fn startup_failure_uses_current_run_logs_and_process_exit() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("strata-launch-test-{}", std::process::id()));
        std::fs::create_dir(&directory)?;
        let stdout_path = directory.join("stdout.log");
        let stderr_path = directory.join("stderr.log");
        let engine_path = directory.join("engine.log");
        let old = "Previous launch error\n";
        std::fs::write(&stdout_path, "[strata] loading the experts into RAM\n")?;
        std::fs::write(
            &stderr_path,
            format!("{old}RuntimeError: CUDA out of memory\n"),
        )?;
        std::fs::write(&engine_path, "")?;
        let mut child = Command::new("cmd.exe")
            .args(["/C", "exit", "/B", "23"])
            .creation_flags(crate::CREATE_NO_WINDOW)
            .spawn()?;
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, child.id()) };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut launch = Launch {
            handle: Some(unsafe { OwnedHandle::from_raw_handle(raw) }),
            started: data::now(),
            phase: Phase::Starting,
            error: String::new(),
            label: "Fixture".into(),
            requested: 2,
            running: true,
            stdout: LogTail::new(stdout_path.clone(), 0),
            stderr: LogTail::new(stderr_path.clone(), old.len() as u64),
            engine: LogTail::new(engine_path.clone(), 0),
        };
        child.wait()?;
        launch.poll(false)?;
        assert_eq!(launch.phase, Phase::Failed);
        assert!(!launch.running);
        assert!(launch.error.contains("23"));
        assert!(launch.error.contains("CUDA out of memory"));
        assert!(!launch.details(false, 1).contains("Previous launch error"));
        launch.phase = Phase::Ready;
        assert!(launch.summary(true, 1).contains("1/2 slot"));
        for file in [stdout_path, stderr_path, engine_path] {
            std::fs::remove_file(file)?;
        }
        std::fs::remove_dir(directory)?;
        Ok(())
    }
    #[test]
    fn startup_phases_follow_observed_logs_without_marking_api_ready() {
        let mut phase = Phase::Starting;
        for (line, expected) in [
            (
                "[strata] starting the engine: reading the model's weights ...",
                Phase::Weights,
            ),
            (
                "[strata] loading the experts into RAM (about 45 GB)",
                Phase::Experts,
            ),
            (
                "[strata] still starting (30 s) - please wait ...",
                Phase::Experts,
            ),
            (
                "[strata] filling the GPU's expert cache (3048 experts)",
                Phase::Gpu,
            ),
            ("[strata] almost ready ...", Phase::Finishing),
            ("ready: http://127.0.0.1:8082/v1", Phase::Finishing),
        ] {
            phase = phase.read(line);
            assert_eq!(phase, expected);
        }
    }
}
