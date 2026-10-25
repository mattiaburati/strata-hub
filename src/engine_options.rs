use crate::data::Result;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Qwen,
    DeepSeek,
}

impl Engine {
    pub fn from_config(config: &Value) -> Self {
        if config["arch"].as_str() == Some("deepseek41") {
            Self::DeepSeek
        } else {
            Self::Qwen
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Qwen => "Qwen",
            Self::DeepSeek => "DeepSeek",
        }
    }

    pub fn supports_field(self, index: usize) -> bool {
        self == Self::Qwen || index == 0 || (7..13).contains(&index)
    }
}

pub enum Kind {
    Integer(u64, u64),
    Number(f64, f64),
    Choice(&'static [&'static str]),
    Text,
    Gpus,
    Split,
    Flag,
}

pub struct OptionSpec {
    pub engine: Engine,
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
}

impl OptionSpec {
    pub fn choices(&self) -> &'static [&'static str] {
        match self.kind {
            Kind::Choice(values) => values,
            Kind::Flag => &["on"],
            _ => &[],
        }
    }

    pub fn validate(&self, value: &str) -> Result<()> {
        let value = value.trim();
        let valid = !value.contains(['\0', '\r', '\n'])
            && (value.is_empty()
                || match self.kind {
                    Kind::Integer(min, max) => {
                        value.parse::<u64>().is_ok_and(|n| (min..=max).contains(&n))
                    }
                    Kind::Number(min, max) => value
                        .parse::<f64>()
                        .is_ok_and(|n| n.is_finite() && (min..=max).contains(&n)),
                    Kind::Choice(values) => values.contains(&value),
                    Kind::Text => true,
                    Kind::Gpus => value.split(',').all(|n| n.parse::<u32>().is_ok()),
                    Kind::Split => {
                        value == "auto" || {
                            let values = value
                                .split(',')
                                .map(str::parse::<u32>)
                                .collect::<std::result::Result<Vec<_>, _>>();
                            values.is_ok_and(|v| {
                                !v.is_empty()
                                    && v[0] >= 2
                                    && v.iter().all(|n| *n < 48)
                                    && v.windows(2).all(|w| w[0] < w[1])
                            })
                        }
                    }
                    Kind::Flag => value == "on",
                });
        if !valid {
            return Err(format!("Invalid value for {} ({})", self.label, self.key).into());
        }
        Ok(())
    }
}

macro_rules! option {
    ($engine:ident, $key:literal, $label:literal, $kind:expr, $help:literal) => {
        OptionSpec {
            engine: Engine::$engine,
            key: $key,
            label: $label,
            kind: $kind,
            help: $help,
        }
    };
}

// Official references: docs/DETAILS.md and strata_cli/launch/params.py in
// https://github.com/Niko1221/Strata, checked against the installed engines' --help.
// Model files, protocol/debug modes and flags already owned by another tab stay out.
pub const OPTIONS: &[OptionSpec] = &[
    option!(
        Qwen,
        "--prompt-cache",
        "Prompt checkpoints",
        Kind::Integer(0, u32::MAX as u64),
        "Checkpoint count retained between requests. 0 disables prompt reuse; engine default: 6."
    ),
    option!(
        Qwen,
        "--prompt-cache-every",
        "Checkpoint interval (tokens)",
        Kind::Integer(0, u32::MAX as u64),
        "Save a checkpoint after this many fresh prompt tokens. 0 disables periodic checkpoints; default: 16384."
    ),
    option!(
        Qwen,
        "--prompt-cache-root",
        "System prompt checkpoint (tokens)",
        Kind::Integer(0, u32::MAX as u64),
        "Minimum system prompt length to retain a root checkpoint. 0 disables it."
    ),
    option!(
        Qwen,
        "--prompt-cache-tail",
        "Checkpoint near prompt end",
        Kind::Flag,
        "Save an extra checkpoint near the end of a prompt. Requires prompt checkpoints and a positive checkpoint interval."
    ),
    option!(
        Qwen,
        "--prompt-cache-spread",
        "Spread prompt checkpoints",
        Kind::Flag,
        "Spread checkpoints back through conversation history to improve reuse after earlier messages change."
    ),
    option!(
        Qwen,
        "--conversation-cache-mib",
        "Conversation RAM budget (MiB)",
        Kind::Integer(0, u32::MAX as u64),
        "RAM reserved for parked conversations. 0 disables parking; omission lets supported engines size it from free RAM."
    ),
    option!(
        Qwen,
        "--conversation-cache-slots",
        "Parked conversation slots",
        Kind::Integer(0, u32::MAX as u64),
        "Maximum parked conversations. Default: 4. Parking preserves alternating sessions and does not run them concurrently."
    ),
    option!(
        Qwen,
        "--conversation-cache-min-free-mib",
        "Conversation RAM floor (MiB)",
        Kind::Integer(0, u32::MAX as u64),
        "Free RAM floor for parking and restoring conversations. Default: 2560 MiB."
    ),
    option!(
        Qwen,
        "--conversation-cache-min-tokens",
        "Minimum parked length (tokens)",
        Kind::Integer(0, u32::MAX as u64),
        "Park conversations only after they reach this length. 0 allows any length."
    ),
    option!(
        Qwen,
        "--conversation-cache-park-gain",
        "Minimum parking growth (tokens)",
        Kind::Integer(0, u32::MAX as u64),
        "Minimum conversation growth before replacing its parked copy. 0 parks every turn."
    ),
    option!(
        Qwen,
        "--memory-limit-mib",
        "Engine RAM limit (MiB)",
        Kind::Integer(0, u32::MAX as u64),
        "Explicit RAM limit for cache planning when a container cannot report its limit. 0 means no explicit limit."
    ),
    option!(
        Qwen,
        "--kv-resident",
        "Custom GPU KV cells",
        Kind::Integer(0, i32::MAX as u64),
        "Custom streaming size: 0 or at least 20480 cells. Clear this before enabling the Engine tab's 32K streaming preset."
    ),
    option!(
        Qwen,
        "--ple-io",
        "PLE table storage",
        Kind::Choice(&["direct", "mmap", "ram"]),
        "Read the n-gram table directly from SSD, through mapped files, or keep the entire table in RAM. Default: direct."
    ),
    option!(
        Qwen,
        "--ple-row-cache",
        "PLE cached rows",
        Kind::Integer(0, u32::MAX as u64),
        "Fetched n-gram rows retained in RAM (about 90 bytes per row). Default: 1048576; 0 disables the cache."
    ),
    option!(
        Qwen,
        "--ple-inflight",
        "PLE outstanding SSD reads",
        Kind::Integer(1, u32::MAX as u64),
        "Maximum outstanding n-gram SSD reads. Default: 256."
    ),
    option!(
        Qwen,
        "--short-read",
        "Decode short prompts (tokens)",
        Kind::Integer(0, u32::MAX as u64),
        "Read up to this many fresh tokens through decode windows instead of batched prefill. Default: 64; 0 disables it."
    ),
    option!(
        Qwen,
        "--suffix-draft",
        "Prompt lookup match (tokens)",
        Kind::Integer(0, u32::MAX as u64),
        "Draft from repeated context suffixes of at least this length. Default: 3; 0 uses MTP only."
    ),
    option!(
        Qwen,
        "--mtp-max-t",
        "Maximum MTP window",
        Kind::Integer(0, 8),
        "Cap the MTP window while allowing longer prompt lookup windows. 0 uses the Engine tab's speculation window."
    ),
    option!(
        Qwen,
        "--lookup-chain",
        "Extra prompt lookup drafts",
        Kind::Integer(0, 8),
        "Append up to this many prompt lookup drafts after MTP proposals. Default: 0; the total window cannot exceed 8."
    ),
    option!(
        Qwen,
        "--lookup-chain-min",
        "Chained lookup match (tokens)",
        Kind::Integer(1, u32::MAX as u64),
        "Minimum suffix match for chained prompt lookup. Default: 3."
    ),
    option!(
        Qwen,
        "--mtp-hnorm",
        "MTP hidden normalization",
        Kind::Choice(&["pooled", "stream"]),
        "Normalize across all hidden streams (pooled, default), or separately per stream. This can change draft acceptance."
    ),
    option!(
        Qwen,
        "--mtp-q4",
        "4-bit MTP copies",
        Kind::Choice(&["all", "proj", "head"]),
        "Use 4-bit copies for the drafter's projections, head, or both. Omit to retain the engine's normal precision."
    ),
    option!(
        Qwen,
        "--adapt-min-gain",
        "Minimum expert replacement gain",
        Kind::Number(0.0, f64::MAX),
        "Replace a cached expert only when the new expert's routing count exceeds the old one by this factor. Default: 1.5."
    ),
    option!(
        Qwen,
        "--pool-tasks",
        "CPU expert task batches",
        Kind::Integer(0, 4096),
        "Tasks per CPU expert phase. 0 uses three tasks per participating thread; maximum: 4096."
    ),
    option!(
        Qwen,
        "--pool-affinity",
        "CPU worker affinity",
        Kind::Choice(&["all", "auto", "p-cores", "measure"]),
        "Choose physical cores, hybrid CPU scheduling, P-cores only, or a measured choice. Default: all."
    ),
    option!(
        Qwen,
        "--host-core",
        "Host thread core",
        Kind::Choice(&["first", "last", "sibling"]),
        "Pin the host loop to the first core, last core, or the first core's SMT sibling. Default: first."
    ),
    option!(
        Qwen,
        "--gpu",
        "Visible GPU indices",
        Kind::Gpus,
        "Comma-separated GPU indices, for example 0,1. Indices follow nvidia-smi/PCI ordering on CUDA."
    ),
    option!(
        Qwen,
        "--layer-split",
        "GPU layer boundaries",
        Kind::Split,
        "Split model layers across GPUs: auto or increasing boundaries such as 12,24. Requires one visible GPU per stage."
    ),
    option!(
        Qwen,
        "--pipeline-windows",
        "GPU pipeline mode",
        Kind::Integer(0, 2),
        "Layer split pipeline: 0 disables it, 1 overlaps short prompt reads, 2 overlaps decode windows too."
    ),
    option!(
        Qwen,
        "--trim-stage-weights",
        "Load only stage weights",
        Kind::Flag,
        "With an explicit layer split, load each GPU's own dense layers instead of the full set."
    ),
    option!(
        Qwen,
        "--expert-cache-device1",
        "GPU 1 expert cache",
        Kind::Text,
        "Expert slots on helper GPU 1: auto or a nonnegative integer. Use separately from layer split and peer GPU modes."
    ),
    option!(
        Qwen,
        "--expert-cache-device2",
        "GPU 2 expert cache",
        Kind::Text,
        "Expert slots on helper GPU 2: auto or a nonnegative integer."
    ),
    option!(
        Qwen,
        "--expert-cache-device3",
        "GPU 3 expert cache",
        Kind::Text,
        "Expert slots on helper GPU 3: auto or a nonnegative integer."
    ),
    option!(
        Qwen,
        "--expert-cache-remote-placement",
        "Helper GPU expert placement",
        Kind::Choice(&["stripe", "layer"]),
        "Distribute expert ranks across helper GPUs (stripe, default), or assign whole layers."
    ),
    option!(
        Qwen,
        "--rope-scaling",
        "RoPE context scaling",
        Kind::Choice(&["none", "linear", "yarn"]),
        "Context extension method. Omission keeps the model's metadata; changing scaling can affect long-context quality."
    ),
    option!(
        Qwen,
        "--rope-scale",
        "RoPE extension factor",
        Kind::Number(f64::MIN_POSITIVE, f64::MAX),
        "Extension factor for linear or YaRN scaling. 1 disables extension; omission keeps the model's factor."
    ),
    option!(
        Qwen,
        "--rope-freq-base",
        "RoPE frequency base",
        Kind::Number(0.0, f64::MAX),
        "Rotary frequency base. 0 uses the model's base."
    ),
    option!(
        Qwen,
        "--yarn-orig-ctx",
        "YaRN trained context (tokens)",
        Kind::Integer(0, u32::MAX as u64),
        "Original trained context for YaRN correction. 0 uses the engine's trained-context default."
    ),
    option!(
        Qwen,
        "--expert-profile",
        "Expert placement profile",
        Kind::Text,
        "Path to the hot-expert profile used to populate the GPU cache. Keep the existing model profile unless changing placement deliberately."
    ),
    option!(
        DeepSeek,
        "--experts",
        "Routed experts (quality / speed)",
        Kind::Choice(&["auto", "1", "2", "3", "4", "5", "6"]),
        "DeepSeek 0.1.50+: 6 keeps full quality, 5 is the Faster mode. auto uses 5 on GPUs with up to 16 GiB and 6 on larger GPUs. Lower counts reduce quality."
    ),
    option!(
        DeepSeek,
        "--dspark",
        "DSpark drafting",
        Kind::Choice(&["auto", "on", "off", "none"]),
        "Choose whether to use the DSpark drafter. auto loads it only when the engine expects a speed benefit."
    ),
    option!(
        DeepSeek,
        "--dspark-conf",
        "DSpark confidence",
        Kind::Number(0.0, 1.0),
        "Minimum confidence for accepting DSpark proposals, from 0 to 1. The prepared profile uses 0.65."
    ),
    option!(
        DeepSeek,
        "--dspark-block",
        "DSpark draft block",
        Kind::Integer(0, i32::MAX as u64),
        "Maximum tokens drafted per round. 0 retains the engine's block choice; shorter blocks can help when acceptance is low."
    ),
    option!(
        DeepSeek,
        "--ssd-cache",
        "Expert RAM cache (GB)",
        Kind::Number(0.0, f64::MAX),
        "RAM used to keep experts instead of reading them from SSD again. Omission lets the engine size this cache."
    ),
    option!(
        DeepSeek,
        "--ssd-policy",
        "Expert RAM eviction policy",
        Kind::Choice(&["lru", "lfu", "lfulru"]),
        "Evict the least recently used, least frequently used, or a combined choice of cached experts."
    ),
    option!(
        DeepSeek,
        "--prefill-chunk",
        "Prompt chunk (tokens)",
        Kind::Integer(0, i32::MAX as u64),
        "Tokens processed together through the fast prompt path. 0 disables chunked prompt processing."
    ),
    option!(
        DeepSeek,
        "--threads",
        "Threaded prompt processing",
        Kind::Choice(&["on", "off"]),
        "Read long prompts layer by layer using CPU threads."
    ),
    option!(
        DeepSeek,
        "--cpu-threads",
        "CPU expert threads",
        Kind::Integer(0, i32::MAX as u64),
        "CPU threads used for experts that do not reside on the GPU. 0 uses the engine's automatic thread count."
    ),
    option!(
        DeepSeek,
        "--tp",
        "Tensor parallel GPU count",
        Kind::Integer(1, i32::MAX as u64),
        "Share every layer across this many graphics cards. Must match the GPUs selected for this run."
    ),
    option!(
        DeepSeek,
        "--gpus",
        "GPU indices",
        Kind::Gpus,
        "Comma-separated GPU indices used together, for example 0 or 0,1."
    ),
    option!(
        DeepSeek,
        "--reserve-mib",
        "GPU VRAM reserve (MiB)",
        Kind::Integer(0, u32::MAX as u64),
        "GPU memory to leave free for Windows and other applications."
    ),
    option!(
        DeepSeek,
        "--vram-experts",
        "GPU expert memory fraction",
        Kind::Number(0.0, 1.0),
        "Fraction of available GPU memory used for experts, from 0 to 1. Omission retains the engine's allocation."
    ),
    option!(
        DeepSeek,
        "--mmap-experts",
        "Mapped expert reads",
        Kind::Choice(&["auto", "on", "off"]),
        "Read experts directly from mapped model files. auto lets the engine select its storage path."
    ),
    option!(
        DeepSeek,
        "--no-expert-cache",
        "Disable GPU expert cache",
        Kind::Flag,
        "Disable retention of hot experts in GPU memory. Clear the value to retain the engine default."
    ),
    option!(
        DeepSeek,
        "--expert-profile",
        "Expert placement profile",
        Kind::Text,
        "Hot-expert profile file, or none to disable the profile. Determines which experts are retained first."
    ),
    option!(
        DeepSeek,
        "--verbose",
        "Verbose engine log",
        Kind::Flag,
        "Print additional engine details in the model log."
    ),
];

pub fn managed(key: &str) -> bool {
    OPTIONS.iter().any(|option| option.key == key)
}

pub fn extract(engine: Engine, args: &mut Vec<String>) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for option in OPTIONS.iter().filter(|o| o.engine == engine) {
        let mut index = 0;
        while index < args.len() {
            let token = &args[index];
            if token != option.key
                && !token
                    .split_once('=')
                    .is_some_and(|(key, _)| key == option.key)
            {
                index += 1;
                continue;
            }
            if values.contains_key(option.key) {
                return Err(format!("Duplicate engine option {}", option.key).into());
            }
            let (value, count) = if matches!(option.kind, Kind::Flag) {
                if token != option.key {
                    return Err(format!("{} does not take a value", option.key).into());
                }
                ("on".to_owned(), 1)
            } else if let Some((_, value)) = token.split_once('=') {
                (value.to_owned(), 1)
            } else {
                (
                    args.get(index + 1)
                        .filter(|v| !v.starts_with("--"))
                        .ok_or_else(|| format!("Missing value for {}", option.key))?
                        .clone(),
                    2,
                )
            };
            if value.is_empty() {
                return Err(format!("Missing value for {}", option.key).into());
            }
            option.validate(&value)?;
            values.insert(option.key.to_owned(), value);
            args.drain(index..index + count);
        }
    }
    Ok(values)
}

pub fn validate(engine: Engine, values: &BTreeMap<String, String>) -> Result<()> {
    for (key, value) in values {
        let value = value.trim();
        let option = OPTIONS
            .iter()
            .find(|o| o.engine == engine && o.key == key)
            .ok_or_else(|| format!("Unsupported {} option: {key}", engine.name()))?;
        option.validate(value)?;
        if key.starts_with("--expert-cache-device")
            && !value.trim().is_empty()
            && value != "auto"
            && value.parse::<u32>().is_err()
        {
            return Err(format!("{key} must be auto or a nonnegative integer").into());
        }
        if key == "--kv-resident" && value.parse::<u32>().is_ok_and(|v| v > 0 && v < 20480) {
            return Err("Custom GPU KV cells must be 0 or at least 20480".into());
        }
    }
    Ok(())
}

pub fn append(engine: Engine, values: &BTreeMap<String, String>, args: &mut Vec<String>) {
    for option in OPTIONS.iter().filter(|o| o.engine == engine) {
        if let Some(value) = values
            .get(option.key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            args.push(option.key.to_owned());
            if !matches!(option.kind, Kind::Flag) {
                args.push(value.to_owned());
            }
        }
    }
}
