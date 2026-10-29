use crate::data::{self, Result};
use crate::engine_options::{self, Engine};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub choices: &'static [&'static str],
}

pub const PERFORMANCE_FIELD_START: usize = 13;
pub const EXPERIMENTAL_PROFILE: &str = "strata-swift-iq3_s-experimental.json";
pub const ENVIRONMENT_SCOPES: &[&str] = &[
    "Standard engine",
    "Experimental engine",
    "Standard server",
    "Experimental server",
];
pub type Environment = BTreeMap<String, String>;

fn managed_environment(name: &str) -> bool {
    ["STRATA_PIN_BY_NEED", "STRATA_KV_GROW"]
        .iter()
        .any(|key| name.eq_ignore_ascii_case(key))
}

fn validate_environment(environment: &Environment) -> Result<()> {
    let mut names = std::collections::BTreeSet::new();
    for (name, value) in environment {
        if name.is_empty()
            || name.trim() != name
            || name.contains(['=', '\0', '\r', '\n'])
            || value.contains(['\0', '\r', '\n'])
        {
            return Err("Environment variables require a nonempty name without = and a single-line value without NUL characters".into());
        }
        if !names.insert(name.to_uppercase()) {
            return Err(format!(
                "Duplicate environment variable: {name} (Windows names are case insensitive)"
            )
            .into());
        }
    }
    Ok(())
}

pub fn parse_environment(text: &str) -> Result<Environment> {
    let mut environment = Environment::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("Environment line {}: expected NAME=value", index + 1))?;
        if environment
            .insert(name.to_owned(), value.to_owned())
            .is_some()
        {
            return Err(format!("Duplicate environment variable on line {}", index + 1).into());
        }
    }
    validate_environment(&environment)?;
    Ok(environment)
}

pub const FIELDS: &[Field] = &[
    Field {
        key: "--max-context",
        label: "Context (tokens)",
        choices: &[],
    },
    Field {
        key: "--vram-reserve-mib",
        label: "VRAM reserve (MiB)",
        choices: &[],
    },
    Field {
        key: "--prefill",
        label: "Prefill",
        choices: &[
            "auto",
            "1024",
            "2048",
            "4096",
            "8192",
            "auto:16384",
            "auto:32768",
        ],
    },
    Field {
        key: "--expert-cache",
        label: "Expert cache",
        choices: &["auto", "0"],
    },
    Field {
        key: "--kv",
        label: "KV cache",
        choices: &["int8", "fp16", "q4_0", "k8v4"],
    },
    Field {
        key: "--spec",
        label: "Speculation window",
        choices: &[],
    },
    Field {
        key: "--spec-min-p",
        label: "MTP minimum probability",
        choices: &[],
    },
    Field {
        key: "temperature",
        label: "Temperature",
        choices: &[],
    },
    Field {
        key: "top_p",
        label: "Top P",
        choices: &[],
    },
    Field {
        key: "top_k",
        label: "Top K",
        choices: &[],
    },
    Field {
        key: "min_p",
        label: "Min P",
        choices: &[],
    },
    Field {
        key: "repetition_penalty",
        label: "Repetition penalty",
        choices: &[],
    },
    Field {
        key: "presence_penalty",
        label: "Presence penalty",
        choices: &[],
    },
    Field {
        key: "--pcie-frac",
        label: "PCIe share (0–1)",
        choices: &[],
    },
    Field {
        key: "--pool-workers",
        label: "CPU workers",
        choices: &[],
    },
    Field {
        key: "--adapt-every",
        label: "Adapt every (windows)",
        choices: &[],
    },
    Field {
        key: "--adapt-swaps",
        label: "Adapt swaps per round",
        choices: &[],
    },
    Field {
        key: "--adapt-decay",
        label: "Adapt decay (0–1)",
        choices: &[],
    },
    Field {
        key: "--prefill-pipe",
        label: "Prefill pipeline (multi-GPU)",
        choices: &[],
    },
];
const FLAGS: &[&str] = &["--resident-experts", "--vision"];
const CALIBRATION_FLAGS: &[&str] = &[
    "--pcie-frac",
    "--spec-min-p",
    "--pool-workers",
    "--adapt-every",
    "--adapt-swaps",
    "--adapt-decay",
    "--prefill-pipe",
];

#[derive(Clone)]
pub struct Settings {
    pub engine: Engine,
    pub gpu: Option<Vec<u32>>,
    pub gpu_keep_order: bool,
    pub environment: [Option<Environment>; 4],
    pub advanced: BTreeMap<String, String>,
    pub values: Vec<String>,
    pub parallel: u32,
    pub port: u16,
    pub adapt_async: Option<bool>,
    pub pin_by_need: bool,
    pub resident: bool,
    pub kv_grow: bool,
    pub kv_stream: bool,
    pub vision: bool,
    pub extra_args: String,
}

pub fn parse_gpu_selection(text: &str) -> Result<Vec<u32>> {
    let devices = text
        .split(',')
        .map(|part| {
            let part = part.trim();
            if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
                return Err("Enter GPU indices separated by commas, for example 0,1".into());
            }
            part.parse::<u32>().map_err(Into::into)
        })
        .collect::<Result<Vec<_>>>()?;
    if devices
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != devices.len()
    {
        return Err("Each GPU index must appear only once".into());
    }
    Ok(devices)
}

fn read_json(path: &Path) -> Result<Value> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(
        bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes),
    )?)
}
fn args(config: &Value) -> Result<Vec<String>> {
    config["args"]
        .as_array()
        .ok_or("Missing engine arguments")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "Invalid engine argument".into())
        })
        .collect()
}
fn argument<'a>(args: &'a [String], name: &str) -> Result<&'a str> {
    let index = args
        .iter()
        .position(|s| s == name)
        .ok_or_else(|| format!("Missing engine option {name}"))?;
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("Missing value for {name}").into())
}
fn managed(name: &str) -> bool {
    engine_options::managed(name)
        || FIELDS
            .iter()
            .any(|f| f.key.starts_with("--") && f.key == name)
        || name == "--adapt-async"
        || matches!(name, "--batch" | "--slots")
        || matches!(name, "--kv-grow" | "--no-kv-grow")
        || FLAGS.contains(&name)
}
fn take_kv_grow(args: &mut Vec<String>, configured: bool) -> bool {
    let mut enabled = configured;
    args.retain(|arg| match arg.as_str() {
        "--kv-grow" => {
            enabled = true;
            false
        }
        "--no-kv-grow" => {
            enabled = false;
            false
        }
        _ => true,
    });
    enabled
}
fn take_kv_stream(args: &mut Vec<String>) -> Result<bool> {
    // Migrate the 32K preset; preserve custom resident sizes in Advanced.
    let mut remaining = args.clone();
    let value = take_argument(&mut remaining, "--kv-resident")?;
    let enabled = value
        .as_deref()
        .is_some_and(|v| v.parse::<u32>() == Ok(32768));
    if enabled {
        *args = remaining;
    }
    Ok(enabled)
}
fn take_parallel(args: &mut Vec<String>, configured: u32) -> Result<u32> {
    let mut parallel = configured;
    let mut found = false;
    let mut i = 0;
    while i < args.len() {
        if matches!(args[i].as_str(), "--batch" | "--slots") {
            if found {
                return Err("Duplicate concurrency options".into());
            }
            parallel = args
                .get(i + 1)
                .ok_or("Missing concurrency value")?
                .parse()?;
            args.drain(i..i + 2);
            found = true;
        } else {
            i += 1;
        }
    }
    Ok(parallel)
}
fn take_argument(arguments: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    let mut value = None;
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        let count = if argument == name {
            2
        } else if argument
            .split_once('=')
            .is_some_and(|(flag, _)| flag == name)
        {
            1
        } else {
            index += 1;
            continue;
        };
        if value.is_some() {
            return Err(format!("Duplicate engine option {name}").into());
        }
        value = Some(if count == 2 {
            arguments
                .get(index + 1)
                .filter(|v| !v.starts_with("--"))
                .ok_or_else(|| format!("Missing value for {name}"))?
                .clone()
        } else {
            argument
                .split_once('=')
                .expect("Matched option assignment")
                .1
                .to_owned()
        });
        if value.as_deref() == Some("") {
            return Err(format!("Missing value for {name}").into());
        }
        arguments.drain(index..index + count);
    }
    Ok(value)
}
impl Settings {
    fn without_calibration(&self, baseline: &Value) -> Result<Self> {
        let before = Self::from_json(baseline, self)?;
        let mut restored = self.clone();
        for (index, field) in FIELDS.iter().enumerate() {
            if CALIBRATION_FLAGS.contains(&field.key) {
                restored.values[index] = before.values[index].clone();
            }
        }
        restored.validate()?;
        Ok(restored)
    }
    fn calibrated(&self, applied: &Value) -> Result<Self> {
        let entries = applied.as_object().ok_or("Invalid calibration settings")?;
        // Older Strata releases calibrated six options, before prefill-pipe was added.
        if CALIBRATION_FLAGS
            .iter()
            .any(|flag| *flag != "--prefill-pipe" && !entries.contains_key(*flag))
            || entries
                .keys()
                .any(|key| !CALIBRATION_FLAGS.contains(&key.as_str()))
        {
            return Err("Unknown or incomplete calibration settings".into());
        }
        let mut result = self.clone();
        for flag in CALIBRATION_FLAGS {
            let Some(value) = applied.get(*flag) else {
                continue;
            };
            let index = FIELDS
                .iter()
                .position(|field| field.key == *flag)
                .expect("Calibration option has a dedicated field");
            if value.is_null() && *flag != "--spec-min-p" {
                result.values[index].clear();
                continue;
            }
            let value = value.as_str().ok_or("Invalid calibrated option value")?;
            let valid = match *flag {
                "--pool-workers" | "--adapt-every" => value.parse::<u32>().is_ok_and(|n| n > 0),
                "--adapt-swaps" => value.parse::<u32>().is_ok(),
                "--prefill-pipe" => value
                    .parse::<f64>()
                    .is_ok_and(|n| n.is_finite() && n >= 0.0),
                _ => value
                    .parse::<f64>()
                    .is_ok_and(|n| n.is_finite() && (0.0..=1.0).contains(&n)),
            };
            if !valid {
                return Err(format!("Invalid calibration value for {flag}").into());
            }
            result.values[index] = value.into();
        }
        result.validate()?;
        Ok(result)
    }
    fn from_config(config: &Value) -> Result<Self> {
        let engine = Engine::from_config(config);
        let mut args = args(config)?;
        let adapt_async = take_argument(&mut args, "--adapt-async")?
            .map(|value| match value.as_str() {
                "0" => Ok(false),
                "1" => Ok(true),
                _ => Err("Adapt async must be 0 or 1"),
            })
            .transpose()?;
        let mut kv_grow = take_kv_grow(&mut args, false);
        let kv_stream = take_kv_stream(&mut args)?;
        if let Some(value) = config["env"]["STRATA_KV_GROW"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            kv_grow = !value.starts_with('0');
        }
        let configured = match config.get("parallel") {
            Some(value) => u32::try_from(value.as_u64().ok_or("Invalid concurrency value")?)?,
            None => 1,
        };
        let parallel = take_parallel(&mut args, configured)?;
        let mut values = Vec::new();
        for field in FIELDS {
            values.push(if field.key.starts_with("--") {
                take_argument(&mut args, field.key)?.unwrap_or_default()
            } else {
                match config["sampling"].get(field.key) {
                    None => String::new(),
                    Some(value) => value
                        .as_number()
                        .ok_or_else(|| format!("Invalid sampling field {}", field.key))?
                        .to_string(),
                }
            });
        }
        let advanced = engine_options::extract(engine, &mut args)?;
        let mut extra = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if FLAGS.contains(&args[i].as_str()) {
                i += 1;
            } else if engine_options::managed(&args[i]) {
                return Err(format!("Unsupported {} option: {}", engine.name(), args[i]).into());
            } else if managed(&args[i]) {
                if i + 1 == args.len() {
                    return Err("Missing engine option value".into());
                }
                i += 2;
            } else {
                extra.push(args[i].clone());
                i += 1;
            }
        }
        Ok(Self {
            engine,
            gpu: None,
            gpu_keep_order: false,
            environment: {
                let mut environment = [None, None, None, None];
                for (index, value) in [
                    &config["env"],
                    &config["hub_experimental"]["env"],
                    &config["server_env"],
                    &config["hub_experimental"]["server_env"],
                ]
                .into_iter()
                .enumerate()
                {
                    if !value.is_null() || index % 2 == 0 {
                        let mut variables: Environment = if value.is_null() {
                            Environment::new()
                        } else {
                            serde_json::from_value(value.clone())?
                        };
                        if engine == Engine::Qwen && index < 2 {
                            variables.retain(|name, _| !managed_environment(name));
                        }
                        environment[index] = Some(variables);
                    }
                }
                environment
            },
            advanced,
            values,
            parallel,
            port: u16::try_from(config["port"].as_u64().ok_or("Missing server port")?)?,
            adapt_async,
            pin_by_need: config["env"]["STRATA_PIN_BY_NEED"].as_str() == Some("1"),
            resident: args.iter().any(|s| s == "--resident-experts"),
            kv_grow,
            kv_stream,
            vision: args.iter().any(|s| s == "--vision") || config["vision"].is_object(),
            extra_args: serde_json::to_string_pretty(&extra)?,
        })
    }
    fn to_json(&self) -> Value {
        json!({"gpu":self.gpu,"gpu_keep_order":self.gpu_keep_order,"environment":self.environment,"advanced":self.advanced,"values":self.values,"parallel":self.parallel,"port":self.port,"adapt_async":self.adapt_async,"pin_by_need":self.pin_by_need,
        "resident":self.resident,"kv_grow":self.kv_grow,"kv_stream":self.kv_stream,"vision":self.vision,"extra_args":self.extra_args})
    }
    fn from_json(value: &Value, original: &Self) -> Result<Self> {
        let mut values = value["values"]
            .as_array()
            .ok_or("Invalid saved settings")?
            .iter()
            .map(|v| v.as_str().map(str::to_owned).ok_or("Invalid saved setting"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let boolean = |key: &str| {
            value[key]
                .as_bool()
                .ok_or_else(|| format!("Invalid saved setting {key}"))
        };
        let mut extra: Vec<String> = serde_json::from_str(
            value["extra_args"]
                .as_str()
                .ok_or("Missing saved engine arguments")?,
        )?;
        // Before the checkbox existed, elastic KV lived in Advanced arguments.
        let legacy_kv_grow = take_kv_grow(&mut extra, false);
        let legacy_kv_stream = take_kv_stream(&mut extra)?;
        let kv_stream = match value.get("kv_stream") {
            Some(value) => value.as_bool().ok_or("Invalid saved setting kv_stream")?,
            None => legacy_kv_stream,
        };
        let kv_grow = match value.get("kv_grow") {
            Some(value) => value.as_bool().ok_or("Invalid saved setting kv_grow")?,
            None => legacy_kv_grow,
        };
        // Older profiles stored calibration options in Advanced arguments.
        // Append fields without shifting the saved Engine/Sampling indices.
        if values.len() == PERFORMANCE_FIELD_START {
            for field in &FIELDS[PERFORMANCE_FIELD_START..] {
                values.push(take_argument(&mut extra, field.key)?.unwrap_or_default());
            }
        }
        // Move existing raw options into named controls without changing their values.
        let mut advanced = engine_options::extract(original.engine, &mut extra)?;
        if let Some(saved) = value.get("advanced") {
            advanced.extend(serde_json::from_value::<BTreeMap<String, String>>(
                saved.clone(),
            )?);
        }
        // Migrate profiles saved before concurrency had a dedicated control.
        let parallel = match value.get("parallel") {
            Some(value) => u32::try_from(value.as_u64().ok_or("Invalid saved concurrency")?)?,
            None => take_parallel(&mut extra, original.parallel)?,
        };
        let result = Self {
            engine: original.engine,
            gpu: match value.get("gpu") {
                Some(value) => serde_json::from_value(value.clone())?,
                None => None,
            },
            gpu_keep_order: match value.get("gpu_keep_order") {
                Some(value) => value.as_bool().ok_or("Invalid saved GPU order")?,
                None => false,
            },
            environment: match value.get("environment") {
                Some(value) => serde_json::from_value(value.clone())?,
                None => original.environment.clone(),
            },
            advanced,
            values,
            parallel,
            port: match value.get("port") {
                Some(value) => u16::try_from(value.as_u64().ok_or("Invalid saved server port")?)?,
                None => original.port,
            },
            adapt_async: match value.get("adapt_async") {
                Some(Value::Null) => None,
                Some(Value::Bool(enabled)) => Some(*enabled),
                _ => return Err("Invalid saved setting adapt_async".into()),
            },
            pin_by_need: boolean("pin_by_need")?,
            resident: boolean("resident")?,
            kv_grow,
            kv_stream,
            vision: boolean("vision")?,
            extra_args: serde_json::to_string_pretty(&extra)?,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(devices) = &self.gpu {
            parse_gpu_selection(
                &devices
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            )?;
            if self
                .advanced
                .get("--gpu")
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Err(
                    "Clear Visible GPU indices in Advanced before using the GPU selector".into(),
                );
            }
            for variables in self.environment.iter().flatten() {
                if variables.keys().any(|key| {
                    [
                        "CUDA_VISIBLE_DEVICES",
                        "HIP_VISIBLE_DEVICES",
                        "ROCR_VISIBLE_DEVICES",
                        "GPU_DEVICE_ORDINAL",
                    ]
                    .iter()
                    .any(|name| key.eq_ignore_ascii_case(name))
                }) {
                    return Err("Remove GPU visibility overrides from Environment before using the GPU selector".into());
                }
            }
        }
        engine_options::validate(self.engine, &self.advanced)?;
        for (index, environment) in self.environment.iter().enumerate() {
            let Some(environment) = environment else {
                if index % 2 == 0 {
                    return Err("Standard environment must be an object".into());
                }
                continue;
            };
            validate_environment(environment)?;
            if self.engine == Engine::Qwen
                && index < 2
                && environment.keys().any(|name| managed_environment(name))
            {
                return Err(
                    "Use the Engine switches for STRATA_PIN_BY_NEED and STRATA_KV_GROW".into(),
                );
            }
        }
        if self.port == 0 {
            return Err("Server port must be between 1 and 65535".into());
        }
        if !(1..=8).contains(&self.parallel) {
            return Err("Concurrent requests must be between 1 and 8".into());
        }
        if self.values.len() != FIELDS.len() {
            return Err("Invalid settings field count".into());
        }
        for (i, field) in FIELDS.iter().enumerate() {
            let value = self.values[i].trim();
            // An empty field preserves the engine/server default by omitting the override.
            if value.is_empty() {
                continue;
            }
            if !self.engine.supports_field(i) {
                return Err(format!(
                    "{} is not supported by the {} engine",
                    field.label,
                    self.engine.name()
                )
                .into());
            }
            let valid = match field.key {
                "--max-context" => value.parse::<i32>().is_ok_and(|n| n > 0),
                "--vram-reserve-mib" | "--spec" | "top_k" | "--pool-workers" | "--adapt-every"
                | "--adapt-swaps" => value.parse::<i32>().is_ok_and(|n| n >= 0),
                "--expert-cache" => value == "auto" || value.parse::<i32>().is_ok_and(|n| n >= 0),
                "--prefill" => {
                    matches!(value, "auto" | "auto:8192" | "auto:16384" | "auto:32768")
                        || value.parse::<i32>().is_ok_and(|n| n > 0)
                }
                "--kv" => matches!(value, "int8" | "fp16" | "q4_0" | "k8v4"),
                _ => value.parse::<f64>().is_ok_and(|n| {
                    n.is_finite()
                        && match field.key {
                            "--spec-min-p" | "--pcie-frac" | "--adapt-decay" | "top_p"
                            | "min_p" => (0.0..=1.0).contains(&n),
                            "repetition_penalty" => n > 0.0,
                            "presence_penalty" => (-2.0..=2.0).contains(&n),
                            _ => n >= 0.0,
                        }
                }),
            };
            if !valid {
                return Err(format!("Invalid value: {}", field.label).into());
            }
        }
        let extra: Vec<String> = serde_json::from_str(&self.extra_args)
            .map_err(|_| "Engine arguments must be a JSON array of strings")?;
        if self.gpu.is_some()
            && extra
                .iter()
                .any(|arg| arg.split('=').next() == Some("--gpus"))
        {
            return Err(
                "Remove --gpus from Advanced arguments before using the GPU selector".into(),
            );
        }
        let custom_resident = self
            .advanced
            .get("--kv-resident")
            .filter(|v| !v.trim().is_empty());
        if self.kv_stream && custom_resident.is_some() {
            return Err(
                "Clear Custom GPU KV cells in Advanced before enabling KV streaming (32K).".into(),
            );
        }
        if self.kv_grow && (self.kv_stream || custom_resident.is_some_and(|v| v.trim() != "0")) {
            return Err("KV streaming and Elastic KV cannot be enabled together.".into());
        }
        if self.engine == Engine::DeepSeek
            && (self.parallel != 1
                || self.adapt_async == Some(true)
                || self.pin_by_need
                || self.resident
                || self.kv_grow
                || self.kv_stream
                || self.vision)
        {
            return Err("DeepSeek requires one request at a time and does not support the Qwen tuning switches".into());
        }
        if extra.iter().any(|s| {
            s.contains('\0')
                || managed(s)
                || s.split_once('=').is_some_and(|(name, _)| managed(name))
        }) {
            return Err(
                "Advanced arguments contain a managed option or an invalid character".into(),
            );
        }
        Ok(())
    }
    fn apply(&self, config: &mut Value, experimental: bool) -> Result<()> {
        self.validate()?;
        if self.engine != Engine::from_config(config) {
            return Err("Settings belong to a different engine family".into());
        }
        let deepseek = self.engine == Engine::DeepSeek;
        if !config.is_object()
            || (!config["sampling"].is_null() && !config["sampling"].is_object())
            || (!config["env"].is_null() && !config["env"].is_object())
        {
            return Err("Invalid profile: expected sampling and environment objects".into());
        }
        if self.vision && !config["vision"].is_object() {
            return Err("This profile has no configured vision projector".into());
        }
        for (base, key) in [(0, "env"), (2, "server_env")] {
            let environment = if experimental {
                self.environment[base + 1]
                    .as_ref()
                    .or(self.environment[base].as_ref())
            } else {
                self.environment[base].as_ref()
            }
            .ok_or("Missing standard environment")?;
            config[key] = json!(environment);
        }
        let mut args: Vec<String> = serde_json::from_str(&self.extra_args)?;
        engine_options::append(self.engine, &self.advanced, &mut args);
        for (field, value) in FIELDS.iter().zip(&self.values) {
            if value.trim().is_empty() {
                continue;
            }
            if field.key.starts_with("--") {
                args.extend([field.key.into(), value.trim().into()]);
            } else {
                if config["sampling"].is_null() {
                    config["sampling"] = json!({});
                }
                config["sampling"][field.key] = if field.key == "top_k" {
                    json!(value.trim().parse::<i32>()?)
                } else {
                    json!(value.trim().parse::<f64>()?)
                };
            }
        }
        if !deepseek && let Some(enabled) = self.adapt_async {
            args.extend([
                "--adapt-async".into(),
                if enabled { "1" } else { "0" }.into(),
            ]);
        }
        if self.resident {
            args.push("--resident-experts".into());
        }
        if self.kv_grow {
            args.push("--kv-grow".into());
        }
        if self.kv_stream {
            args.extend(["--kv-resident".into(), "32768".into()]);
            let reserve = take_argument(&mut args, "--vram-reserve-mib")?
                .map(|v| v.parse::<u32>())
                .transpose()?
                .unwrap_or(0)
                .max(1000);
            args.extend(["--vram-reserve-mib".into(), reserve.to_string()]);
        }
        if self.vision {
            args.push("--vision".into());
        } else {
            config
                .as_object_mut()
                .ok_or("Invalid profile")?
                .remove("vision");
        }
        config["args"] = json!(args);
        if let Some(devices) = &self.gpu {
            if let Some(value) = config.get("user_args").filter(|value| !value.is_null()) {
                let user_args: Vec<String> = serde_json::from_value(value.clone())?;
                if user_args.iter().any(|arg| {
                    matches!(
                        arg.split('=').next(),
                        Some("--gpu" | "--gpus" | "--layer-split")
                    )
                }) {
                    return Err("Remove GPU selection and layer split overrides from the source profile's user_args before using the GPU selector".into());
                }
            }
            let split = self
                .advanced
                .get("--layer-split")
                .map(String::as_str)
                .filter(|value| !value.trim().is_empty());
            let boundaries = match split {
                Some("auto") => None,
                Some(value) => Some(value.split(',').count()),
                None => match &config["layer_split"] {
                    Value::Null => None,
                    Value::String(value) if value == "auto" => None,
                    Value::String(value) => Some(value.split(',').count()),
                    Value::Number(_) => Some(1),
                    Value::Array(values) => Some(values.len()),
                    _ => return Err("Invalid source profile layer_split".into()),
                },
            };
            if let Some(boundaries) = boundaries {
                if boundaries + 1 != devices.len() {
                    return Err("GPU layer boundaries must match the selected GPU count. Set GPU layer boundaries to auto in Advanced.".into());
                }
            }
            // Clear upstream selection metadata so it cannot override this explicit choice.
            let object = config.as_object_mut().ok_or("Invalid profile")?;
            for key in [
                "gpu_roles",
                "gpu_cards",
                "gpu_none",
                "hip_ordinal",
                "gpu_order",
            ] {
                object.remove(key);
            }
            config["gpu"] = if devices.len() == 1 {
                json!(devices[0])
            } else {
                json!(devices)
            };
            if self.gpu_keep_order {
                config["gpu_order"] = json!("as_given");
            }
        }
        config["parallel"] = json!(self.parallel);
        config["port"] = json!(self.port);
        if config["env"].is_null() {
            config["env"] = json!({});
        }
        if !deepseek {
            config["env"]["STRATA_PIN_BY_NEED"] = json!(if self.pin_by_need { "1" } else { "0" });
            // The engine gives this environment variable priority over its CLI flag.
            config["env"]["STRATA_KV_GROW"] = json!(if self.kv_grow { "1" } else { "0" });
        }
        if self.kv_stream {
            // Streaming is incompatible with the experimental budget allocator.
            // configuration() starts from the base profile on every launch, so
            // disabling this preset restores its budget and selected reserve.
            config["env"]["STRATA_VRAM_BUDGET"] = json!("0");
        }
        config["open_browser"] = json!(false);
        Ok(())
    }
}

pub struct Profile {
    pub file: String,
    pub label: String,
    pub path: PathBuf,
    pub settings: Settings,
    pub gpu_description: String,
    pub has_vision: bool,
    pub has_experimental: bool,
    pub calibration: Option<Value>,
    aliases: Vec<String>,
    identity: Value,
    vision_config: Option<Value>,
}
pub struct Hub {
    pub profiles: Vec<Profile>,
    pub selected: usize,
    pub notice: String,
}
fn experimental_configured(config: &Value) -> bool {
    config["hub_experimental"]["exe"]
        .as_str()
        .is_some_and(|path| !path.trim().is_empty())
}
impl Hub {
    pub fn load() -> Result<Self> {
        let state_path = data::root().join("hub-profiles.json");
        let saved = match read_json(&state_path) {
            Ok(v) => v,
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                json!({})
            }
            Err(e) => return Err(e),
        };
        let mut paths = fs::read_dir(data::profiles_dir())?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.retain(|p| {
            p.extension().is_some_and(|e| e == "json")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("strata-"))
        });
        paths.sort();
        let configs = paths
            .into_iter()
            .map(|path| read_json(&path).map(|config| (path, config)))
            .collect::<Result<Vec<_>>>()?;
        Self::from_configs(configs, &saved)
    }
    fn from_configs(configs: Vec<(PathBuf, Value)>, saved: &Value) -> Result<Self> {
        let mut profiles: Vec<Profile> = Vec::new();
        for (path, config) in configs {
            if config.get("args").is_none() && config.get("model_name").is_none() {
                continue;
            }
            let file = path
                .file_name()
                .ok_or("Invalid profile path")?
                .to_str()
                .ok_or("Invalid profile name")?
                .to_owned();
            let label = match file.as_str() {
                EXPERIMENTAL_PROFILE => "Huihui Swift IQ3_S · Experimental".into(),
                "strata-swift-iq3_s-progressive.json" => {
                    "Huihui Swift IQ3_S · Progressive (test)".into()
                }
                "strata-swift-iq3_s-abliterated-no-images.json" => {
                    "Huihui Swift IQ3_S · Abliterated".into()
                }
                "strata-iq3_s-abliterated-no-images.json" => "Qwen IQ3_S · Abliterated".into(),
                "strata-iq3_s-abliterated.json" => "Qwen IQ3_S · Abliterated".into(),
                "strata-iq3_s-original.json" => "Qwen IQ3_S · Original".into(),
                _ => config["model_name"]
                    .as_str()
                    .ok_or("Missing model name")?
                    .to_owned(),
            };
            let original = Settings::from_config(&config)?;
            // Only merge modality variants of the same engine, weights and steering.
            // Keep separate credentials/endpoints and genuinely different model variants.
            let identity = json!({"model":config["model_name"],"exe":config["exe"],
                "cwd":config["cwd"],"tokenizer":config["tokenizer"],"host":config["host"],
                "port":config["port"],"api_key":config["api_key"],
                "arguments":serde_json::from_str::<Value>(&original.extra_args)?,
                "arch":config["arch"],"advanced":original.advanced,
                "gpu":config["gpu"],"gpu_roles":config["gpu_roles"],"hip_ordinal":config["hip_ordinal"],
                "gpu_order":config["gpu_order"],"gpu_none":config["gpu_none"]});
            let settings = if let Some(value) = saved["profiles"].get(&file) {
                let mut settings = Settings::from_json(value, &original)?;
                // Legacy Advanced arguments replaced the CLI list, but not the profile environment.
                if value.get("kv_grow").is_none()
                    && config["env"]["STRATA_KV_GROW"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                {
                    settings.kv_grow = original.kv_grow;
                }
                settings
            } else {
                original
            };
            let calibration = saved["calibrations"].get(&file).cloned();
            settings.validate()?;
            let vision_config = match config.get("hub_vision").or_else(|| config.get("vision")) {
                Some(value) if value.is_object() => Some(value.clone()),
                None => None,
                Some(_) => return Err(format!("Invalid vision assets in {file}").into()),
            };
            if let Some(existing) = profiles.iter_mut().find(|p| p.identity == identity) {
                if let Some(vision) = vision_config {
                    if existing
                        .vision_config
                        .as_ref()
                        .is_some_and(|old| *old != vision)
                    {
                        return Err(format!("Conflicting vision assets for {file}").into());
                    }
                    existing.vision_config = Some(vision);
                    existing.has_vision = true;
                }
                // The previously selected modality keeps all its tuning and enabled state.
                if saved["selected"].as_str() == Some(file.as_str()) {
                    existing.settings = settings;
                    existing.calibration = calibration;
                }
                existing.aliases.push(file);
                continue;
            }
            profiles.push(Profile {
                gpu_description: format!(
                    "Source profile GPU selection: {}{}.",
                    if config["gpu"].is_null() {
                        "automatic".into()
                    } else {
                        config["gpu"].to_string()
                    },
                    if config["backend"].as_str() == Some("hip") && !config["hip_ordinal"].is_null()
                    {
                        format!(" (HIP device {})", config["hip_ordinal"])
                    } else {
                        String::new()
                    }
                ),
                aliases: vec![file.clone()],
                identity,
                has_vision: vision_config.is_some(),
                has_experimental: experimental_configured(&config),
                vision_config,
                file,
                label,
                path,
                settings,
                calibration,
            });
        }
        for profile in &profiles {
            if profile.settings.vision && !profile.has_vision {
                return Err(format!(
                    "Saved vision settings are incompatible with {}",
                    profile.file
                )
                .into());
            }
        }
        let selected = match saved.get("selected").filter(|_| !profiles.is_empty()) {
            Some(name) => profiles
                .iter()
                .position(|p| {
                    p.aliases
                        .iter()
                        .any(|alias| Some(alias.as_str()) == name.as_str())
                })
                .ok_or("The saved selected profile no longer exists")?,
            None => 0,
        };
        Ok(Self {
            profiles,
            selected,
            notice: String::new(),
        })
    }
    fn saved_state(&self) -> Value {
        let profiles: serde_json::Map<String, Value> = self
            .profiles
            .iter()
            .flat_map(|p| {
                p.aliases
                    .iter()
                    .map(|alias| (alias.clone(), p.settings.to_json()))
            })
            .collect();
        let calibrations: serde_json::Map<String, Value> = self
            .profiles
            .iter()
            .flat_map(|p| {
                p.aliases.iter().filter_map(|alias| {
                    p.calibration
                        .as_ref()
                        .map(|value| (alias.clone(), value.clone()))
                })
            })
            .collect();
        json!({"selected":self.profiles[self.selected].file,"profiles":profiles,"calibrations":calibrations})
    }
    pub fn save(&mut self) -> Result<()> {
        write_json(&data::root().join("hub-profiles.json"), &self.saved_state())?;
        self.notice = "Settings saved.".into();
        Ok(())
    }
    pub fn calibration_request(&self) -> Result<Value> {
        let profile = &self.profiles[self.selected];
        Ok(
            json!({"profile":profile.file,"config":self.configuration()?,
            "original_settings":profile.settings.to_json(),
            "saved_state":optional_state(&data::root().join("hub-profiles.json"))?}),
        )
    }
    pub fn apply_calibration(&mut self, result: &Value, directory: &Path) -> Result<()> {
        self.apply_calibration_at(result, directory, &data::root().join("hub-profiles.json"))
    }
    fn apply_calibration_at(
        &mut self,
        result: &Value,
        directory: &Path,
        path: &Path,
    ) -> Result<()> {
        let profile = &self.profiles[self.selected];
        if result["profile"].as_str() != Some(profile.file.as_str())
            || result["original_settings"] != profile.settings.to_json()
        {
            return Err(
                "The selected profile changed during calibration. Results were not applied.".into(),
            );
        }
        let state = optional_state(path)?;
        if state != result["saved_state"] {
            return Err(
                "Saved profiles changed during calibration. Results were not applied.".into(),
            );
        }
        let speed = result["report"]["tok_s"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.0)
            .ok_or("Calibration has no valid measured speed")?;
        let settings = profile.settings.calibrated(&result["applied"])?;
        let baseline = profile.calibration_baseline()?;
        let metadata = json!({"date":result["date"],"tok_s":speed,"seconds":result["report"]["seconds"],
            "context":result["context"],"vision":result["vision"],"parallel":result["parallel"],
            "before":baseline,
            "result":directory.join("result.json"),"log":directory.join("operation.log")});
        let mut updated = if state.is_null() {
            self.saved_state()
        } else {
            state.clone()
        };
        for field in ["profiles", "calibrations"] {
            if updated[field].is_null() {
                updated[field] = json!({});
            } else if !updated[field].is_object() {
                return Err(format!("Invalid saved {field}").into());
            }
        }
        updated["selected"] = json!(profile.file);
        for alias in &profile.aliases {
            updated["profiles"][alias] = settings.to_json();
            updated["calibrations"][alias] = metadata.clone();
        }
        if !state.is_null() {
            fs::copy(path, directory.join("hub-profiles.before.json"))?;
        }
        write_json(path, &updated)?;
        self.profiles[self.selected].settings = settings;
        self.profiles[self.selected].calibration = Some(metadata);
        self.notice = format!("Calibration saved: {speed:.1} token/s in the calibration workload.");
        Ok(())
    }
    pub fn reset_calibration(&mut self, directory: &Path) -> Result<()> {
        self.reset_calibration_at(directory, &data::root().join("hub-profiles.json"))
    }
    fn reset_calibration_at(&mut self, directory: &Path, path: &Path) -> Result<()> {
        let profile = &self.profiles[self.selected];
        let metadata = profile
            .calibration
            .as_ref()
            .ok_or("This profile has no calibration to reset.")?;
        let mut state = read_json(path)?;
        if state["calibrations"][&profile.file] != *metadata {
            return Err(
                "The saved calibration changed. Reopen the hub before resetting it.".into(),
            );
        }
        let saved = Settings::from_json(&state["profiles"][&profile.file], &profile.settings)?;
        let current = Settings::from_json(&profile.settings.to_json(), &profile.settings)?;
        if saved.to_json() != current.to_json() {
            return Err(
                "Saved settings changed. Reopen the hub before resetting calibration.".into(),
            );
        }
        let restored = saved.without_calibration(&profile.calibration_baseline()?)?;
        for alias in &profile.aliases {
            state["profiles"][alias] = restored.to_json();
            state["calibrations"]
                .as_object_mut()
                .ok_or("Invalid saved calibrations")?
                .remove(alias);
        }
        fs::copy(path, directory.join("hub-profiles.before-reset.json"))?;
        write_json(path, &state)?;
        self.profiles[self.selected].settings = restored;
        self.profiles[self.selected].calibration = None;
        self.notice = "Calibration reset. Pre-calibration tuning restored.".into();
        Ok(())
    }
    pub fn reset(&mut self) -> Result<()> {
        self.profiles[self.selected].settings =
            Settings::from_config(&read_json(&self.profiles[self.selected].path)?)?;
        self.notice = "Original settings loaded. Click Save or Start.".into();
        Ok(())
    }
    pub fn configuration(&self) -> Result<Value> {
        self.engine_configuration(false)
    }
    pub fn engine_configuration(&self, experimental: bool) -> Result<Value> {
        let profile = &self.profiles[self.selected];
        let mut config = read_json(&profile.path)?;
        if experimental && !experimental_configured(&config) {
            return Err("No experimental engine path is configured for this model".into());
        }
        let runtime = config
            .as_object_mut()
            .ok_or("Invalid profile")?
            .remove("hub_experimental");
        if experimental {
            let runtime = runtime
                .as_ref()
                .and_then(Value::as_object)
                .ok_or("No experimental engine is configured for this model")?;
            for (key, value) in runtime {
                if ![
                    "exe",
                    "cwd",
                    "env",
                    "lib_dirs",
                    "server_dir",
                    "server_env",
                    "chat_template",
                ]
                .contains(&key.as_str())
                {
                    return Err(format!("Invalid experimental runtime field: {key}").into());
                }
                config[key] = value.clone();
            }
        }
        config
            .as_object_mut()
            .ok_or("Invalid profile")?
            .remove("hub_vision");
        if let Some(vision) = &profile.vision_config {
            config["vision"] = vision.clone();
        }
        profile.settings.apply(&mut config, experimental)?;
        if config.get("host").is_none() {
            config["host"] = json!("127.0.0.1");
        }
        if config["host"].as_str() != Some("127.0.0.1") {
            return Err("The hub requires Strata bound to 127.0.0.1".into());
        }
        for name in ["exe", "cwd", "tokenizer"] {
            let path = Path::new(
                config[name]
                    .as_str()
                    .ok_or_else(|| format!("Missing profile field {name}"))?,
            );
            if !path.exists() {
                return Err(format!("Missing path: {}", path.display()).into());
            }
        }
        let arguments = args(&config)?;
        for name in ["--pack", "--native", "--ple-gguf"] {
            if name == "--native" && config["arch"].as_str() == Some("deepseek41") {
                continue;
            }
            if name == "--ple-gguf" && !arguments.iter().any(|argument| argument == name) {
                continue;
            }
            let path = Path::new(argument(&arguments, name)?);
            if !path.exists() {
                return Err(format!("Missing path for {name}: {}", path.display()).into());
            }
        }
        if profile.settings.vision {
            for name in ["exe", "mmproj", "model"] {
                let path = Path::new(
                    config["vision"][name]
                        .as_str()
                        .ok_or("Incomplete vision profile")?,
                );
                if !path.is_file() {
                    return Err(format!("Missing vision file: {}", path.display()).into());
                }
            }
        }
        if config.get("log").is_none() {
            config["log"] = json!(
                data::root()
                    .join("logs")
                    .join(format!("{}.log", profile.file.trim_end_matches(".json")))
            );
        }
        Ok(config)
    }
    pub fn prepare(&self, experimental: bool) -> Result<PathBuf> {
        let config = self.engine_configuration(experimental)?;
        let log = Path::new(config["log"].as_str().ok_or("Invalid profile log path")?);
        fs::create_dir_all(log.parent().ok_or("Invalid profile log directory")?)?;
        let profile = &self.profiles[self.selected];
        let directory = data::root().join("profiles-run");
        fs::create_dir_all(&directory)?;
        let path = directory.join(&profile.file);
        write_json(&path, &config)?;
        Ok(path)
    }
    pub fn log_path(&self) -> Result<PathBuf> {
        let config = self.configuration()?;
        Ok(PathBuf::from(
            config["log"].as_str().ok_or("Missing profile log path")?,
        ))
    }
}
impl Profile {
    fn calibration_baseline(&self) -> Result<Value> {
        let Some(mut metadata) = self.calibration.clone() else {
            return Ok(self.settings.to_json());
        };
        // Migrate successful calibrations made before reset snapshots were stored inline.
        // Follow their retained reports to the first run; never substitute engine defaults.
        let mut visited = std::collections::BTreeSet::new();
        loop {
            if let Some(before) = metadata.get("before") {
                Settings::from_json(before, &self.settings)?;
                return Ok(before.clone());
            }
            let path = PathBuf::from(
                metadata["result"]
                    .as_str()
                    .ok_or("Missing pre-calibration snapshot")?,
            );
            if visited.len() >= 64 || !visited.insert(path.clone()) {
                return Err("Invalid calibration history: too many reports or a cycle".into());
            }
            if fs::metadata(&path)?.len() > 1024 * 1024 {
                return Err("Calibration report exceeds 1 MiB".into());
            }
            let result = read_json(&path)?;
            let file = result["profile"]
                .as_str()
                .ok_or("Missing calibration profile")?;
            if !self.aliases.iter().any(|alias| alias == file) {
                return Err("The calibration report belongs to another model profile".into());
            }
            if let Some(previous) = result["saved_state"]["calibrations"].get(file) {
                metadata = previous.clone();
            } else {
                let before = &result["original_settings"];
                Settings::from_json(before, &self.settings)?;
                return Ok(before.clone());
            }
        }
    }
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}
fn optional_state(path: &Path) -> Result<Value> {
    match read_json(path) {
        Ok(value) => Ok(value),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(Value::Null)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_selection_roundtrips_and_preserves_source_defaults() -> Result<()> {
        let source = json!({"port":8082,"args":["--native","model.gguf"],"gpu":[0,1],
            "gpu_order":"as_given","gpu_roles":{"0":"helper","1":"main"},
            "gpu_cards":{"0":"fixture"},"hip_ordinal":1,"gpu_none":false,
            "api_key":["fixture-key"]});
        let mut settings = Settings::from_config(&source)?;
        let mut legacy = settings.to_json();
        legacy.as_object_mut().unwrap().remove("gpu");
        legacy.as_object_mut().unwrap().remove("gpu_keep_order");
        settings = Settings::from_json(&legacy, &settings)?;
        let mut applied = source.clone();
        settings.apply(&mut applied, false)?;
        for key in [
            "gpu",
            "gpu_order",
            "gpu_roles",
            "gpu_cards",
            "gpu_none",
            "hip_ordinal",
        ] {
            assert_eq!(applied[key], source[key]);
        }
        settings.gpu = Some(parse_gpu_selection("2, 0")?);
        settings.gpu_keep_order = true;
        settings = Settings::from_json(&settings.to_json(), &settings)?;
        settings.apply(&mut applied, false)?;
        assert_eq!(applied["gpu"], json!([2, 0]));
        assert_eq!(applied["gpu_order"], "as_given");
        assert_eq!(applied["api_key"], source["api_key"]);
        for key in ["gpu_roles", "gpu_cards", "gpu_none", "hip_ordinal"] {
            assert!(applied.get(key).is_none());
        }
        settings.gpu = Some(vec![1]);
        settings.gpu_keep_order = false;
        settings.apply(&mut applied, false)?;
        assert_eq!(applied["gpu"], 1);
        assert!(applied.get("gpu_order").is_none());
        Ok(())
    }

    #[test]
    fn gpu_selection_rejects_invalid_and_conflicting_choices() -> Result<()> {
        for text in ["", "0,", "0,0", "-1", "0 1", "all", "4294967296"] {
            assert!(parse_gpu_selection(text).is_err(), "{text}");
        }
        let source = json!({"port":8082,"args":["--layer-split","12"]});
        let mut settings = Settings::from_config(&source)?;
        settings.gpu = Some(vec![0]);
        assert!(settings.apply(&mut source.clone(), false).is_err());
        settings
            .advanced
            .insert("--layer-split".into(), "auto".into());
        settings.apply(&mut source.clone(), false)?;
        let numeric_split = json!({"port":8082,"args":[],"layer_split":12});
        let mut numeric = Settings::from_config(&numeric_split)?;
        numeric.gpu = Some(vec![0]);
        assert!(numeric.apply(&mut numeric_split.clone(), false).is_err());
        settings.advanced.insert("--gpu".into(), "1".into());
        assert!(settings.validate().is_err());
        settings.advanced.remove("--gpu");
        settings.environment[0]
            .as_mut()
            .unwrap()
            .insert("cuda_visible_devices".into(), "1".into());
        assert!(settings.validate().is_err());
        let source = json!({"port":8082,"args":[],"arch":"deepseek41"});
        let mut settings = Settings::from_config(&source)?;
        settings.gpu = Some(vec![0, 1]);
        settings.apply(&mut source.clone(), false)?;
        Ok(())
    }

    #[test]
    fn environment_edits_preserve_values_and_reject_invalid_input() -> Result<()> {
        let variables = parse_environment(
            "STRATA_STAGER_SLEEP=0.001\r\nEMPTY=\r\nPATH=C:\\some folder\\bin\r\nEQUALS= a=b \r\n\r\n",
        )?;
        assert_eq!(variables["EMPTY"], "");
        assert_eq!(variables["PATH"], "C:\\some folder\\bin");
        assert_eq!(variables["EQUALS"], " a=b ");
        for text in [
            "MISSING",
            "=value",
            " NAME=value",
            "NAME=1\nNAME=2",
            "Path=1\nPATH=2",
            "NAME=bad\0value",
            "NAME=bad\rvalue",
        ] {
            assert!(parse_environment(text).is_err());
        }
        let mut settings = Settings::from_config(&json!({"port":8080,"args":[]}))?;
        settings.environment[0] = Some(parse_environment("strata_kv_grow=1")?);
        assert!(settings.validate().is_err());
        Ok(())
    }

    #[test]
    fn environment_settings_round_trip_migrate_and_select_runtime() -> Result<()> {
        let base = json!({"port":8080,"args":[],
            "env":{"KEEP":"standard","REMOVE":"old","STRATA_PIN_BY_NEED":"1"},
            "server_env":{"SERVER":"standard"},
            "hub_experimental":{"env":{"KEEP":"experimental"},
                "server_env":{"SERVER":"experimental"}}});
        let original = Settings::from_config(&base)?;
        let mut legacy = original.to_json();
        legacy.as_object_mut().unwrap().remove("environment");
        assert_eq!(
            Settings::from_json(&legacy, &original)?.environment,
            original.environment
        );
        let mut settings = original.clone();
        settings.environment[0] = Some(parse_environment("KEEP=changed\nEMPTY=")?);
        settings.environment[1] = Some(parse_environment("KEEP=experimental-edited")?);
        let restored = Settings::from_json(&settings.to_json(), &original)?;
        for experimental in [false, true] {
            let mut runtime = base.clone();
            restored.apply(&mut runtime, experimental)?;
            assert_eq!(
                runtime["env"]["KEEP"],
                if experimental {
                    "experimental-edited"
                } else {
                    "changed"
                }
            );
            assert_eq!(
                runtime["server_env"]["SERVER"],
                if experimental {
                    "experimental"
                } else {
                    "standard"
                }
            );
            assert!(runtime["env"].get("REMOVE").is_none());
            assert_eq!(runtime["env"]["STRATA_PIN_BY_NEED"], "1");
        }
        settings.environment[1] = None;
        settings.environment[3] = None;
        let mut inherited = base.clone();
        settings.apply(&mut inherited, true)?;
        assert_eq!(inherited["env"]["KEEP"], "changed");
        assert_eq!(inherited["server_env"]["SERVER"], "standard");
        settings.environment[1] = Some(Environment::new());
        settings.environment[3] = Some(Environment::new());
        settings.apply(&mut inherited, true)?;
        assert!(inherited["env"].get("KEEP").is_none());
        assert_eq!(inherited["server_env"], json!({}));
        assert_eq!(base["env"]["REMOVE"], "old");
        Ok(())
    }

    #[test]
    fn advanced_adapt_async_preserves_auto_and_legacy_overrides() -> Result<()> {
        let base = json!({"port":8082,"args":["--native","model.gguf"]});
        let original = Settings::from_config(&base)?;
        assert_eq!(original.adapt_async, None);
        for mode in [None, Some(true), Some(false)] {
            let mut settings = original.clone();
            settings.adapt_async = mode;
            let restored = Settings::from_json(&settings.to_json(), &original)?;
            assert_eq!(restored.adapt_async, mode);
            let mut applied = base.clone();
            restored.apply(&mut applied, false)?;
            let arguments = args(&applied)?;
            if let Some(enabled) = mode {
                assert_eq!(
                    argument(&arguments, "--adapt-async")?,
                    if enabled { "1" } else { "0" }
                );
            } else {
                assert!(!arguments.contains(&"--adapt-async".into()));
            }
            assert_eq!(Settings::from_config(&applied)?.adapt_async, mode);
        }
        Ok(())
    }

    #[test]
    fn advanced_deepseek_migrates_saved_arguments_and_round_trips() -> Result<()> {
        let base = json!({"arch":"deepseek41", "port":8082,
            "args":["--pack","deepseek-pack","--max-context","8192","--eos","1",
                "--dspark","auto","--dspark-conf","0.65"]});
        let original = Settings::from_config(&base)?;
        let mut legacy = original.to_json();
        legacy.as_object_mut().unwrap().remove("advanced");
        legacy["extra_args"] = json!(
            r#"["--pack","deepseek-pack","--eos","1","--experts","6","--dspark","on","--dspark-conf=0.7","--ssd-cache","12.5","--no-expert-cache"]"#
        );
        let migrated = Settings::from_json(&legacy, &original)?;
        assert_eq!(migrated.advanced["--dspark"], "on");
        assert_eq!(migrated.advanced["--ssd-cache"], "12.5");
        assert!(!migrated.extra_args.contains("dspark"));
        let reloaded = Settings::from_json(&migrated.to_json(), &original)?;
        let mut applied = base.clone();
        reloaded.apply(&mut applied, false)?;
        let arguments = args(&applied)?;
        assert_eq!(argument(&arguments, "--dspark")?, "on");
        assert_eq!(argument(&arguments, "--experts")?, "6");
        assert_eq!(arguments.iter().filter(|a| *a == "--dspark").count(), 1);
        assert!(arguments.contains(&"--no-expert-cache".into()));
        assert_eq!(argument(&arguments, "--eos")?, "1");
        assert_eq!(argument(&arguments, "--pack")?, "deepseek-pack");
        assert_eq!(Settings::from_config(&applied)?.advanced, reloaded.advanced);
        Ok(())
    }

    #[test]
    fn advanced_qwen_values_and_flags_preserve_unmanaged_arguments() -> Result<()> {
        let base = json!({"port":8082,"args":["--native","model.gguf","--prompt-cache","3",
            "--prompt-cache-spread","--expert-profile","profile.bin","--rope-scale","1.5"]});
        let original = Settings::from_config(&base)?;
        assert_eq!(original.advanced["--prompt-cache"], "3");
        let restored = Settings::from_json(&original.to_json(), &original)?;
        let mut applied = base.clone();
        restored.apply(&mut applied, false)?;
        assert_eq!(argument(&args(&applied)?, "--native")?, "model.gguf");
        assert_eq!(Settings::from_config(&applied)?.advanced, original.advanced);
        let mut cleared = restored.clone();
        cleared.advanced.remove("--prompt-cache");
        cleared.apply(&mut applied, false)?;
        assert!(!args(&applied)?.contains(&"--prompt-cache".into()));
        Ok(())
    }

    #[test]
    fn advanced_rejects_wrong_family_duplicates_and_invalid_values() -> Result<()> {
        let base = json!({"arch":"deepseek41","port":8082,"args":[]});
        let mut settings = Settings::from_config(&base)?;
        settings
            .advanced
            .insert("--dspark-conf".into(), "NaN".into());
        assert!(settings.validate().is_err());
        settings
            .advanced
            .insert("--dspark-conf".into(), "1.1".into());
        assert!(settings.validate().is_err());
        settings.advanced.clear();
        settings.advanced.insert("--experts".into(), "7".into());
        assert!(settings.validate().is_err());
        settings.advanced.insert("--experts".into(), "auto".into());
        settings.validate()?;
        settings.advanced.clear();
        settings
            .advanced
            .insert("--prompt-cache".into(), "3".into());
        assert!(settings.validate().is_err());
        settings.advanced.clear();
        settings.extra_args = r#"["--dspark=on"]"#.into();
        assert!(settings.validate().is_err());
        let duplicate =
            json!({"arch":"deepseek41","port":8082,"args":["--dspark","on","--dspark","off"]});
        assert!(Settings::from_config(&duplicate).is_err());
        let qwen_option = json!({"arch":"deepseek41","port":8082,"args":["--prompt-cache","3"]});
        assert!(Settings::from_config(&qwen_option).is_err());
        Ok(())
    }

    #[test]
    fn advanced_experimental_requires_an_explicit_engine_path() {
        for runtime in [Value::Null, json!({}), json!({"exe":" "}), json!({"exe":2})] {
            assert!(!experimental_configured(
                &json!({"hub_experimental":runtime})
            ));
        }
        assert!(experimental_configured(
            &json!({"hub_experimental":{"exe":"engine/strata.exe"}})
        ));
    }

    #[test]
    fn kv_stream_preset_round_trips_without_changing_base_settings() -> Result<()> {
        let base = json!({"port":8082,"args":["--native","same.gguf",
            "--max-context","131072","--kv","int8","--vram-reserve-mib","500"],
            "env":{"STRATA_VRAM_BUDGET":"1","OTHER":"kept"}});
        let original = Settings::from_config(&base)?;
        let mut legacy = original.to_json();
        legacy.as_object_mut().unwrap().remove("kv_stream");
        assert!(!Settings::from_json(&legacy, &original)?.kv_stream);
        let mut settings = original.clone();
        settings.kv_stream = true;
        let reloaded = Settings::from_json(&settings.to_json(), &original)?;
        assert!(reloaded.kv_stream);
        assert_eq!(reloaded.values, original.values);
        let mut enabled = base.clone();
        reloaded.apply(&mut enabled, false)?;
        assert_eq!(argument(&args(&enabled)?, "--kv-resident")?, "32768");
        assert_eq!(argument(&args(&enabled)?, "--vram-reserve-mib")?, "1000");
        assert_eq!(argument(&args(&enabled)?, "--max-context")?, "131072");
        assert_eq!(argument(&args(&enabled)?, "--kv")?, "int8");
        assert_eq!(enabled["env"]["STRATA_VRAM_BUDGET"], "0");
        assert_eq!(enabled["env"]["OTHER"], "kept");
        assert!(Settings::from_config(&enabled)?.kv_stream);
        settings.kv_stream = false;
        // The Hub reads the base profile afresh for each launch.
        let mut disabled = base.clone();
        settings.apply(&mut disabled, false)?;
        assert!(!args(&disabled)?.iter().any(|v| v == "--kv-resident"));
        assert_eq!(argument(&args(&disabled)?, "--vram-reserve-mib")?, "500");
        assert_eq!(disabled["env"]["STRATA_VRAM_BUDGET"], "1");
        assert_eq!(original.to_json(), Settings::from_config(&base)?.to_json());
        settings.kv_stream = true;
        settings.values[1] = "1500".into();
        settings.apply(&mut enabled, false)?;
        assert_eq!(argument(&args(&enabled)?, "--vram-reserve-mib")?, "1500");
        settings.values[1].clear();
        settings.apply(&mut enabled, false)?;
        assert_eq!(argument(&args(&enabled)?, "--vram-reserve-mib")?, "1000");
        settings.kv_grow = true;
        assert!(settings.validate().is_err());
        settings.kv_grow = false;
        settings.extra_args = json!(["--kv-resident=65536"]).to_string();
        assert!(settings.validate().is_err());
        settings.kv_stream = false;
        assert!(settings.validate().is_err());
        settings.extra_args = "[]".into();
        settings
            .advanced
            .insert("--kv-resident".into(), "65536".into());
        settings.validate()?;
        settings.extra_args = json!(["--kv-resident", "65536\0"]).to_string();
        assert!(settings.validate().is_err());
        Ok(())
    }

    #[test]
    fn kv_stream_migrates_32k_and_preserves_custom_resident_sizes() -> Result<()> {
        for spelling in [
            json!(["--kv-resident", "32768"]),
            json!(["--kv-resident=32768"]),
        ] {
            let config = json!({"port":8082,"args":spelling});
            let original = Settings::from_config(&config)?;
            assert!(original.kv_stream);
            assert!(!original.extra_args.contains("kv-resident"));
            let mut legacy = original.to_json();
            legacy.as_object_mut().unwrap().remove("kv_stream");
            legacy["extra_args"] = json!(spelling.to_string());
            assert!(Settings::from_json(&legacy, &original)?.kv_stream);
            legacy["kv_stream"] = json!(false);
            assert!(!Settings::from_json(&legacy, &original)?.kv_stream);
        }
        let custom = json!({"port":8082,"args":["--kv-resident","65536"]});
        let settings = Settings::from_config(&custom)?;
        assert!(!settings.kv_stream);
        assert_eq!(
            settings.advanced.get("--kv-resident").map(String::as_str),
            Some("65536")
        );
        let mut applied = custom.clone();
        settings.apply(&mut applied, false)?;
        assert_eq!(argument(&args(&applied)?, "--kv-resident")?, "65536");
        assert!(applied["env"]["STRATA_VRAM_BUDGET"].is_null());
        Ok(())
    }

    #[test]
    fn elastic_kv_migrates_legacy_flags_and_round_trips_the_toggle() -> Result<()> {
        let config = json!({"port":8082,"args":["--native","same.gguf","--kv-grow"],
            "env":{"STRATA_KV_GROW":"1","OTHER":"kept"}});
        let original = Settings::from_config(&config)?;
        assert!(original.kv_grow);
        assert!(!original.extra_args.contains("--kv-grow"));
        let mut legacy = original.to_json();
        legacy.as_object_mut().unwrap().remove("kv_grow");
        legacy["extra_args"] =
            json!(r#"["--native","same.gguf","--kv-grow","--no-kv-grow","--kv-grow"]"#);
        let mut settings = Settings::from_json(&legacy, &original)?;
        assert!(settings.kv_grow);
        assert!(!settings.extra_args.contains("kv-grow"));
        let mut legacy_disabled = legacy.clone();
        legacy_disabled["extra_args"] = json!(r#"["--native","same.gguf"]"#);
        assert!(!Settings::from_json(&legacy_disabled, &original)?.kv_grow);
        settings.kv_grow = false;
        let reloaded = Settings::from_json(&settings.to_json(), &original)?;
        assert!(!reloaded.kv_grow);
        let mut effective = config.clone();
        reloaded.apply(&mut effective, false)?;
        assert_eq!(effective["env"]["STRATA_KV_GROW"], "0");
        assert_eq!(effective["env"]["OTHER"], "kept");
        assert!(!args(&effective)?.iter().any(|a| a.contains("kv-grow")));
        assert!(!Settings::from_config(&effective)?.kv_grow);
        settings.kv_grow = true;
        settings.apply(&mut effective, false)?;
        assert_eq!(
            args(&effective)?
                .iter()
                .filter(|a| *a == "--kv-grow")
                .count(),
            1
        );
        assert_eq!(effective["env"]["STRATA_KV_GROW"], "1");
        settings.extra_args = json!(["--no-kv-grow"]).to_string();
        assert!(settings.validate().is_err());
        Ok(())
    }
    #[test]
    fn calibration_replaces_only_tuned_options_and_rejects_invalid_values() -> Result<()> {
        let original = json!({"args":["--pack","own-pack","--native","own.gguf","--max-context","131072",
            "--vram-reserve-mib","700","--spec-min-p","0.7","--pool-workers","7","--pcie-frac=0.35",
            "--control-vector-scaled","own-vector:1","--adapt-swaps","160"],"port":8082});
        let settings = Settings::from_config(&original)?;
        let applied = json!({"--pcie-frac":"0.55","--spec-min-p":"0.5","--pool-workers":"4",
            "--adapt-every":null,"--adapt-swaps":null,"--adapt-decay":null});
        let result = settings.calibrated(&applied)?;
        assert_eq!(&result.values[..6], &settings.values[..6]);
        assert_eq!(
            &result.values[7..PERFORMANCE_FIELD_START],
            &settings.values[7..PERFORMANCE_FIELD_START]
        );
        assert_eq!(result.values[6], "0.5");
        assert_eq!(result.port, settings.port);
        let mut effective = original.clone();
        result.apply(&mut effective, false)?;
        let extra = args(&effective)?;
        assert_eq!(argument(&extra, "--pool-workers")?, "4");
        assert_eq!(argument(&extra, "--pcie-frac")?, "0.55");
        assert_eq!(argument(&extra, "--control-vector-scaled")?, "own-vector:1");
        assert!(
            !extra
                .iter()
                .any(|arg| arg == "--adapt-swaps" || arg.starts_with("--pcie-frac="))
        );
        let mut invalid = applied.clone();
        invalid["--pool-workers"] = json!("0");
        assert!(settings.calibrated(&invalid).is_err());
        invalid = applied.clone();
        invalid["--pcie-frac"] = json!("NaN");
        assert!(settings.calibrated(&invalid).is_err());
        invalid = applied.clone();
        invalid["--native"] = json!("other.gguf");
        assert!(settings.calibrated(&invalid).is_err());
        let mut changed = result.clone();
        changed.values[0] = "65536".into();
        changed.values[7] = "0.8".into();
        changed.port = 8093;
        changed.vision = true;
        changed.parallel = 3;
        let mut extra: Vec<String> = serde_json::from_str(&changed.extra_args)?;
        extra.extend(["--new-option".into(), "kept".into()]);
        changed.extra_args = serde_json::to_string_pretty(&extra)?;
        let restored = changed.without_calibration(&settings.to_json())?;
        assert_eq!(restored.values[6], settings.values[6]);
        assert_eq!(&restored.values[..6], &changed.values[..6]);
        assert_eq!(
            &restored.values[7..PERFORMANCE_FIELD_START],
            &changed.values[7..PERFORMANCE_FIELD_START]
        );
        assert_eq!(restored.port, 8093);
        assert!(restored.vision);
        assert_eq!(restored.parallel, 3);
        assert_eq!(
            &restored.values[PERFORMANCE_FIELD_START..],
            &settings.values[PERFORMANCE_FIELD_START..]
        );
        assert_eq!(restored.extra_args, changed.extra_args);
        let defaults = Settings::from_config(&json!({"port":8082,"args":[]}))?;
        let restored_defaults = result.without_calibration(&defaults.to_json())?;
        assert_eq!(restored_defaults.values[6], "");
        assert!(
            restored_defaults.values[PERFORMANCE_FIELD_START..]
                .iter()
                .all(String::is_empty)
        );
        Ok(())
    }
    #[test]
    fn performance_fields_migrate_legacy_tuning_and_round_trip_without_duplicate_options()
    -> Result<()> {
        let original = json!({"port":8082,"args":["--native","own.gguf"],
            "sampling":{"temperature":1.0}});
        let defaults = Settings::from_config(&original)?;
        let mut legacy = defaults.to_json();
        legacy["values"]
            .as_array_mut()
            .unwrap()
            .truncate(PERFORMANCE_FIELD_START);
        legacy["extra_args"] = json!(serde_json::to_string(&[
            "--native",
            "own.gguf",
            "--pcie-frac=0.35",
            "--pool-workers",
            "7",
            "--adapt-every",
            "1",
            "--adapt-swaps",
            "160",
            "--adapt-decay",
            "0.97"
        ])?);
        let settings = Settings::from_json(&legacy, &defaults)?;
        assert_eq!(
            &settings.values[PERFORMANCE_FIELD_START..],
            &["0.35", "7", "1", "160", "0.97", ""]
        );
        assert_eq!(
            &settings.values[..PERFORMANCE_FIELD_START],
            &defaults.values[..PERFORMANCE_FIELD_START]
        );
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&settings.extra_args)?,
            ["--native", "own.gguf"]
        );
        let reloaded = Settings::from_json(&settings.to_json(), &defaults)?;
        let mut effective = original.clone();
        reloaded.apply(&mut effective, false)?;
        let arguments = args(&effective)?;
        for (field, value) in FIELDS
            .iter()
            .zip(&settings.values)
            .skip(PERFORMANCE_FIELD_START)
        {
            assert_eq!(
                arguments.iter().filter(|s| *s == field.key).count(),
                usize::from(!value.is_empty())
            );
        }
        assert_eq!(
            Settings::from_config(&effective)?.to_json(),
            settings.to_json()
        );
        let mut edited = reloaded;
        edited.values[13] = "0.2".into();
        edited.values[14].clear();
        edited.values[18] = "2".into();
        edited.apply(&mut effective, false)?;
        assert_eq!(argument(&args(&effective)?, "--pcie-frac")?, "0.2");
        assert!(!args(&effective)?.iter().any(|v| v == "--pool-workers"));
        assert_eq!(argument(&args(&effective)?, "--prefill-pipe")?, "2");
        let applied = json!({"--pcie-frac":"0.35","--spec-min-p":"0.7","--pool-workers":null,
            "--adapt-every":"1","--adapt-swaps":"160","--adapt-decay":"0.97","--prefill-pipe":"1"});
        assert_eq!(edited.calibrated(&applied)?.values[18], "1");
        let mut defaults_applied = applied;
        defaults_applied["--prefill-pipe"] = Value::Null;
        assert_eq!(edited.calibrated(&defaults_applied)?.values[18], "");
        for (index, value) in [
            (13, "1.1"),
            (14, "1.5"),
            (15, "-1"),
            (16, "-1"),
            (17, "NaN"),
        ] {
            let mut invalid = settings.clone();
            invalid.values[index] = value.into();
            assert!(invalid.validate().is_err());
        }
        legacy["extra_args"] = json!("[\"--pcie-frac\",\"0.3\",\"--pcie-frac=0.4\"]");
        assert!(Settings::from_json(&legacy, &defaults).is_err());
        legacy["extra_args"] = json!("[\"--adapt-swaps\"]");
        assert!(Settings::from_json(&legacy, &defaults).is_err());
        Ok(())
    }
    #[test]
    fn calibration_is_backed_up_persisted_for_aliases_and_refuses_stale_results() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("strata-calibration-test-{}", std::process::id()));
        fs::create_dir(&directory)?;
        let path = directory.join("hub-profiles.json");
        let text = json!({"model_name":"fixture","port":8082,"args":["--native","own.gguf"]});
        let mut vision = text.clone();
        vision["vision"] = json!({"model":"own.gguf"});
        let other = json!({"model_name":"other","port":8082,"args":["--native","other.gguf"]});
        let configs = vec![
            ("strata-text.json".into(), text),
            ("strata-vision.json".into(), vision),
            ("strata-other.json".into(), other),
        ];
        let mut hub = Hub::from_configs(configs.clone(), &json!({}))?;
        let mut before = hub.saved_state();
        before["profiles"]["strata-other.json"]["port"] = json!(8999);
        write_json(&path, &before)?;
        let result = json!({"profile":hub.profiles[0].file,"original_settings":hub.profiles[0].settings.to_json(),
            "saved_state":before,"date":"2026-10-17T12:00:00+02:00","report":{"tok_s":70.0,"seconds":42},
            "context":"131072","vision":false,"parallel":1,
            "applied":{"--pcie-frac":"0.55","--spec-min-p":"0.7","--pool-workers":"4",
                "--adapt-every":null,"--adapt-swaps":null,"--adapt-decay":null}});
        hub.apply_calibration_at(&result, &directory, &path)?;
        let saved = read_json(&path)?;
        assert_eq!(
            read_json(&directory.join("hub-profiles.before.json"))?,
            before
        );
        assert_eq!(
            saved["profiles"]["strata-other.json"],
            before["profiles"]["strata-other.json"]
        );
        assert_eq!(
            saved["profiles"]["strata-text.json"],
            saved["profiles"]["strata-vision.json"]
        );
        assert_eq!(saved["calibrations"]["strata-text.json"]["tok_s"], 70.0);
        let reloaded = Hub::from_configs(configs.clone(), &saved)?;
        assert_eq!(reloaded.profiles[0].settings.values[6], "0.7");
        assert_eq!(
            reloaded.profiles[0].calibration.as_ref().unwrap()["tok_s"],
            70.0
        );
        assert!(
            hub.apply_calibration_at(&result, &directory, &path)
                .is_err()
        );
        assert_eq!(read_json(&path)?, saved);
        assert_eq!(
            saved["calibrations"]["strata-text.json"]["before"],
            result["original_settings"]
        );

        // Recalibrating must retain the first baseline, including after a restart.
        let second_directory = directory.join("second-run");
        fs::create_dir(&second_directory)?;
        hub = reloaded;
        let mut second_result = result.clone();
        second_result["original_settings"] = hub.profiles[0].settings.to_json();
        second_result["saved_state"] = saved.clone();
        second_result["applied"]["--spec-min-p"] = json!("0.4");
        hub.apply_calibration_at(&second_result, &second_directory, &path)?;
        assert_eq!(
            hub.profiles[0].calibration.as_ref().unwrap()["before"],
            result["original_settings"]
        );
        hub.profiles[0].settings.values[0] = "65536".into();
        hub.profiles[0].settings.values[7] = "0.9".into();
        hub.profiles[0].settings.port = 8094;
        hub.profiles[0].settings.parallel = 2;
        hub.profiles[0].settings.vision = true;
        let before_reset = hub.saved_state();
        write_json(&path, &before_reset)?;
        hub = Hub::from_configs(configs.clone(), &before_reset)?;
        let reset_directory = directory.join("reset");
        fs::create_dir(&reset_directory)?;
        let mut changed = before_reset.clone();
        changed["profiles"]["strata-text.json"]["port"] = json!(8888);
        write_json(&path, &changed)?;
        assert!(hub.reset_calibration_at(&reset_directory, &path).is_err());
        assert_eq!(read_json(&path)?, changed);
        write_json(&path, &before_reset)?;
        hub.reset_calibration_at(&reset_directory, &path)?;
        assert_eq!(
            read_json(&reset_directory.join("hub-profiles.before-reset.json"))?,
            before_reset
        );
        let reset = read_json(&path)?;
        assert_eq!(
            reset["profiles"]["strata-other.json"],
            saved["profiles"]["strata-other.json"]
        );
        assert_eq!(
            reset["profiles"]["strata-text.json"],
            reset["profiles"]["strata-vision.json"]
        );
        assert!(reset["calibrations"].as_object().unwrap().is_empty());
        let reloaded = Hub::from_configs(configs, &reset)?;
        let restored = &reloaded.profiles[0];
        assert!(restored.calibration.is_none());
        assert_eq!(restored.settings.values[6], "");
        assert!(
            restored.settings.values[PERFORMANCE_FIELD_START..]
                .iter()
                .all(String::is_empty)
        );
        assert_eq!(restored.settings.values[0], "65536");
        assert_eq!(restored.settings.values[7], "0.9");
        assert_eq!(restored.settings.port, 8094);
        assert_eq!(restored.settings.parallel, 2);
        assert!(restored.settings.vision);
        assert!(hub.reset_calibration_at(&reset_directory, &path).is_err());
        fs::remove_dir_all(directory)?;
        Ok(())
    }
    #[test]
    fn calibration_reset_reads_legacy_reports_and_rejects_missing_snapshots() -> Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "strata-calibration-legacy-test-{}",
            std::process::id()
        ));
        fs::create_dir(&directory)?;
        let mut hub = Hub::from_configs(
            vec![(
                "strata-text.json".into(),
                json!({"model_name":"fixture", "port":8082,
            "args":["--native","own.gguf","--spec-min-p","0.65","--pool-workers","6"]}),
            )],
            &json!({}),
        )?;
        let before = hub.profiles[0].settings.to_json();
        let first = directory.join("first.json");
        write_json(
            &first,
            &json!({"profile":"strata-text.json","original_settings":before,"saved_state":null}),
        )?;
        let second = directory.join("second.json");
        write_json(
            &second,
            &json!({"profile":"strata-text.json","saved_state":{"calibrations":{
            "strata-text.json":{"result":first}}}}),
        )?;
        hub.profiles[0].calibration = Some(json!({"result":second}));
        assert_eq!(hub.profiles[0].calibration_baseline()?, before);
        // A missing historical report must never silently reset to engine defaults.
        fs::remove_file(&first)?;
        assert!(hub.profiles[0].calibration_baseline().is_err());
        fs::remove_dir_all(directory)?;
        Ok(())
    }
    #[test]
    fn modality_variants_share_assets_and_keep_the_selected_saved_settings() -> Result<()> {
        let text = json!({"model_name":"fixture", "args":["--native","model.gguf"], "port":8082});
        let mut vision = text.clone();
        vision["vision"] =
            json!({"exe":"encoder.exe", "mmproj":"projector.gguf", "model":"model.gguf"});
        let mut tuning = Settings::from_config(&vision)?;
        tuning.values[0] = "65536".into();
        tuning.values[1] = "700".into();
        tuning.port = 8097;
        tuning.parallel = 2;
        let saved = json!({"selected":"strata-vision.json", "profiles":{"strata-vision.json":tuning.to_json()}});
        let hub = Hub::from_configs(
            vec![
                ("strata-text.json".into(), text.clone()),
                ("strata-vision.json".into(), vision.clone()),
            ],
            &saved,
        )?;
        assert_eq!(hub.profiles.len(), 1);
        let profile = &hub.profiles[hub.selected];
        assert_eq!(profile.aliases, ["strata-text.json", "strata-vision.json"]);
        assert!(profile.has_vision && profile.settings.vision);
        assert_eq!(profile.settings.values[0], "65536");
        assert_eq!(profile.settings.values[1], "700");
        assert_eq!(profile.settings.port, 8097);
        assert_eq!(profile.settings.parallel, 2);
        let hub = Hub::from_configs(
            vec![
                ("strata-text.json".into(), text),
                ("strata-vision.json".into(), vision),
            ],
            &json!({"selected":"strata-text.json"}),
        )?;
        assert!(hub.profiles[0].has_vision);
        assert!(!hub.profiles[0].settings.vision);
        Ok(())
    }
    #[test]
    fn optional_vision_assets_do_not_enable_images_by_default() -> Result<()> {
        let config = json!({"model_name":"swift-fixture", "args":["--native","swift.gguf"], "port":8082,
            "hub_vision":{"exe":"encoder.exe", "mmproj":"swift-projector.gguf", "model":"swift.gguf"}});
        let hub = Hub::from_configs(vec![("strata-swift.json".into(), config)], &json!({}))?;
        assert!(hub.profiles[0].has_vision);
        assert!(!hub.profiles[0].settings.vision);
        let mut runtime = json!({"args":[], "vision":hub.profiles[0].vision_config});
        let mut settings = hub.profiles[0].settings.clone();
        settings.vision = true;
        settings.apply(&mut runtime, false)?;
        assert_eq!(runtime["vision"]["model"], "swift.gguf");
        assert!(args(&runtime)?.iter().any(|arg| arg == "--vision"));
        settings.vision = false;
        settings.apply(&mut runtime, false)?;
        assert!(runtime.get("vision").is_none());
        assert!(!args(&runtime)?.iter().any(|arg| arg == "--vision"));
        Ok(())
    }
    #[test]
    fn distinct_steering_stays_separate_and_conflicting_assets_fail() -> Result<()> {
        let first = json!({"model_name":"fixture", "args":["--native","model.gguf"], "port":8082,
            "vision":{"mmproj":"first.gguf"}});
        let mut second = first.clone();
        second["args"] = json!([
            "--native",
            "model.gguf",
            "--control-vector-scaled",
            "vector.gguf:1"
        ]);
        assert_eq!(
            Hub::from_configs(
                vec![
                    ("strata-a.json".into(), first.clone()),
                    ("strata-b.json".into(), second)
                ],
                &json!({})
            )?
            .profiles
            .len(),
            2
        );
        let mut conflict = first.clone();
        conflict["vision"]["mmproj"] = json!("other.gguf");
        assert!(
            Hub::from_configs(
                vec![
                    ("strata-a.json".into(), first),
                    ("strata-b.json".into(), conflict)
                ],
                &json!({})
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn sparse_profiles_keep_unset_engine_and_sampling_defaults() -> Result<()> {
        let original = json!({"args":["--pack","pack","--native","model.gguf"],"port":8080});
        let settings = Settings::from_config(&original)?;
        settings.validate()?;
        assert!(settings.values.iter().all(String::is_empty));
        let mut result = original.clone();
        settings.apply(&mut result, false)?;
        assert!(result.get("sampling").is_none());
        let arguments = args(&result)?;
        assert!(
            !arguments
                .iter()
                .any(|argument| argument == "--max-context" || argument == "--vram-reserve-mib")
        );
        Ok(())
    }
    #[test]
    fn deepseek_preserves_pack_arguments_without_qwen_overrides() -> Result<()> {
        let original = json!({"arch":"deepseek41", "port":8082,
            "args":["--pack","deepseek-pack","--max-context","8192","--eos","1","--dspark","auto","--dspark-conf","0.65"]});
        let settings = Settings::from_config(&original)?;
        let mut result = original.clone();
        settings.apply(&mut result, false)?;
        let arguments = args(&result)?;
        assert_eq!(argument(&arguments, "--pack")?, "deepseek-pack");
        assert_eq!(argument(&arguments, "--dspark")?, "auto");
        assert_eq!(argument(&arguments, "--max-context")?, "8192");
        assert!(
            !arguments
                .iter()
                .any(|arg| arg == "--adapt-async" || arg == "--native")
        );
        assert!(
            result["env"]
                .as_object()
                .ok_or("Missing environment")?
                .is_empty()
        );
        let mut unsupported = settings.clone();
        unsupported.kv_grow = true;
        assert!(unsupported.apply(&mut original.clone(), false).is_err());
        unsupported = settings.clone();
        unsupported.values[1] = "1000".into();
        assert!(unsupported.apply(&mut original.clone(), false).is_err());
        Ok(())
    }
    #[test]
    fn hub_settings_preserve_model_and_validate_overrides() -> Result<()> {
        let original = json!({"args":["--pack","pack","--native","model.gguf","--ple-gguf","model.gguf","--control-vector-layer-range","4","44","--max-context","131072","--vram-reserve-mib","1536","--prefill","auto","--expert-cache","auto","--kv","int8","--spec","4","--spec-min-p","0.5","--vision"],"vision":{"mmproj":"projection.gguf"},"sampling":{"temperature":1.0,"top_p":0.95,"top_k":20,"min_p":0.0,"repetition_penalty":1.0,"presence_penalty":0.0},"api_key":["fixture-key"],"env":{"KEEP":"yes"},"tokenizer":"tokenizer","port":8082});
        let mut settings = Settings::from_config(&original)?;
        settings.values[0] = "65536".into();
        settings.values[7] = "0.7".into();
        settings.adapt_async = Some(true);
        settings.pin_by_need = true;
        settings.vision = false;
        settings.parallel = 2;
        settings.port = 8091;
        let settings =
            Settings::from_json(&settings.to_json(), &Settings::from_config(&original)?)?;
        let mut result = original.clone();
        settings.apply(&mut result, false)?;
        let arguments = args(&result)?;
        assert_eq!(argument(&arguments, "--native")?, "model.gguf");
        assert!(
            arguments
                .windows(3)
                .any(|a| a == ["--control-vector-layer-range", "4", "44"])
        );
        assert_eq!(argument(&arguments, "--max-context")?, "65536");
        assert_eq!(
            arguments
                .iter()
                .filter(|s| s.as_str() == "--max-context")
                .count(),
            1
        );
        assert_eq!(result["api_key"], original["api_key"]);
        assert_eq!(result["tokenizer"], original["tokenizer"]);
        assert_eq!(result["env"]["KEEP"], "yes");
        assert_eq!(result["env"]["STRATA_PIN_BY_NEED"], "1");
        assert_eq!(result["sampling"]["temperature"], 0.7);
        assert_eq!(result["parallel"], 2);
        assert_eq!(result["port"], 8091);
        assert!(result.get("vision").is_none());
        let mut invalid = settings.clone();
        invalid.values[0] = "0".into();
        assert!(invalid.validate().is_err());
        let mut legacy = settings.to_json();
        legacy.as_object_mut().unwrap().remove("parallel");
        legacy["extra_args"] = json!("[\"--batch\",\"3\",\"--native\",\"model.gguf\"]");
        let migrated = Settings::from_json(&legacy, &Settings::from_config(&original)?)?;
        assert_eq!(migrated.parallel, 3);
        assert!(!migrated.extra_args.contains("--batch"));
        invalid = settings.clone();
        invalid.parallel = 9;
        assert!(invalid.validate().is_err());
        invalid = settings.clone();
        invalid.extra_args = "[\"--slots\",\"2\"]".into();
        assert!(invalid.validate().is_err());
        invalid = settings.clone();
        invalid.values[7] = "NaN".into();
        assert!(invalid.validate().is_err());
        invalid = settings.clone();
        invalid.extra_args = "[\"--max-context\",\"1\"]".into();
        assert!(invalid.validate().is_err());
        Ok(())
    }
}
