use crate::data::Result;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub const FIELDS: &[(&str, &str)] = &[
    ("strata_dir", "Strata installation folder"),
    ("python", "Strata Python executable"),
    ("models_dir", "Strata model data folder"),
    ("profiles_dir", "Model profiles folder"),
    ("data_dir", "Hub data folder"),
    ("pricing_file", "Pricing JSON file"),
];

#[derive(Clone, Debug)]
pub struct Paths {
    pub strata_dir: PathBuf,
    pub python: PathBuf,
    pub models_dir: PathBuf,
    pub profiles_dir: PathBuf,
    pub data_dir: PathBuf,
    pub pricing_file: PathBuf,
}
pub struct Loaded {
    pub file: PathBuf,
    pub paths: Paths,
}
static CONFIG: OnceLock<Loaded> = OnceLock::new();
pub fn loaded() -> Option<&'static Loaded> {
    CONFIG.get()
}
pub fn get() -> &'static Loaded {
    CONFIG.get().expect("Hub settings are not initialized")
}
pub fn set(file: PathBuf, paths: Paths) -> Result<()> {
    CONFIG
        .set(Loaded { file, paths })
        .map_err(|_| "Hub settings were already initialized".into())
}
pub fn default_file() -> Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or("LOCALAPPDATA is unavailable; use --config <file>")?;
    Ok(PathBuf::from(local).join("StrataHub").join("settings.json"))
}
pub fn resolve(base: &Path, value: &str) -> Result<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return Err("All path fields are required".into());
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Ok(path.into());
    }
    if path.has_root() || value.contains(':') {
        return Err("Use a full path or a path relative to the settings file".into());
    }
    Ok(base.join(path))
}
impl Paths {
    pub fn from_json(value: &Value, file: &Path) -> Result<Self> {
        if value["version"].as_u64() != Some(1) {
            return Err("Unsupported hub settings version".into());
        }
        let base = file.parent().ok_or("Invalid settings file location")?;
        let read = |name: &str| -> Result<PathBuf> {
            resolve(
                base,
                value[name]
                    .as_str()
                    .ok_or_else(|| format!("Missing setting: {name}"))?,
            )
        };
        Ok(Self {
            strata_dir: read("strata_dir")?,
            python: read("python")?,
            models_dir: read("models_dir")?,
            profiles_dir: read("profiles_dir")?,
            data_dir: read("data_dir")?,
            pricing_file: read("pricing_file")?,
        })
    }
    pub fn to_json(&self) -> Value {
        json!({"version":1,"strata_dir":self.strata_dir,"python":self.python,"models_dir":self.models_dir,
            "profiles_dir":self.profiles_dir,"data_dir":self.data_dir,"pricing_file":self.pricing_file})
    }
    pub fn values(&self) -> Vec<String> {
        let value = self.to_json();
        FIELDS
            .iter()
            .map(|(key, _)| value[key].as_str().expect("Paths must be UTF-8").to_owned())
            .collect()
    }
    pub fn validate(&self) -> Result<()> {
        for path in [
            self.strata_dir.join("serve/server.py"),
            self.strata_dir.join("setup.py"),
            self.python.clone(),
            self.strata_dir.join("engine/BUILD.json"),
        ] {
            if !path.is_file() {
                return Err(format!("Required file not found: {}", path.display()).into());
            }
        }
        for path in [&self.models_dir, &self.profiles_dir] {
            if !path.is_dir() {
                return Err(format!("Directory not found: {}", path.display()).into());
            }
        }
        if !self.pricing_file.is_file()
            && self.pricing_file != self.data_dir.join("qwen_pricing.json")
        {
            return Err("Choose an existing pricing JSON file, or use qwen_pricing.json in the hub data folder for initial setup".into());
        }
        Ok(())
    }
}
pub fn read(file: &Path) -> Result<Paths> {
    let bytes = fs::read(file)?;
    Paths::from_json(
        &serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))?,
        file,
    )
}
pub fn read_if_present(file: &Path) -> Result<Option<Paths>> {
    match read(file) {
        Ok(paths) => Ok(Some(paths)),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(None)
        }
        Err(error) => Err(format!(
            "Cannot read saved hub settings {}: {error}. The saved file has not been changed.",
            file.display()
        )
        .into()),
    }
}
pub fn save(file: &Path, paths: &Paths) -> Result<()> {
    paths.validate()?;
    fs::create_dir_all(file.parent().ok_or("Invalid settings file location")?)?;
    if file.exists() {
        fs::copy(file, file.with_extension("previous.json"))?;
    }
    let temporary = file.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(&paths.to_json())?)?;
    fs::rename(temporary, file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_settings_are_loaded_and_read_errors_do_not_trigger_first_run() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("strata-settings-state-{}", std::process::id()));
        fs::create_dir(&directory)?;
        let file = directory.join("settings.json");
        assert!(read_if_present(&file)?.is_none());
        let mut value = json!({"version":1});
        for (name, _) in FIELDS {
            value[name] = json!(format!("configured/{name}"));
        }
        fs::write(&file, serde_json::to_vec(&value)?)?;
        let expected = Paths::from_json(&value, &file)?;
        assert_eq!(
            read_if_present(&file)?.expect("Saved paths").to_json(),
            expected.to_json()
        );
        assert!(read_if_present(&directory).is_err());
        fs::write(&file, b"invalid JSON")?;
        assert!(read_if_present(&file).is_err());
        assert_eq!(fs::read(&file)?, b"invalid JSON");
        fs::remove_file(file)?;
        fs::remove_dir(directory)?;
        Ok(())
    }
    #[test]
    fn paths_resolve_from_the_settings_file_without_machine_defaults() -> Result<()> {
        let file = std::env::temp_dir()
            .join("portable-hub")
            .join("settings.json");
        let mut value = json!({"version":1});
        for (name, _) in FIELDS {
            value[name] = json!(format!("folder/{name}"));
        }
        let paths = Paths::from_json(&value, &file)?;
        assert_eq!(
            paths.strata_dir,
            file.parent().unwrap().join("folder/strata_dir")
        );
        assert_eq!(
            paths.profiles_dir,
            file.parent().unwrap().join("folder/profiles_dir")
        );
        assert_eq!(
            Paths::from_json(&paths.to_json(), &file)?.data_dir,
            paths.data_dir
        );
        value["python"] = json!("");
        assert!(Paths::from_json(&value, &file).is_err());
        Ok(())
    }
}
