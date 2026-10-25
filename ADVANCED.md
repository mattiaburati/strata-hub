# Engine-specific advanced settings

The Advanced tab follows the selected model profile's `arch`: `deepseek41`
selects DeepSeek; existing Qwen profiles retain the Qwen controls. Choose a
parameter to edit its value and read its description. Values remain in the form
while choosing other parameters; Save changes or Start persists the settings.
Blank values omit the override and use the engine default.

The catalogue contains 39 Qwen options and 17 DeepSeek options, in addition to
the existing Engine, Performance and Sampling controls:

- Qwen: prompt checkpoints, parked conversations, RAM/KV budgets, PLE I/O,
  prompt lookup and MTP, CPU scheduling, multiple GPUs and RoPE scaling.
- DeepSeek: routed expert count, DSpark, expert RAM cache and eviction, prompt chunks, CPU threads,
  tensor parallelism, GPU selection/reserve and mapped expert storage.

DeepSeek hides Qwen-only controls and calibration. Its server currently accepts
one request at a time without vision. The experimental start button is visible
only when the selected profile contains a nonblank `hub_experimental.exe`.
Launch still validates that the configured executable exists.

Named advanced settings are stored by CLI option name in the `advanced` object
of each saved Hub profile. Existing values are migrated from `extra_args` when
loaded; values already in a base profile are also imported. The migration does
not write profile files or change settings on disk until the normal Save/Start
flow. Model paths and other additional arguments remain in the JSON editor.
Named options cannot also appear there. Existing 32K KV streaming presets keep
their Engine checkbox; custom KV sizes use Advanced and are checked for
conflicts with both that preset and Elastic KV.

## Official sources

Checked on 2026-10-25:

- [Strata technical settings](https://github.com/Niko1221/Strata/blob/main/docs/DETAILS.md)
- [DeepSeek engine documentation](https://github.com/Niko1221/Strata/blob/main/docs/DEEPSEEK41.md)
- [Official Advanced parameter descriptions](https://github.com/Niko1221/Strata/blob/main/strata_cli/launch/params.py)

Both official 0.1.50 engine packages were checked against their own help.
DeepSeek's `--experts` chooses the routed expert count: 6 keeps full quality,
5 chooses Faster, and auto lets the engine choose for the hardware (5 on GPUs
with up to 16 GiB). When upgrading from 0.1.49.2, explicitly selecting 6
preserves the previous expert count; omitting the override lets 0.1.50 choose.

Qwen's Adapt async selector now supports Auto, On and Off. Auto omits the CLI
override so 0.1.50 can measure and choose its asynchronous tier. Previously saved
boolean choices retain their explicit On/Off behavior; no saved tuning is reset.
The catalogue covers normal tuning, not
one-shot generation, benchmark, diagnostic or model-conversion commands.

An experimental Qwen engine is a separately configured executable. The model's
settings are shared between standard and experimental starts. Option support
depends on that executable's version; the catalogue is not a per-version
capability inventory.
