//! Process-scoped memory readings, sampled by the existing metrics worker.
use crate::{
    data::{self, Result},
    wide,
};
use std::{
    collections::BTreeSet,
    os::windows::io::{FromRawHandle, OwnedHandle},
    path::Path,
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::*,
    NetworkManagement::IpHelper::*,
    System::{Diagnostics::ToolHelp::*, Performance::*, ProcessStatus::*, Threading::*},
};

#[derive(Clone, Default)]
pub struct Memory {
    pub ram: Option<u64>,
    pub vram: Option<u64>,
    pub error: String,
}
#[derive(Default)]
pub struct Monitor {
    gpu: Option<std::result::Result<GpuQuery, String>>,
}
impl Monitor {
    pub fn sample(&mut self, profile: Option<&Path>) -> Memory {
        let pids = match profile.map(processes).transpose() {
            Ok(Some(pids)) if !pids.is_empty() => pids,
            Ok(_) => {
                self.gpu = None;
                return Memory::default();
            }
            Err(error) => {
                return Memory {
                    error: format!("Strata memory: {error}"),
                    ..Default::default()
                };
            }
        };
        let mut result = Memory::default();
        match working_set(&pids) {
            Ok(bytes) => result.ram = Some(bytes),
            Err(error) => result.error = format!("Strata RAM: {error}"),
        }
        let gpu = self
            .gpu
            .get_or_insert_with(|| GpuQuery::new().map_err(|error| error.to_string()));
        let measured = match gpu {
            Ok(query) => query.sample(&pids).map_err(|error| error.to_string()),
            Err(error) => Err(error.clone()),
        };
        match measured {
            Ok(bytes) => result.vram = Some(bytes),
            Err(error) => {
                if !result.error.is_empty() {
                    result.error.push_str("; ");
                }
                result
                    .error
                    .push_str(&format!("Strata VRAM unavailable: {error}"));
            }
        }
        result
    }
}

fn processes(profile: &Path) -> Result<BTreeSet<u32>> {
    let bytes = std::fs::read(profile)?;
    let config: serde_json::Value =
        serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))?;
    let port = u16::try_from(config["port"].as_u64().ok_or("Missing Strata port")?)?;
    let mut root = listener(port)?;
    // The owned server can still be loading, before its HTTP listener exists.
    let owned = if root.is_none() {
        crate::recorded_child()?
    } else {
        None
    };
    if let Some((pid, _)) = &owned {
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(data::root().join("owned-strata.json"))?)?;
        let runtime: serde_json::Value = serde_json::from_slice(&std::fs::read(
            record["config"]
                .as_str()
                .ok_or("Missing owned Strata configuration")?,
        )?)?;
        if runtime["port"].as_u64() == Some(u64::from(port)) {
            root = Some(*pid);
        }
    }
    let Some(root) = root else {
        return Ok(BTreeSet::new());
    };
    let tree = process_table()?;
    let mut pids = BTreeSet::from([root]);
    loop {
        let count = pids.len();
        for &(pid, parent) in &tree {
            if pids.contains(&parent) {
                pids.insert(pid);
            }
        }
        if pids.len() == count {
            break;
        }
    }
    Ok(pids)
}
fn process_table() -> Result<Vec<(u32, u32)>> {
    unsafe {
        let handle = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().into());
        }
        let _snapshot = OwnedHandle::from_raw_handle(handle);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(handle, &mut entry) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut tree = Vec::new();
        loop {
            tree.push((entry.th32ProcessID, entry.th32ParentProcessID));
            if Process32NextW(handle, &mut entry) == 0 {
                if GetLastError() != ERROR_NO_MORE_FILES {
                    return Err(std::io::Error::last_os_error().into());
                }
                break;
            }
        }
        Ok(tree)
    }
}

fn listener(port: u16) -> Result<Option<u32>> {
    unsafe {
        let mut size = 0;
        let status =
            GetExtendedTcpTable(null_mut(), &mut size, 0, 2, TCP_TABLE_OWNER_PID_LISTENER, 0);
        if status != ERROR_INSUFFICIENT_BUFFER {
            return Err(format!("TCP owner query failed: {status:#x}").into());
        }
        if size > 4 * 1024 * 1024 {
            return Err("TCP owner table exceeds 4 MiB".into());
        }
        let mut buffer = vec![0u32; (size as usize).div_ceil(4)];
        let status = GetExtendedTcpTable(
            buffer.as_mut_ptr().cast(),
            &mut size,
            0,
            2,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if status != 0 {
            return Err(format!("TCP owner query failed: {status:#x}").into());
        }
        let table = buffer.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
        let count = (*table).dwNumEntries as usize;
        if 4 + count * std::mem::size_of::<MIB_TCPROW_OWNER_PID>() > size as usize {
            return Err("Invalid TCP owner table".into());
        }
        let rows = std::slice::from_raw_parts(
            std::ptr::addr_of!((*table).table).cast::<MIB_TCPROW_OWNER_PID>(),
            count,
        );
        let mut matches = rows.iter().filter(|row| {
            u16::from_be(row.dwLocalPort as u16) == port
                && matches!(row.dwLocalAddr, 0 | 0x0100007f)
        });
        let pid = matches.next().map(|row| row.dwOwningPid);
        if matches.next().is_some() {
            return Err("Multiple listeners match the Strata endpoint".into());
        }
        Ok(pid)
    }
}

fn working_set(pids: &BTreeSet<u32>) -> Result<u64> {
    let mut bytes = 0;
    for &pid in pids {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
            if handle.is_null() {
                if GetLastError() == ERROR_INVALID_PARAMETER {
                    continue;
                } // Process exited between samples.
                return Err(std::io::Error::last_os_error().into());
            }
            let _process = OwnedHandle::from_raw_handle(handle);
            let mut counters = PROCESS_MEMORY_COUNTERS {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ..Default::default()
            };
            if K32GetProcessMemoryInfo(handle, &mut counters, counters.cb) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            bytes += counters.WorkingSetSize as u64;
        }
    }
    Ok(bytes)
}

struct GpuQuery {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}
impl Drop for GpuQuery {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}
fn pdh(status: u32) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!("Windows GPU counter error {status:#x}").into())
    }
}
impl GpuQuery {
    fn new() -> Result<Self> {
        unsafe {
            let mut query = Self {
                query: null_mut(),
                counter: null_mut(),
            };
            pdh(PdhOpenQueryW(std::ptr::null(), 0, &mut query.query))?;
            pdh(PdhAddEnglishCounterW(
                query.query,
                wide(r"\GPU Process Memory(*)\Dedicated Usage").as_ptr(),
                0,
                &mut query.counter,
            ))?;
            Ok(query)
        }
    }
    fn sample(&self, pids: &BTreeSet<u32>) -> Result<u64> {
        unsafe {
            pdh(PdhCollectQueryData(self.query))?;
            let (mut size, mut count) = (0, 0);
            let status = PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_LARGE,
                &mut size,
                &mut count,
                null_mut(),
            );
            if status != PDH_MORE_DATA {
                pdh(status)?;
                return Ok(0);
            }
            if size > 4 * 1024 * 1024 {
                return Err("GPU counter table exceeds 4 MiB".into());
            }
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            pdh(PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_LARGE,
                &mut size,
                &mut count,
                buffer.as_mut_ptr().cast(),
            ))?;
            if count as usize * std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>() > size as usize {
                return Err("Invalid GPU counter table".into());
            }
            let rows = std::slice::from_raw_parts(
                buffer.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
                count as usize,
            );
            let mut bytes = 0;
            for row in rows {
                let start = row.szName as usize;
                let base = buffer.as_ptr() as usize;
                if start < base || start >= base + size as usize || start % 2 != 0 {
                    return Err("Invalid GPU counter name".into());
                }
                let name =
                    std::slice::from_raw_parts(row.szName, (base + size as usize - start) / 2);
                let end = name
                    .iter()
                    .position(|&unit| unit == 0)
                    .ok_or("Unterminated GPU counter name")?;
                let name = String::from_utf16_lossy(&name[..end]);
                if counter_pid(&name).is_some_and(|pid| pids.contains(&pid)) {
                    if !matches!(
                        row.FmtValue.CStatus,
                        PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
                    ) {
                        pdh(row.FmtValue.CStatus)?;
                    }
                    let value = row.FmtValue.Anonymous.largeValue;
                    bytes += u64::try_from(value).map_err(|_| "Negative GPU memory value")?;
                }
            }
            Ok(bytes)
        }
    }
}
fn counter_pid(name: &str) -> Option<u32> {
    name.strip_prefix("pid_")?.split('_').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Native smoke check requires Windows GPU performance counters"]
    fn native_process_memory_smoke() -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let path = std::env::temp_dir().join(format!("strata-memory-{}.json", std::process::id()));
        std::fs::write(&path, format!(r#"{{"port":{port}}}"#))?;
        let measured = processes(&path);
        std::fs::remove_file(path)?;
        let pids = measured?;
        assert!(pids.contains(&std::process::id()));
        assert!(working_set(&pids)? > 0);
        let started = std::time::Instant::now();
        let gpu = GpuQuery::new()?;
        let bytes = gpu.sample(&pids)?;
        eprintln!(
            "Native GPU query: {:?}; current process dedicated bytes: {bytes}",
            started.elapsed()
        );
        let desktop_pids = process_table()?.into_iter().map(|(pid, _)| pid).collect();
        let started = std::time::Instant::now();
        let allocated = gpu.sample(&desktop_pids)?;
        assert!(allocated > 0, "Expected a nonzero desktop GPU allocation");
        eprintln!(
            "Existing GPU allocations: {allocated} bytes; warm sample: {:?}",
            started.elapsed()
        );
        assert_eq!(counter_pid("pid_1234_luid_0x0_0x123_phys_0"), Some(1234));
        assert_eq!(counter_pid("pid_bad_luid_0"), None);
        drop(listener);
        assert_eq!(super::listener(port)?, None);
        Ok(())
    }
}
