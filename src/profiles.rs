use crate::data::{self, Result};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub choices: &'static [&'static str],
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
];
const FLAGS: &[&str] = &["--resident-experts", "--vision"];

#[derive(Clone)]
pub struct Settings {
    pub values: Vec<String>,
    pub parallel: u32,
    pub port: u16,
    pub adapt_async: bool,
    pub pin_by_need: bool,
    pub resident: bool,
    pub vision: bool,
    pub extra_args: String,
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
    FIELDS
        .iter()
        .any(|f| f.key.starts_with("--") && f.key == name)
        || name == "--adapt-async"
        || matches!(name, "--batch" | "--slots")
        || FLAGS.contains(&name)
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
impl Settings {
    fn from_config(config: &Value) -> Result<Self> {
        let mut args = args(config)?;
        let configured = match config.get("parallel") {
            Some(value) => u32::try_from(value.as_u64().ok_or("Invalid concurrency value")?)?,
            None => 1,
        };
        let parallel = take_parallel(&mut args, configured)?;
        let mut values = Vec::new();
        for field in FIELDS {
            values.push(if field.key.starts_with("--") {
                if args.iter().any(|argument| argument == field.key) {
                    argument(&args, field.key)?.to_owned()
                } else {
                    String::new()
                }
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
        let mut extra = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if FLAGS.contains(&args[i].as_str()) {
                i += 1;
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
            values,
            parallel,
            port: u16::try_from(config["port"].as_u64().ok_or("Missing server port")?)?,
            adapt_async: args
                .windows(2)
                .any(|a| a[0] == "--adapt-async" && a[1] == "1"),
            pin_by_need: config["env"]["STRATA_PIN_BY_NEED"].as_str() == Some("1"),
            resident: args.iter().any(|s| s == "--resident-experts"),
            vision: args.iter().any(|s| s == "--vision") || config["vision"].is_object(),
            extra_args: serde_json::to_string_pretty(&extra)?,
        })
    }
    fn to_json(&self) -> Value {
        json!({"values":self.values,"parallel":self.parallel,"port":self.port,"adapt_async":self.adapt_async,"pin_by_need":self.pin_by_need,
        "resident":self.resident,"vision":self.vision,"extra_args":self.extra_args})
    }
    fn from_json(value: &Value, original: &Self) -> Result<Self> {
        let values = value["values"]
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
        // Migrate profiles saved before concurrency had a dedicated control.
        let parallel = match value.get("parallel") {
            Some(value) => u32::try_from(value.as_u64().ok_or("Invalid saved concurrency")?)?,
            None => take_parallel(&mut extra, original.parallel)?,
        };
        let result = Self {
            values,
            parallel,
            port: match value.get("port") {
                Some(value) => u16::try_from(value.as_u64().ok_or("Invalid saved server port")?)?,
                None => original.port,
            },
            adapt_async: boolean("adapt_async")?,
            pin_by_need: boolean("pin_by_need")?,
            resident: boolean("resident")?,
            vision: boolean("vision")?,
            extra_args: serde_json::to_string_pretty(&extra)?,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
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
            let valid = match field.key {
                "--max-context" => value.parse::<i32>().is_ok_and(|n| n > 0),
                "--vram-reserve-mib" | "--spec" | "top_k" => {
                    value.parse::<i32>().is_ok_and(|n| n >= 0)
                }
                "--expert-cache" => value == "auto" || value.parse::<i32>().is_ok_and(|n| n >= 0),
                "--prefill" => {
                    matches!(value, "auto" | "auto:8192" | "auto:16384" | "auto:32768")
                        || value.parse::<i32>().is_ok_and(|n| n > 0)
                }
                "--kv" => matches!(value, "int8" | "fp16" | "q4_0" | "k8v4"),
                _ => value.parse::<f64>().is_ok_and(|n| {
                    n.is_finite()
                        && match field.key {
                            "--spec-min-p" | "top_p" | "min_p" => (0.0..=1.0).contains(&n),
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
    fn apply(&self, config: &mut Value) -> Result<()> {
        self.validate()?;
        if !config.is_object()
            || (!config["sampling"].is_null() && !config["sampling"].is_object())
            || (!config["env"].is_null() && !config["env"].is_object())
        {
            return Err("Invalid profile: expected sampling and environment objects".into());
        }
        if self.vision && !config["vision"].is_object() {
            return Err("This profile has no configured vision projector".into());
        }
        let mut args: Vec<String> = serde_json::from_str(&self.extra_args)?;
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
        args.extend([
            "--adapt-async".into(),
            if self.adapt_async { "1" } else { "0" }.into(),
        ]);
        if self.resident {
            args.push("--resident-experts".into());
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
        config["parallel"] = json!(self.parallel);
        config["port"] = json!(self.port);
        if config["env"].is_null() {
            config["env"] = json!({});
        }
        config["env"]["STRATA_PIN_BY_NEED"] = json!(if self.pin_by_need { "1" } else { "0" });
        config["open_browser"] = json!(false);
        Ok(())
    }
}

pub struct Profile {
    pub file: String,
    pub label: String,
    pub path: PathBuf,
    pub settings: Settings,
    pub has_vision: bool,
}
pub struct Hub {
    pub profiles: Vec<Profile>,
    pub selected: usize,
    pub notice: String,
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
        let mut profiles = Vec::new();
        for path in paths {
            let config = read_json(&path)?;
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
                "strata-swift-iq3_s-abliterated-no-images.json" => {
                    "Huihui Swift IQ3_S · Abliterated · Text".into()
                }
                "strata-iq3_s-abliterated-no-images.json" => {
                    "Qwen IQ3_S · Abliterated · Text".into()
                }
                "strata-iq3_s-abliterated.json" => "Qwen IQ3_S · Abliterated · Vision".into(),
                "strata-iq3_s-original.json" => "Qwen IQ3_S · Original · Vision".into(),
                _ => config["model_name"]
                    .as_str()
                    .ok_or("Missing model name")?
                    .to_owned(),
            };
            let settings = if let Some(value) = saved["profiles"].get(&file) {
                Settings::from_json(value, &Settings::from_config(&config)?)?
            } else {
                Settings::from_config(&config)?
            };
            settings.validate()?;
            let has_vision = config["vision"].is_object();
            if settings.vision && !has_vision {
                return Err(format!("Saved vision settings are incompatible with {file}").into());
            }
            profiles.push(Profile {
                file,
                label,
                path,
                settings,
                has_vision,
            });
        }
        let selected = match saved.get("selected").filter(|_| !profiles.is_empty()) {
            Some(name) => profiles
                .iter()
                .position(|p| Some(p.file.as_str()) == name.as_str())
                .ok_or("The saved selected profile no longer exists")?,
            None => 0,
        };
        Ok(Self {
            profiles,
            selected,
            notice: String::new(),
        })
    }
    pub fn save(&mut self) -> Result<()> {
        let profiles: serde_json::Map<String, Value> = self
            .profiles
            .iter()
            .map(|p| (p.file.clone(), p.settings.to_json()))
            .collect();
        write_json(
            &data::root().join("hub-profiles.json"),
            &json!({"selected":self.profiles[self.selected].file,"profiles":profiles}),
        )?;
        self.notice = "Settings saved.".into();
        Ok(())
    }
    pub fn reset(&mut self) -> Result<()> {
        self.profiles[self.selected].settings =
            Settings::from_config(&read_json(&self.profiles[self.selected].path)?)?;
        self.notice = "Original settings loaded. Click Save or Start.".into();
        Ok(())
    }
    pub fn configuration(&self) -> Result<Value> {
        let profile = &self.profiles[self.selected];
        let mut config = read_json(&profile.path)?;
        profile.settings.apply(&mut config)?;
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
    pub fn prepare(&self) -> Result<PathBuf> {
        let config = self.configuration()?;
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
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sparse_profiles_keep_unset_engine_and_sampling_defaults() -> Result<()> {
        let original = json!({"args":["--pack","pack","--native","model.gguf"],"port":8080});
        let settings = Settings::from_config(&original)?;
        settings.validate()?;
        assert!(settings.values.iter().all(String::is_empty));
        let mut result = original.clone();
        settings.apply(&mut result)?;
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
    fn hub_settings_preserve_model_and_validate_overrides() -> Result<()> {
        let original = json!({"args":["--pack","pack","--native","model.gguf","--ple-gguf","model.gguf","--control-vector-layer-range","4","44","--max-context","131072","--vram-reserve-mib","1536","--prefill","auto","--expert-cache","auto","--kv","int8","--spec","4","--spec-min-p","0.5","--vision"],"vision":{"mmproj":"projection.gguf"},"sampling":{"temperature":1.0,"top_p":0.95,"top_k":20,"min_p":0.0,"repetition_penalty":1.0,"presence_penalty":0.0},"api_key":["fixture-key"],"env":{"KEEP":"yes"},"tokenizer":"tokenizer","port":8082});
        let mut settings = Settings::from_config(&original)?;
        settings.values[0] = "65536".into();
        settings.values[7] = "0.7".into();
        settings.adapt_async = true;
        settings.pin_by_need = true;
        settings.vision = false;
        settings.parallel = 2;
        settings.port = 8091;
        let settings =
            Settings::from_json(&settings.to_json(), &Settings::from_config(&original)?)?;
        let mut result = original.clone();
        settings.apply(&mut result)?;
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
