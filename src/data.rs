use rusqlite::{Connection, params};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{FILETIME, SYSTEMTIME},
    Storage::FileSystem::FileTimeToLocalFileTime,
    System::Time::FileTimeToSystemTime,
};

pub fn root() -> &'static Path {
    &crate::config::get().paths.data_dir
}
pub fn python() -> &'static Path {
    &crate::config::get().paths.python
}
pub fn strata_home() -> &'static Path {
    &crate::config::get().paths.strata_dir
}
pub fn model_root() -> &'static Path {
    &crate::config::get().paths.models_dir
}
pub fn profiles_dir() -> &'static Path {
    &crate::config::get().paths.profiles_dir
}
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub fn initialize(paths: &crate::config::Paths) -> Result<()> {
    fs::create_dir_all(&paths.data_dir)?;
    let database = paths.data_dir.join("usage.sqlite3");
    if !database.exists() {
        let mut db = Connection::open(&database)?;
        let transaction = db.transaction()?;
        transaction.execute_batch(include_str!("../assets/schema.sql"))?;
        transaction.commit()?;
    }
    if !paths.pricing_file.exists() {
        if paths.pricing_file != paths.data_dir.join("qwen_pricing.json") {
            return Err("Configured pricing file is missing".into());
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&paths.pricing_file)?;
        file.write_all(include_bytes!("../assets/qwen-pricing.json"))?;
    }
    Ok(())
}

#[derive(Clone, Default, Debug)]
pub struct Counts {
    pub requests: i64,
    pub prompt: i64,
    pub cached: i64,
    pub output: i64,
    pub prompt_ms: f64,
    pub decode_ms: f64,
    pub offered: i64,
    pub accepted: i64,
}
impl Counts {
    fn add(&mut self, other: &Self) {
        self.requests += other.requests;
        self.prompt += other.prompt;
        self.cached += other.cached;
        self.output += other.output;
        self.prompt_ms += other.prompt_ms;
        self.decode_ms += other.decode_ms;
        self.offered += other.offered;
        self.accepted += other.accepted;
    }
    fn from_row(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Self> {
        Ok(Self {
            requests: row.get(offset)?,
            prompt: row.get(offset + 1)?,
            cached: row.get(offset + 2)?,
            output: row.get(offset + 3)?,
            prompt_ms: row.get(offset + 4)?,
            decode_ms: row.get(offset + 5)?,
            offered: row.get(offset + 6)?,
            accepted: row.get(offset + 7)?,
        })
    }
}
#[derive(Clone, Default)]
pub struct Group {
    pub name: String,
    pub counts: Counts,
}
#[derive(Clone, Default)]
pub struct LogGroup {
    pub period: String,
    pub counts: Counts,
    pub source: String,
}
#[derive(Clone, Default)]
pub struct Recent {
    pub at: f64,
    pub output: i64,
    pub fresh: i64,
    pub seconds: f64,
    pub finish: String,
}
#[derive(Clone, Default)]
pub struct Prices {
    pub input: f64,
    pub cached: f64,
    pub output: f64,
    pub usd_per_eur: f64,
    pub date: String,
    pub fx_date: String,
}
impl Prices {
    pub fn costs(&self, c: &Counts) -> [f64; 4] {
        let input = c.prompt as f64 * self.input / 1e6;
        let output = c.output as f64 * self.output / 1e6;
        let cached = ((c.prompt - c.cached) as f64 * self.input + c.cached as f64 * self.cached)
            / 1e6
            + output;
        [input + output, input, output, cached]
    }
    fn read() -> Result<Self> {
        let data: Value =
            serde_json::from_slice(&fs::read(&crate::config::get().paths.pricing_file)?)?;
        let number = |name: &str| -> Result<f64> { Ok(required_str(&data, name)?.parse()?) };
        let rate: f64 = required_str(&data["fx"], "usd_per_eur")?.parse()?;
        if rate <= 0.0 {
            return Err("Invalid EUR/USD rate".into());
        }
        Ok(Self {
            input: number("input_per_million")?,
            cached: number("implicit_cache_input_per_million")?,
            output: number("output_per_million")?,
            usd_per_eur: rate,
            date: required_str(&data, "verified_on")?.into(),
            fx_date: required_str(&data["fx"], "date")?.into(),
        })
    }
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub totals: Counts,
    pub daily: Vec<Group>,
    pub models: Vec<Group>,
    pub logs: Vec<LogGroup>,
    pub recent: Vec<Recent>,
    pub prices: Prices,
    pub online: bool,
    pub model: String,
    pub phase: String,
    pub speed: Option<f64>,
    pub speed_mean: Option<f64>,
    pub prefill_speed: Option<f64>,
    pub last_speed: Option<f64>,
    pub generated: Option<i64>,
    pub slots: u32,
    pub memory: crate::resources::Memory,
    pub updated: f64,
    pub error: String,
}
impl Snapshot {
    pub fn apply_live(&mut self, value: &Value) -> Result<()> {
        let live = &value["live"];
        let rate = |value: &Value| -> Result<Option<f64>> {
            if value.is_null() {
                return Ok(None);
            }
            let rate = value
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or("Invalid token rate")?;
            Ok(Some(rate))
        };
        self.model = value["engine"]["model"]
            .as_str()
            .ok_or("Missing loaded model")?
            .into();
        self.phase = live["phase"]
            .as_str()
            .unwrap_or(live["state"].as_str().unwrap_or(""))
            .into();
        self.speed = if live["state"] == "generating" {
            rate(&live["tok_s"])?
        } else {
            None
        };
        self.speed_mean = if live["state"] == "generating" {
            rate(&live["tok_s_mean"])?
        } else {
            None
        };
        self.prefill_speed = if live["state"] == "reading" {
            rate(&live["prefill_tok_s_mean"])?
        } else {
            None
        };
        self.last_speed = rate(&value["requests"][0]["decode_tok_s"])?;
        self.generated = live["generated"].as_i64();
        self.slots = u32::try_from(live["parallel"].as_u64().unwrap_or(1))?.max(1);
        self.online = true;
        self.error.clear();
        self.updated = now();
        Ok(())
    }
    pub fn rate_label(&self, now: f64) -> String {
        if !self.online || !self.error.is_empty() || now - self.updated > 3.0 {
            return "Live — · Mean — t/s".into();
        }
        if self.phase == "idle" {
            return self
                .last_speed
                .map(|n| format!("Last request {n:.1} t/s"))
                .unwrap_or_else(|| "No completed request".into());
        }
        let value = |n: Option<f64>| n.map(|n| format!("{n:.1}")).unwrap_or_else(|| "—".into());
        format!(
            "Live {} · Mean {} t/s",
            value(self.speed),
            value(self.speed_mean)
        )
    }
    pub fn prefill_label(&self, now: f64) -> String {
        let speed = self
            .prefill_speed
            .filter(|_| self.online && self.error.is_empty() && now - self.updated <= 3.0);
        speed
            .map(|speed| format!("Prefill {speed:.1} t/s"))
            .unwrap_or_else(|| "Prefill N/A t/s".into())
    }
}

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Invalid system clock")
        .as_secs_f64()
}
pub fn local_date(timestamp: f64, with_time: bool) -> String {
    let ticks = ((timestamp + 11644473600.0) * 10000000.0) as u64;
    let utc = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut local: FILETIME = unsafe { std::mem::zeroed() };
    let mut date: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        if FileTimeToLocalFileTime(&utc, &mut local) == 0
            || FileTimeToSystemTime(&local, &mut date) == 0
        {
            return "Invalid date".into();
        }
    }
    if with_time {
        format!(
            "{:02}/{:02} {:02}:{:02}:{:02}",
            date.wDay, date.wMonth, date.wHour, date.wMinute, date.wSecond
        )
    } else {
        format!("{:04}-{:02}-{:02}", date.wYear, date.wMonth, date.wDay)
    }
}
fn required_str<'a>(data: &'a Value, key: &str) -> Result<&'a str> {
    data.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing string field: {key}").into())
}
fn integer(data: &Value, key: &str) -> Result<i64> {
    data.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("Missing integer field: {key}").into())
}
fn number(data: &Value, key: &str) -> Result<f64> {
    data.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("Missing numeric field: {key}").into())
}
pub fn connect() -> Result<Connection> {
    let db = Connection::open_with_flags(
        root().join("usage.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.set_prepared_statement_cache_capacity(4);
    db.execute_batch("PRAGMA cache_size=-256; PRAGMA temp_store=FILE;")?;
    Ok(db)
}
pub fn fetch_metrics(profile: &Path, all_requests: bool) -> Result<Value> {
    let bytes = fs::read(profile)?;
    let cfg: Value = serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))?;
    let key = match &cfg["api_key"] {
        Value::String(s) => Some(s.as_str()),
        Value::Array(a) => a
            .first()
            .and_then(Value::as_str)
            .map(Some)
            .ok_or("Empty API key list")?,
        Value::Null => None,
        _ => return Err("Missing Strata API key".into()),
    };
    if key.is_some_and(|key| key.contains(['\r', '\n'])) {
        return Err("Invalid API key format".into());
    }
    let port = u16::try_from(cfg["port"].as_u64().ok_or("Missing server port")?)?;
    if port == 0 {
        return Err("Invalid server port".into());
    }
    let address: SocketAddr = format!("127.0.0.1:{port}").parse()?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(4)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "GET {} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n{}Connection: close\r\n\r\n",
        if all_requests {
            "/metrics?requests=all"
        } else {
            "/metrics"
        },
        key.map(|key| format!("Authorization: Bearer {key}\r\n"))
            .unwrap_or_default()
    )?;
    let mut bytes = Vec::with_capacity(65536);
    stream.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Strata metrics response exceeds 8 MiB".into());
    }
    let split = bytes
        .windows(4)
        .position(|s| s == b"\r\n\r\n")
        .ok_or("Invalid HTTP response")?;
    let header = std::str::from_utf8(&bytes[..split])?;
    if header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        != Some("200")
    {
        return Err(format!(
            "Strata: {}",
            header.lines().next().unwrap_or("invalid status")
        )
        .into());
    }
    Ok(serde_json::from_slice(&bytes[split + 4..])?)
}
pub fn store(db: &mut Connection, data: &Value) -> Result<()> {
    let totals = &data["totals"];
    let started = number(totals, "since")?;
    let run_id = format!("{started:.6}");
    let model = required_str(&data["engine"], "model")?;
    let c = Counts {
        requests: integer(totals, "requests")?,
        prompt: integer(totals, "prompt_tokens")?,
        cached: integer(totals, "reused")?,
        output: integer(totals, "output_tokens")?,
        prompt_ms: number(totals, "prompt_ms")?,
        decode_ms: number(totals, "decode_ms")?,
        offered: integer(totals, "drafts_offered")?,
        accepted: integer(totals, "drafts_accepted")?,
    };
    if c.requests < 0
        || c.prompt < 0
        || c.cached < 0
        || c.cached > c.prompt
        || c.output < 0
        || c.prompt_ms < 0.0
        || c.decode_ms < 0.0
        || c.offered < 0
        || c.accepted < 0
        || c.accepted > c.offered
    {
        return Err("Invalid Strata counters".into());
    }
    let tx = db.transaction()?;
    let previous=tx.query_row("SELECT requests,prompt_tokens,reused,output_tokens,prompt_ms,decode_ms,drafts_offered,drafts_accepted FROM runs WHERE id=?",[&run_id],|row|Counts::from_row(row,0));
    match previous {
        Ok(old)
            if c.requests < old.requests
                || c.prompt < old.prompt
                || c.cached < old.cached
                || c.output < old.output
                || c.prompt_ms < old.prompt_ms
                || c.decode_ms < old.decode_ms
                || c.offered < old.offered
                || c.accepted < old.accepted =>
        {
            return Err("Strata counters decreased without a new session".into());
        }
        Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => (),
        Err(error) => return Err(error.into()),
    }
    let observed = now();
    tx.execute("INSERT INTO runs VALUES (?1,?2,?3,?4,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(id) DO UPDATE SET model=excluded.model,last_seen=excluded.last_seen,requests=excluded.requests,prompt_tokens=excluded.prompt_tokens,reused=excluded.reused,output_tokens=excluded.output_tokens,prompt_ms=excluded.prompt_ms,decode_ms=excluded.decode_ms,drafts_offered=excluded.drafts_offered,drafts_accepted=excluded.drafts_accepted",
        params![run_id,model,started,observed,c.requests,c.prompt,c.cached,c.output,c.prompt_ms,c.decode_ms,c.offered,c.accepted])?;
    let mut insert=tx.prepare_cached("INSERT INTO requests SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12 WHERE NOT EXISTS (SELECT 1 FROM requests WHERE run_id=?2 AND started=?3 AND duration_s=?6 AND prompt_tokens=?7 AND reused=?8 AND output_tokens=?9 AND finish=?12)")?;
    for r in data["requests"]
        .as_array()
        .ok_or("Missing request history")?
    {
        let at = number(r, "time")?;
        let duration = number(r, "duration_s")?;
        let prompt = integer(r, "prompt_tokens")?;
        let output = integer(r, "output_tokens")?;
        let cached = r["reused"].as_i64().unwrap_or(0);
        let finish = required_str(r, "finish")?;
        let id = format!(
            "native:{run_id}:{}:{}:{prompt}:{cached}:{output}:{finish}",
            at.to_bits(),
            duration.to_bits()
        );
        insert.execute(params![
            id,
            run_id,
            at,
            local_date(at, false),
            model,
            duration,
            prompt,
            cached,
            output,
            r["prompt_ms"].as_f64().unwrap_or(0.0),
            r["decode_ms"].as_f64().unwrap_or(0.0),
            finish
        ])?;
    }
    drop(insert);
    tx.commit()?;
    Ok(())
}

pub fn load(db: &Connection) -> Result<Snapshot> {
    const SUM: &str = "COALESCE(SUM(requests),0),COALESCE(SUM(prompt_tokens),0),COALESCE(SUM(reused),0),COALESCE(SUM(output_tokens),0),COALESCE(SUM(prompt_ms),0),COALESCE(SUM(decode_ms),0),COALESCE(SUM(drafts_offered),0),COALESCE(SUM(drafts_accepted),0)";
    let union = "SELECT requests,prompt_tokens,reused,output_tokens,prompt_ms,decode_ms,drafts_offered,drafts_accepted,model FROM runs UNION ALL SELECT requests,prompt_tokens,reused,output_tokens,prompt_ms,decode_ms,drafts_offered,drafts_accepted,model FROM log_imports";
    let totals = db.query_row(&format!("SELECT {SUM} FROM ({union})"), [], |row| {
        Counts::from_row(row, 0)
    })?;
    let mut model_query = db.prepare(&format!(
        "SELECT model,{SUM} FROM ({union}) GROUP BY model ORDER BY SUM(output_tokens) DESC"
    ))?;
    let models = model_query
        .query_map([], |row| {
            Ok(Group {
                name: row.get(0)?,
                counts: Counts::from_row(row, 1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut day_query=db.prepare("SELECT day,SUM(n),SUM(p),SUM(c),SUM(o),SUM(pm),SUM(dm),0,0 FROM (SELECT day,1 n,prompt_tokens p,reused c,output_tokens o,prompt_ms pm,decode_ms dm FROM requests UNION ALL SELECT day,requests,prompt_tokens,reused,output_tokens,prompt_ms,decode_ms FROM log_imports WHERE day IS NOT NULL) GROUP BY day ORDER BY day")?;
    let daily = day_query
        .query_map([], |row| {
            Ok(Group {
                name: row.get(0)?,
                counts: Counts::from_row(row, 1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut logs_query=db.prepare("SELECT date_from,date_to,requests,prompt_tokens,reused,output_tokens,prompt_ms,decode_ms,drafts_offered,drafts_accepted,source_path FROM log_imports ORDER BY date_to,date_from")?;
    let logs = logs_query
        .query_map([], |row| {
            let from: Option<String> = row.get(0)?;
            let to: String = row.get(1)?;
            let period = match from {
                Some(start) if start == to => start,
                Some(start) => format!("{start} / {to}"),
                None => format!("Through {to}"),
            };
            let path: String = row.get(10)?;
            Ok(LogGroup {
                period,
                counts: Counts::from_row(row, 2)?,
                source: Path::new(&path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut recent_query=db.prepare("SELECT started,output_tokens,prompt_tokens-reused,duration_s,finish FROM requests ORDER BY started DESC LIMIT 100")?;
    let recent = recent_query
        .query_map([], |row| {
            Ok(Recent {
                at: row.get(0)?,
                output: row.get(1)?,
                fresh: row.get(2)?,
                seconds: row.get(3)?,
                finish: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let model = db
        .query_row(
            "SELECT model FROM runs ORDER BY last_seen DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .or_else(|error| {
            if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                Ok(String::new())
            } else {
                Err(error)
            }
        })?;
    Ok(Snapshot {
        totals,
        daily,
        models,
        logs,
        recent,
        prices: Prices::read()?,
        model,
        ..Default::default()
    })
}

fn csv_cell(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}
pub fn export(snapshot: &Snapshot) -> Result<PathBuf> {
    let directory = root().join("exports");
    fs::create_dir_all(&directory)?;
    let stamp = now() as u64;
    let mut daily = String::from(
        "\u{feff}day,requests,prompt_tokens,reused,output_tokens,prompt_ms,decode_ms,qwen_no_cache_usd,qwen_no_cache_eur,qwen_same_cache_usd,qwen_same_cache_eur,pricing_date,fx_date,usd_per_eur\n",
    );
    for g in &snapshot.daily {
        let c = &g.counts;
        let costs = snapshot.prices.costs(c);
        let fx = snapshot.prices.usd_per_eur;
        daily += &format!(
            "{},{},{},{},{},{},{},{:.8},{:.8},{:.8},{:.8},{},{},{}\n",
            g.name,
            c.requests,
            c.prompt,
            c.cached,
            c.output,
            c.prompt_ms,
            c.decode_ms,
            costs[0],
            costs[0] / fx,
            costs[3],
            costs[3] / fx,
            snapshot.prices.date,
            snapshot.prices.fx_date,
            fx
        );
    }
    fs::write(directory.join(format!("strata-giorni-{stamp}.csv")), daily)?;
    let mut logs = String::from(
        "\u{feff}period,requests,prompt_tokens,reused,output_tokens,prompt_ms,decode_ms,source,qwen_no_cache_usd,qwen_no_cache_eur,qwen_same_cache_usd,qwen_same_cache_eur\n",
    );
    for g in &snapshot.logs {
        let c = &g.counts;
        let costs = snapshot.prices.costs(c);
        let fx = snapshot.prices.usd_per_eur;
        logs += &format!(
            "{},{},{},{},{},{},{},{},{:.8},{:.8},{:.8},{:.8}\n",
            csv_cell(&g.period),
            c.requests,
            c.prompt,
            c.cached,
            c.output,
            c.prompt_ms,
            c.decode_ms,
            csv_cell(&g.source),
            costs[0],
            costs[0] / fx,
            costs[3],
            costs[3] / fx
        );
    }
    fs::write(directory.join(format!("strata-storico-{stamp}.csv")), logs)?;
    Ok(directory)
}

pub fn check() -> Result<()> {
    let db = connect()?;
    let snapshot = load(&db)?;
    let mut models = Counts::default();
    for model in &snapshot.models {
        models.add(&model.counts)
    }
    if models.output != snapshot.totals.output || models.requests != snapshot.totals.requests {
        return Err("Model totals do not reconcile".into());
    }
    let cost = snapshot.prices.costs(&snapshot.totals);
    let output = serde_json::json!({"requests":snapshot.totals.requests,"output_tokens":snapshot.totals.output,"no_cache_usd":cost[0],"same_cache_usd":cost[3],"no_cache_eur":cost[0]/snapshot.prices.usd_per_eur,"days":snapshot.daily.len(),"log_segments":snapshot.logs.len()});
    fs::write(
        root().join("native-check.json"),
        serde_json::to_vec_pretty(&output)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_rate_distinguishes_live_mean_last_and_stale_samples() -> Result<()> {
        let mut snapshot = Snapshot::default();
        let mut metrics = serde_json::json!({"engine":{"model":"fixture"},
            "live":{"state":"generating","phase":"thinking","tok_s":61.2,"tok_s_mean":53.1,"generated":500},
            "requests":[{"decode_tok_s":51.7}]});
        snapshot.apply_live(&metrics)?;
        assert_eq!(snapshot.phase, "thinking");
        assert_eq!(
            snapshot.rate_label(snapshot.updated),
            "Live 61.2 · Mean 53.1 t/s"
        );
        assert_eq!(
            snapshot.rate_label(snapshot.updated + 3.1),
            "Live — · Mean — t/s"
        );
        metrics["live"]["state"] = "idle".into();
        metrics["live"]["phase"] = Value::Null;
        snapshot.apply_live(&metrics)?;
        assert_eq!(snapshot.speed, None);
        assert_eq!(snapshot.speed_mean, None);
        assert_eq!(
            snapshot.rate_label(snapshot.updated),
            "Last request 51.7 t/s"
        );
        metrics["live"]["state"] = "reading".into();
        snapshot.apply_live(&metrics)?;
        assert_eq!(snapshot.rate_label(snapshot.updated), "Live — · Mean — t/s");
        metrics["live"]["state"] = "generating".into();
        metrics["live"]["tok_s"] = (-1).into();
        assert!(snapshot.apply_live(&metrics).is_err());
        Ok(())
    }
    #[test]
    fn prefill_rate_uses_engine_mean_only_while_reading_and_fresh() -> Result<()> {
        let mut snapshot = Snapshot::default();
        let mut metrics = serde_json::json!({"engine":{"model":"fixture"},
            "live":{"state":"reading","phase":"reading the prompt","prefill_tok_s_mean":1245.6}});
        snapshot.apply_live(&metrics)?;
        assert_eq!(
            snapshot.prefill_label(snapshot.updated),
            "Prefill 1245.6 t/s"
        );
        assert_eq!(
            snapshot.prefill_label(snapshot.updated + 3.1),
            "Prefill N/A t/s"
        );
        assert_eq!(snapshot.speed, None);
        metrics["live"]["state"] = "generating".into();
        snapshot.apply_live(&metrics)?;
        assert_eq!(snapshot.prefill_speed, None);
        assert_eq!(snapshot.prefill_label(snapshot.updated), "Prefill N/A t/s");
        metrics["live"]["state"] = "reading".into();
        metrics["live"]["prefill_tok_s_mean"] = Value::Null;
        snapshot.apply_live(&metrics)?;
        assert_eq!(snapshot.prefill_label(snapshot.updated), "Prefill N/A t/s");
        metrics["live"]["prefill_tok_s_mean"] = (-1).into();
        assert!(snapshot.apply_live(&metrics).is_err());
        Ok(())
    }
    #[test]
    fn new_data_folder_initializes_without_overwriting_existing_data() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("strata-hub-data-test-{}", std::process::id()));
        let paths = crate::config::Paths {
            strata_dir: directory.join("strata"),
            python: directory.join("python.exe"),
            models_dir: directory.join("models"),
            profiles_dir: directory.join("profiles"),
            data_dir: directory.clone(),
            pricing_file: directory.join("qwen_pricing.json"),
        };
        initialize(&paths)?;
        let db = Connection::open(directory.join("usage.sqlite3"))?;
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM runs", [], |row| row.get::<_, i64>(0))?,
            0
        );
        db.execute(
            "INSERT INTO runs VALUES ('fixture','model',0,0,0,1,2,0,3,4,5,0,0)",
            [],
        )?;
        fs::write(&paths.pricing_file, b"fixture pricing")?;
        initialize(&paths)?;
        assert_eq!(
            db.query_row(
                "SELECT output_tokens FROM runs WHERE id='fixture'",
                [],
                |row| row.get::<_, i64>(0)
            )?,
            3
        );
        assert_eq!(fs::read(&paths.pricing_file)?, b"fixture pricing");
        drop(db);
        fs::remove_file(directory.join("usage.sqlite3"))?;
        fs::remove_file(paths.pricing_file)?;
        fs::remove_dir(directory)?;
        Ok(())
    }
    #[test]
    fn metrics_use_the_configured_server_port() -> Result<()> {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let profile =
            std::env::temp_dir().join(format!("strata-port-test-{}.json", std::process::id()));
        fs::write(
            &profile,
            serde_json::to_vec(&serde_json::json!({"port":port,"api_key":"fixture-key"}))?,
        )?;
        let server = std::thread::spawn(move || -> Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut bytes = Vec::new();
            let mut byte = [0; 1];
            while !bytes.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte)?;
                bytes.push(byte[0]);
                assert!(bytes.len() <= 4096);
            }
            let request = std::str::from_utf8(&bytes)?;
            assert!(request.contains(&format!("Host: 127.0.0.1:{port}")));
            assert!(request.starts_with("GET /metrics HTTP/1.0"));
            stream.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 11\r\n\r\n{\"ok\":true}")?;
            Ok(())
        });
        assert_eq!(fetch_metrics(&profile, false)?["ok"], true);
        server.join().expect("Metrics fixture server panicked")?;
        fs::remove_file(profile)?;
        Ok(())
    }
    #[test]
    fn metrics_preserve_python_history_and_deduplicate_snapshots() -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE runs(id TEXT PRIMARY KEY,model TEXT NOT NULL,started REAL NOT NULL,first_seen REAL NOT NULL,last_seen REAL NOT NULL,requests INTEGER NOT NULL,prompt_tokens INTEGER NOT NULL,reused INTEGER NOT NULL,output_tokens INTEGER NOT NULL,prompt_ms REAL NOT NULL,decode_ms REAL NOT NULL,drafts_offered INTEGER NOT NULL,drafts_accepted INTEGER NOT NULL);
            CREATE TABLE requests(id TEXT PRIMARY KEY,run_id TEXT NOT NULL,started REAL NOT NULL,day TEXT NOT NULL,model TEXT NOT NULL,duration_s REAL NOT NULL,prompt_tokens INTEGER NOT NULL,reused INTEGER NOT NULL,output_tokens INTEGER NOT NULL,prompt_ms REAL NOT NULL,decode_ms REAL NOT NULL,finish TEXT NOT NULL);
            INSERT INTO requests VALUES ('python-sha256-id','100.000000',110,'1970-01-01','test',1,10,2,3,1,2,'stop');")?;
        let mut metrics = serde_json::json!({"totals":{"since":100.0,"requests":1,"prompt_tokens":10,"reused":2,"output_tokens":3,"prompt_ms":1.0,"decode_ms":2.0,"drafts_offered":2,"drafts_accepted":1},"engine":{"model":"test"},"requests":[{"time":110.0,"duration_s":1.0,"prompt_tokens":10,"prompt_total":10,"reused":2,"output_tokens":3,"prompt_ms":1.0,"decode_ms":2.0,"finish":"stop"}]});
        store(&mut db, &metrics)?;
        store(&mut db, &metrics)?;
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM requests", [], |row| row
                .get::<_, i64>(0))?,
            1
        );
        assert_eq!(
            db.query_row("SELECT SUM(output_tokens) FROM runs", [], |row| row
                .get::<_, i64>(0))?,
            3
        );
        metrics["totals"]["output_tokens"] = 2.into();
        assert!(store(&mut db, &metrics).is_err());
        assert_eq!(
            db.query_row("SELECT SUM(output_tokens) FROM runs", [], |row| row
                .get::<_, i64>(0))?,
            3
        );
        metrics["totals"]["since"] = 200.into();
        metrics["totals"]["output_tokens"] = 3.into();
        metrics["requests"][0]["time"] = 210.into();
        store(&mut db, &metrics)?;
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM requests", [], |row| row
                .get::<_, i64>(0))?,
            2
        );
        assert_eq!(
            db.query_row("SELECT SUM(output_tokens) FROM runs", [], |row| row
                .get::<_, i64>(0))?,
            6
        );
        Ok(())
    }
}
