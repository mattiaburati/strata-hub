//! Process-scoped memory readings, sampled by the existing metrics worker.
use crate::{data::Result, wide};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    os::windows::io::{FromRawHandle, OwnedHandle},
    path::Path,
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::*,
    NetworkManagement::IpHelper::*,
    System::{Diagnostics::ToolHelp::*, Performance::*},
};

#[derive(Clone, Default)]
pub struct Memory {
    pub ram: Option<u64>,
    pub vram: Option<u64>,
    pub error: String,
}
#[derive(Default)]
pub struct Monitor {
    ram: Option<std::result::Result<RamQuery, String>>,
    gpu: Option<std::result::Result<GpuQuery, String>>,
}
impl Monitor {
    pub fn sample(&mut self, profile: Option<&Path>) -> Memory {
        let pids = match profile.map(processes).transpose() {
            Ok(Some(pids)) if !pids.is_empty() => pids,
            Ok(_) => {
                self.ram = None;
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
        let ram = self
            .ram
            .get_or_insert_with(|| RamQuery::new().map_err(|e| e.to_string()));
        let measured = match ram {
            Ok(query) => query.sample(&pids).map_err(|e| e.to_string()),
            Err(error) => Err(error.clone()),
        };
        match measured {
            Ok(bytes) => result.ram = bytes,
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
    if let Some((pid, _, record)) = &owned {
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

// Both readings use Windows performance counters, without opening model processes.
struct CounterQuery {
    handle: PDH_HQUERY,
}
impl Drop for CounterQuery {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.handle);
        }
    }
}
fn pdh(status: u32) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!("Windows performance counter error {status:#x}").into())
    }
}
struct CounterValue {
    name: String,
    status: u32,
    value: i64,
}
impl CounterValue {
    fn unsigned(&self) -> Result<u64> {
        if !matches!(self.status, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA) {
            return Err(
                format!("Counter {} has invalid data: {:#x}", self.name, self.status).into(),
            );
        }
        Ok(u64::try_from(self.value).map_err(|_| "Negative performance counter value")?)
    }
}
impl CounterQuery {
    fn new() -> Result<Self> {
        let mut query = Self { handle: null_mut() };
        unsafe {
            pdh(PdhOpenQueryW(std::ptr::null(), 0, &mut query.handle))?;
        }
        Ok(query)
    }
    fn add(&self, path: &str) -> Result<PDH_HCOUNTER> {
        let mut counter = null_mut();
        unsafe {
            pdh(PdhAddEnglishCounterW(
                self.handle,
                wide(path).as_ptr(),
                0,
                &mut counter,
            ))?;
        }
        Ok(counter)
    }
    fn collect(&self) -> Result<()> {
        unsafe { pdh(PdhCollectQueryData(self.handle)) }
    }
    fn values(&self, counter: PDH_HCOUNTER) -> Result<Vec<CounterValue>> {
        unsafe {
            let (mut size, mut count) = (0, 0);
            // PDH_FMT_NOSCALE from the Windows SDK Pdh.h; absent in windows-sys 0.61.
            let format = PDH_FMT_LARGE | 0x00001000;
            let status =
                PdhGetFormattedCounterArrayW(counter, format, &mut size, &mut count, null_mut());
            if status != PDH_MORE_DATA {
                pdh(status)?;
                if count != 0 {
                    return Err("Missing performance counter buffer".into());
                }
                return Ok(Vec::new());
            }
            if size == 0 || size > 4 * 1024 * 1024 {
                return Err("Invalid performance counter table size".into());
            }
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            pdh(PdhGetFormattedCounterArrayW(
                counter,
                format,
                &mut size,
                &mut count,
                buffer.as_mut_ptr().cast(),
            ))?;
            if size as usize > buffer.len() * 8
                || count as usize * std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>()
                    > size as usize
            {
                return Err("Invalid performance counter table".into());
            }
            let rows = std::slice::from_raw_parts(
                buffer.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
                count as usize,
            );
            let mut values = Vec::with_capacity(rows.len());
            for row in rows {
                let start = row.szName as usize;
                let base = buffer.as_ptr() as usize;
                if start < base || start >= base + size as usize || start % 2 != 0 {
                    return Err("Invalid performance counter name".into());
                }
                let name =
                    std::slice::from_raw_parts(row.szName, (base + size as usize - start) / 2);
                let end = name
                    .iter()
                    .position(|&unit| unit == 0)
                    .ok_or("Unterminated performance counter name")?;
                values.push(CounterValue {
                    name: String::from_utf16_lossy(&name[..end]),
                    status: row.FmtValue.CStatus,
                    value: row.FmtValue.Anonymous.largeValue,
                });
            }
            Ok(values)
        }
    }
}
struct RamQuery {
    query: CounterQuery,
    ids: PDH_HCOUNTER,
    working_set: PDH_HCOUNTER,
}
impl RamQuery {
    fn new() -> Result<Self> {
        let query = CounterQuery::new()?;
        // Join instance names to PIDs within this collection, never across samples.
        let ids = query.add(r"\Process(*)\ID Process")?;
        let working_set = query.add(r"\Process(*)\Working Set")?;
        Ok(Self {
            query,
            ids,
            working_set,
        })
    }
    fn sample(&self, pids: &BTreeSet<u32>) -> Result<Option<u64>> {
        self.query.collect()?;
        let ids = self.query.values(self.ids)?;
        let working_sets = self.query.values(self.working_set)?;
        sum_working_sets(pids, ids, working_sets)
    }
}
fn sum_working_sets(
    pids: &BTreeSet<u32>,
    ids: Vec<CounterValue>,
    working_sets: Vec<CounterValue>,
) -> Result<Option<u64>> {
    if ids.len() != working_sets.len() {
        return Err("Process counter instance counts do not match".into());
    }
    // PDH arrays omit #n suffixes for duplicate names. Both counters come from
    // the same Process object in one collection: match each name's occurrence
    // in that snapshot, then select by its ID Process, never by executable name.
    let mut memory: BTreeMap<String, VecDeque<CounterValue>> = BTreeMap::new();
    for row in working_sets {
        memory.entry(row.name.clone()).or_default().push_back(row);
    }
    let mut matched = BTreeSet::new();
    let mut bytes = 0u64;
    for row in ids {
        let working_set = memory
            .get_mut(&row.name)
            .and_then(VecDeque::pop_front)
            .ok_or_else(|| format!("Missing working set instance {}", row.name))?;
        let pid = u32::try_from(row.unsigned()?)?;
        if !pids.contains(&pid) {
            continue;
        }
        if !matched.insert(pid) {
            return Err(format!("Duplicate process PID {pid}").into());
        }
        bytes = bytes
            .checked_add(working_set.unsigned()?)
            .ok_or("RAM counter overflow")?;
    }
    // No matching instances means the sampled process tree has already exited.
    Ok((!matched.is_empty()).then_some(bytes))
}
struct GpuQuery {
    query: CounterQuery,
    counter: PDH_HCOUNTER,
}
impl GpuQuery {
    fn new() -> Result<Self> {
        let query = CounterQuery::new()?;
        let counter = query.add(r"\GPU Process Memory(*)\Dedicated Usage")?;
        Ok(Self { query, counter })
    }
    fn sample(&self, pids: &BTreeSet<u32>) -> Result<u64> {
        self.query.collect()?;
        let mut bytes = 0u64;
        for row in self.query.values(self.counter)? {
            if counter_pid(&row.name).is_some_and(|pid| pids.contains(&pid)) {
                bytes = bytes
                    .checked_add(row.unsigned()?)
                    .ok_or("GPU counter overflow")?;
            }
        }
        Ok(bytes)
    }
}
fn counter_pid(name: &str) -> Option<u32> {
    name.strip_prefix("pid_")?.split('_').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_counters_match_instances_and_reject_invalid_selected_values() -> Result<()> {
        let row = |name: &str, value| CounterValue {
            name: name.into(),
            value,
            status: PDH_CSTATUS_VALID_DATA,
        };
        let pids = BTreeSet::from([10, 20]);
        assert_eq!(
            sum_working_sets(
                &BTreeSet::from([20]),
                vec![row("python", 10), row("python", 20)],
                vec![row("python", 100), row("python", 200)]
            )?,
            Some(200)
        );
        assert_eq!(
            sum_working_sets(
                &pids,
                vec![
                    row("python_10", 10),
                    row("python_20", 20),
                    row("other_30", 30)
                ],
                vec![
                    row("other_30", 9000),
                    row("python_20", 200),
                    row("python_10", 100)
                ]
            )?,
            Some(300)
        );
        assert_eq!(
            sum_working_sets(
                &pids,
                vec![row("other_30", 30)],
                vec![row("other_30", 9000)]
            )?,
            None
        );
        assert!(sum_working_sets(&pids, vec![row("python_10", 10)], vec![]).is_err());
        let mut invalid = row("python_10", 100);
        invalid.status = PDH_CSTATUS_INVALID_DATA;
        assert!(sum_working_sets(&pids, vec![row("python_10", 10)], vec![invalid]).is_err());
        Ok(())
    }

    #[test]
    fn working_set_handles_running_and_terminated_processes() -> Result<()> {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        let ram = RamQuery::new()?;
        assert!(
            ram.sample(&BTreeSet::from([std::process::id()]))?
                .is_some_and(|n| n > 0)
        );
        let mut child = std::process::Command::new("ping.exe")
            .args(["-t", "127.0.0.1"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .spawn()?;
        let pid = child.id();
        let running = ram.sample(&BTreeSet::from([pid]));
        child.kill()?;
        child.wait()?;
        running?;
        // Keep the Child handle open, as happens while Stop tears down a tree.
        assert_eq!(ram.sample(&BTreeSet::from([pid]))?, None);
        Ok(())
    }

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
        assert!(RamQuery::new()?.sample(&pids)?.is_some_and(|n| n > 0));
        if let Some(pid) = std::env::var_os("STRATA_TEST_MEMORY_PID") {
            let pid = pid.to_str().ok_or("Invalid test PID")?.parse()?;
            let query = RamQuery::new()?;
            let started = std::time::Instant::now();
            let measured = query.sample(&BTreeSet::from([pid]))?;
            assert!(measured.is_some_and(|n| n > 0), "Missing live Strata RAM");
            eprintln!(
                "Live Strata PID {pid}: {measured:?} RAM bytes; {:?}",
                started.elapsed()
            );
        }
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
