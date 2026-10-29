# Strata Hub

A lightweight Windows desktop hub for [Strata](https://github.com/Niko1221/Strata). Install Strata or use an existing installation, start and stop local models, edit per-model settings, track usage and cost estimates, check stable updates, and connect coding agents. The interface is written in Rust with native Win32/GDI controls and SQLite. It needs no browser, WebView or permanent Python collector.

## Requirements

- Windows 10 or 11, x64.
- Hardware and a graphics driver supported by Strata. Its official setup checks compatibility.
- The release ZIP includes the MSVC runtime statically; Rust is only needed to build from source.
- Git for installing or updating Strata. Node.js 22.19+ with npm only if you want to install missing coding agents.
- Agent integration requires Python 3.11+ in the configured Strata environment (`tomllib` is part of the standard library from Python 3.11).

Use an existing Strata installation with its Python environment and engine, or install a fresh copy from Settings. Running inference requires a prepared model profile; the library and usage report also work without profiles. Existing model quantization and paths remain those of your profiles.

## First run

Extract the Windows release ZIP and open `StrataDashboard.exe` or `Start-StrataHub.bat`. Settings opens automatically only for a first run with no saved default configuration. Choose:

| Setting | What to select |
| --- | --- |
| Strata installation folder | The folder containing `setup.py`, `serve` and `engine` |
| Strata Python executable | Usually `.venv/Scripts/python.exe` inside Strata |
| Strata model data folder | The `Strata-data` folder containing models and packed weights |
| Model profiles folder | The folder for your `strata-*.json` model configurations; it may be empty |
| Hub data folder | Your chosen location for usage history, profile drafts, logs and backups |
| Pricing JSON file | An existing pricing file, or `qwen_pricing.json` in the hub data folder for the bundled dated snapshot |

Enter the Strata installation folder and click **Use Strata defaults** to fill the usual Python, model-data and profiles paths. Adjust any custom locations, then click **Save settings**. Opening the hub never starts a model automatically.

### Install Strata when missing

Choose a new or empty installation folder and a separate model data folder, then use **Install Strata** in Settings. Select Qwen's recommended size, a specific Qwen size, Swift's recommended size or Coder's recommended size. Recommended sizes are selected by upstream setup for the detected hardware. These are the official model families; third-party abliterated models can be used through existing profiles.

The installer clones the latest published stable release from the official Git repository. It detects a supported 64-bit Python 3.10+ or installs Python 3.12 for the current Windows user from python.org, checking its Authenticode signature first. It creates Strata's private `.venv` and runs the official `setup.py --yes --no-start --no-browser --host 127.0.0.1 --data-dir ... --vision no`. A random local API key is generated for the fresh installation and passed with `--api-key`, so agent connectors work without manually editing the new profile. Upstream setup installs the engine, dependencies and selected model. Downloads can be about 70-120 GB and setup may install build tools when the hardware needs a local engine build.

Settings displays progress stages and elapsed time; **Open install log** opens detailed output in `strata-install.log` beside the hub settings JSON. The hub adds only one temporary worker and PowerShell process during installation, with no additional resident process afterwards. Closing Settings and saving are blocked while installation is running. On success, Python and profiles paths are filled from the new installation; click **Save settings** to open the hub. The model stays stopped. Enable images later through Strata's setup if needed.

Existing or partially populated installation folders are refused, including hidden files. Errors are shown explicitly; no automatic retries, cleanup, driver installation or rollback is added by the hub. After an interrupted installation, inspect its log and finish setup manually or choose a different empty folder. Model data can live on another drive. A custom Strata installation is never replaced by this installer.

By default settings are stored in `%LOCALAPPDATA%/StrataHub/settings.json`, and a new user's hub data lives in its `data` subfolder. Existing histories and pricing files are preserved. New histories start empty. The bundled pricing and EUR/USD values are dated snapshots, not live quotes; replace the configured pricing JSON when you want to update them.

Subsequent launches read the saved paths. Unreadable or invalid settings and unavailable drives produce an explicit error, preserving the configuration. They do not open a fresh setup or start installation. A launcher using `--config <file>` requires that exact file to exist; use `--settings --config <file>` to create it intentionally. Pin custom Windows shortcuts and startup entries to the same absolute settings filename with `--config` so they use the same configuration regardless of the working directory or launcher environment.

Use **Settings** to edit the paths later, while Strata is stopped and no operation is running. Saving restarts only the hub. A previous settings copy is retained beside the JSON file. `Open-Settings.bat` or `StrataDashboard.exe --settings` can edit paths even when the old installation was moved and the main window cannot open.

## Portable configuration

To keep settings beside the executable, copy `hub-settings.example.json` to `hub-settings.json`, edit it and launch:

```powershell
.\StrataDashboard.exe --config .\hub-settings.json
```

Relative paths resolve from the settings file's folder, not from the shell's working directory. The example assumes sibling `Strata` and `Strata-data` folders and keeps hub data in `./data`. Use absolute paths for any other layout. No drive letters, usernames or developer-machine locations are compiled into the app.

Moving the hub does not rewrite paths inside third-party Strata profiles. Those profiles must already point to valid engine, tokenizer, projector and weight files on the destination PC. No personal profiles or credentials are included in the release or source packages.

## Models

Choose a discovered profile under **Models**, edit its settings, then save or start it. Profile filenames may differ between PCs. Empty numeric or sampling fields preserve unset Strata defaults. Strata metadata files without a model definition are skipped.

**GPUs** supports single-card and multi-card configurations for Qwen and DeepSeek. **Use source profile** preserves the profile's GPU selection, roles and ordering, including older saved drafts. **Choose GPUs** accepts device indices such as `0` or `0,1`; **Detect GPUs** lists NVIDIA names/memory through `nvidia-smi`, or AMD HIP devices through the installed engine's `strata-device.exe`. Click rows to toggle cards, or edit indices directly. Detection runs only when requested, without starting inference; errors are shown explicitly. GPU indices are zero based and backend specific. Cards from different backends cannot be combined.

**Automatic (Strata)** uses the server's GPU ordering and multi-GPU split. **Keep listed order** preserves the entered stage order; the last card runs the final stage. Selection is saved per model and applied to runtime copies on the next launch. It does not change source profiles. Explicit choices replace source GPU roles and HIP ordinal metadata. Remove GPU visibility environment overrides and clear **Visible GPU indices** in Advanced before using an explicit selection. Fixed layer boundaries must match the selected GPU count; use `auto` in Advanced to let Strata partition layers. GPU choices do not verify that an engine build supports every selected card. Detection errors are recorded in `strata-gpus.log`.

The shared UI groups workspace navigation separately from tools, uses consistent panel/input colors and selected-tab indicators, and distinguishes disabled controls. Engine settings provide more space between section titles, field labels and memory switches.

**Advanced** adapts to the profile's engine family, with 39 Qwen options and 17 DeepSeek options. Select a parameter to read its description and edit its value. Existing raw arguments migrate into named controls when loaded. DeepSeek 0.1.50 adds `--experts`: 6 keeps full quality, 5 chooses Faster, and auto lets the engine choose for the hardware. Qwen's **Adapt async** supports Auto, On and Off while preserving saved boolean choices. See [ADVANCED.md](ADVANCED.md) for supported options and migration behavior.

**Start standard** uses the profile's normal engine. **Start experimental** appears only when that profile has a nonblank `hub_experimental.exe`; both starts share the model's saved settings. The optional runtime object can override executable, working directory, libraries, server and environment without creating a second model entry.

Profiles without an experimental runtime show a single **Start** button and only the standard engine/server environment scopes. Removing `hub_experimental` retires that profile's experimental integration without deleting engine files or changing its standard settings.

**Environment** edits per-model variables for the standard/experimental engine and Python server. Enter one `NAME=value` per line, without shell quoting or `set`/`export`. Values can contain `=` and spaces; `NAME=` sets an empty value. Removing a line removes that profile override, so any inherited process variable still applies. Experimental environments replace their standard counterpart; enable **Inherit standard environment** to use the standard variables instead. An unchecked inheritance box with an empty editor supplies an empty override object. Existing profile environments and older saved settings are preserved when loaded. **Save changes** stores edits with the model settings; **Reset** reloads the source profile. Changes take effect at the next model launch and do not edit Windows user/system variables. Values are stored as plain text in local profile drafts and runtime JSON files.

For Qwen, `STRATA_PIN_BY_NEED` and `STRATA_KV_GROW` remain controlled by the Engine switches and cannot be entered again in engine environments. KV streaming forces `STRATA_VRAM_BUDGET=0` while enabled. Invalid names, duplicate names (case insensitive on Windows), NUL characters and multiline values are rejected explicitly.

Hover **Available variables (?)** for help, or click it for the complete searchable reference for the selected engine/server scope. The reference contains variable names and source locations from the 27 October 2026 standard and custom experimental source snapshots, including DeepSeek, diagnostics and backend-specific switches. It is a source reference, not runtime feature detection: installed engine/server versions and hardware backends may support different switches. Search matches names and source paths; the read-only results can be selected and copied. Qwen's Engine-managed variables are listed for reference but remain controlled by their switches. The reference is bundled in the executable and requires no network access or installed source tree. Refresh `assets/environment-variables.tsv` from the corresponding production source trees when updating these snapshots.

Qwen also has **Performance** controls and an explicit **Calibration** action. Calibration requires a stopped model and saved settings, runs upstream hardware measurements and saves tuning only after success. **Elastic KV cache** and **KV streaming (32K GPU, more RAM)** are mutually exclusive. DeepSeek hides these Qwen controls and currently supports one text request at a time.

**Context** offers 4K, 8K, 16K, 32K, 64K, 96K, 128K, 192K and 256K presets (K = 1024 tokens). The adjacent numeric field remains editable; typing another token count selects **Custom**, while entering a preset's exact count selects its label. Choosing **Custom** focuses the numeric field without replacing its current value. Context edits follow the same Save/next-launch behavior as other settings. The Engine page groups memory, generation and server settings separately; native controls retain keyboard and accessibility behavior.

Server port and 1–8 concurrent requests are saved per profile. The launcher, authenticated metrics collector and agent connectors use the selected port. More request slots consume VRAM and may reduce individual request speed. The hub displays the number of slots Strata actually allocated. Save or start to synchronize existing local agent connections after a port change; update external reverse proxy forwarding separately if it points to the old port.

Engine and sampling edits use runtime copies under the hub data folder's `profiles-run` directory. Model identity, credentials and tokenizer configuration are retained. Library moves update source paths, and library deletions remove the explicitly listed profiles after confirmation and backup. The hub keeps the server bound to `127.0.0.1`. A dedicated log is assigned to runtime profiles that do not already configure one.

Startup shows observed loading stages, elapsed time, readiness and process errors. **Startup details** includes bounded excerpts from the current launch only. **Stop** terminates only a server launched by this hub, identified by PID and creation time. Other servers remain controlled by their original launcher.

Closing the window hides it to the notification area. Double-click the tray icon to reopen it. **Exit hub** leaves an owned Strata server running; ownership is retained across hub restarts when using the same data folder.

## Model library

**Model library** discovers GGUF models and prepares them as runnable Strata profiles. Paste a Hugging Face model repository URL or `owner/repository`, then click **Analyze repo**. The hub lists quantization recipes, total download sizes and complete shard sets. Projectors and draft-only files are excluded.

Compatibility is determined by the GGUF headers through the inspector and fingerprints in the installed Strata version. A known layout can be downloaded; a fine-tune with matching experts is clearly marked as an untested fine-tune. Incompatible, untested or failed inspections cannot be downloaded through this screen. A header verdict does not prove generation quality or that every hardware backend supports the layout. Changing Strata requires a new repository analysis before downloading. Analysis reads bounded HTTP byte ranges, with a 64 MiB header budget per variant, rather than downloading the weights.

Choose an existing **Download directory**, select a variant and click **Download & prepare** with Strata stopped. Every shard is pinned to the scanned repository commit and checked against Hugging Face's SHA-256. Downloads stream through a 1 MiB buffer into `.part` files; interrupted transfers resume when the user clicks the same action again. There are no automatic retries. The hub then inspects the local headers, runs the installed `iq_pack.py --compat-bf16`, exports that GGUF's own tokenizer and creates a profile under the configured profiles directory. **Prepare model** runs the same preparation for completed downloads and unconfigured local GGUF folders. Already configured models keep their profiles.

Preparation currently supports the installed inspector's known or matching `qwen4exp` layouts. Every model gets a dedicated `packs/hub-<id>` folder; dense weights and tokenizers are never borrowed from another fine-tune. Experts remain in the GGUF, avoiding another full expert-weight copy. The official setup supplies its pinned gguf-py dependency when needed. The common Qwen MTP draft layer is verified and reused; first setup fetches about 5 GB of tensors through Strata's official MTP tools, then packs them. Shared runtime files are checked for completeness. A failed preparation publishes no new runnable profile; logs and partial outputs remain for inspection or an explicit preparation action.

New profiles inherit the selected profile's local port, API key, sampling, GPU selection and supported resource settings. Model-specific steering vectors and image assets are excluded. Without an existing profile, the hub uses localhost port 8080, 32K context, int8 KV and automatic prefill/expert cache, with the installed engine's library paths. Settings can then be edited in **Models**. The completed profile appears there automatically; select it and click **Start**. Preparation does not start inference or switch the active selection.

Private/gated GGUF repositories use `HF_TOKEN` or the token saved by `hf auth login`; tokens are never written to the model catalog or logs. The hub uses the Hub HTTPS API and the existing Strata Python environment, so installing the `hf` CLI is optional. MTP downloads use the installed official Strata tools and their configured source.

**Scan local** lists library downloads, GGUF folders under the configured model data folder's `models` directory, and native-model folders referenced by configured profiles. Choose an existing **Move to directory** and click **Move**. Strata must be stopped. The hub checks the selected folder, destination, process ownership and configured ports, then presents the exact file paths and affected profiles for confirmation. Same-volume moves use a rename; cross-volume moves copy with bounded buffers, verify the copies, update profile references and only then remove source files. Existing destinations are never overwritten. Absolute paths in source/runtime profiles, saved hub drafts and referenced pack metadata are updated, with original JSON files backed up under `model-backups` in the hub data folder. Launchers or raw command strings containing direct model paths require manual adjustment first.

**Delete** requires Strata to be stopped and confirmation of the complete file list, including source/runtime profiles and dedicated model launchers that will be removed. Their original files are backed up, and saved hub drafts/selection are updated. GGUF deletion is permanent and does not use the Recycle Bin. External packs and shared assets outside the selected folder are retained. The library and usage report remain usable after deleting the last profile; model and agent controls require a prepared profile. **Scan local** reloads newly prepared profiles when the hub is empty. Symlinks, junctions, protected hub/Strata/profile directories, oversized file lists and changed confirmation plans are refused. Backups and interrupted move copies are retained for manual review after an error; the hub adds no automatic cleanup or rollback.

Catalog, library and inventory metadata live in the hub data folder; transfer, preparation progress and errors are recorded in `strata-models.log`. The preparation template contains local profile credentials and stays in that private data folder. Catalog and file scans run only when requested. The Python worker exits after each operation, with no permanent download service or browser. The sidebar, tray and executable use the embedded multi-size Strata Hub icon; the original transparent PNG and generation prompt are in `assets`.

## Usage

The report includes daily totals, model totals, recent requests, historical log-import records and CSV export. Generation speed appears beside model status. The header also shows **RAM** (combined working set of the selected Strata server and its engine/child processes) and **VRAM** (their dedicated GPU memory across adapters), in GiB. The server is identified by the configured localhost port; an owned server still loading is identified by its checked process record. These are process readings, not whole-PC usage or Windows commit size. Shared pages can be counted in more than one process working set; mapped files held only in the OS cache are not included. The footer's **Hub RAM** remains separate.

Memory readings use native Windows process APIs and the language-neutral `GPU Process Memory` performance counter, within the existing worker. No helper process, inference request or extra polling thread is started. GPU counters are opened only while a Strata process exists. Missing or unsupported readings display **N/A**, with the error in the footer; no device-total value is substituted. Dedicated GPU readings depend on the Windows WDDM driver/counters and can differ from physical adapter totals. See [Microsoft's per-process GPU memory explanation](https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/) and [GPU counter limitations](https://learn.microsoft.com/en-us/troubleshoot/windows-client/performance/gpu-process-memory-counters-report-wrong-value).

Live metrics refresh every second while visible, with memory readings sampled every 2 seconds. Full history collection and SQLite updates occur every 15 seconds; hidden or minimized windows poll every 15 seconds. Memory is live only and is not stored in the usage database. No prompts or responses are stored in the usage database.

Model pricing and EUR/USD conversion come from the configured pricing JSON and are displayed to two decimal places. Cost figures estimate the equivalent hosted API usage. Historical log imports from an existing data folder are preserved; undated segments are not assigned invented dates. Importing old external logs is not automated by this release.

## Updates

**Check for updates** checks the official stable source release and the signed standard (Qwen) engine feed only when clicked. Their versions are compared independently. **Install update** requires an available update and a stopped model. It verifies the official Git origin and a clean tracked working tree, backs up the engine and profiles, and fast-forwards the checkout when the source is newer. Python dependencies follow official setup's selection. Engine packages use the official hardware selector, signatures and file hashes, with a binary-only version/DLL check. The verified engine is also installed into the legacy `engine` folder used by existing profiles. See [UPDATES.md](UPDATES.md) for details.

DeepSeek uses its own engine package and configured executable path. Its package updates are handled separately through the official Strata tools; the Hub update button currently manages the standard engine.

The updater checks the installed version, protected profile hashes and model-weight metadata afterwards. Backups live under `update-backups` in the hub data folder and contain manual restore instructions. There are no automatic retries, forced source resets or automatic rollback. Python package versions are not included in the engine backup. Upstream changes to protected source profiles can require manual review. See `strata-update.log` for progress and errors.

## Agents

**Scan agents** detects Pi, OpenCode and Codex CLI without launching them. Missing clients can be installed into the hub data folder under `agents/packages`. Existing installations are retained. Pi uses its official script-free npm install; the OpenCode connector and installer target v1.

**Connect to Strata** backs up existing client configuration and adds or updates one generic `strata-local/strata` connection for the selected port, context and vision support. Configure an API key in the Strata profile before connecting agents. Pi and OpenCode retain unrelated providers and settings. JSONC files containing comments and OpenCode v2 configurations fail clearly rather than being rewritten incorrectly.

**Save changes** and **Start model** automatically synchronize port, image input support and explicit Context to existing generic `strata-local/strata` connections. A local connection follows the selected port even when it previously pointed to another localhost port. Remote endpoints and unrelated providers are left alone; missing connections are not created. Unsaved edits do not modify client files. An empty Context leaves client context unchanged while port and image support still synchronize. An existing output limit is clamped only if it exceeds the new context. Credentials and unrelated settings are retained, with backups before writes. An already running client may need to reload its configuration; the engine uses saved settings on its next launch.

Codex uses a dedicated `strata-hub.models.json` catalog referenced only by its Strata Hub profile. Synchronization preserves existing model instructions and other catalog entries. If no Strata model metadata exists, the hub creates metadata with basic coding instructions and explicit text/image modalities. An existing external catalog is copied into the dedicated catalog instead of being edited in place. The main Codex configuration is unchanged. The supported catalog option is documented in the [official Codex configuration reference](https://developers.openai.com/codex/config-reference/).

**Scan agents** remains read-only. Agents displays detected context and image support, with **Update needed** for a mismatch. **Update connection** can still apply the full selected connection settings. Synchronization reuses the short-lived agent helper, waits for another agent job or engine update to finish, and reports errors without automatic retries.

Codex uses a separate `strata-hub.config.toml` profile and a launcher that reads the local API key into its process environment. The main Codex configuration is unchanged. **Launch** opens an interactive terminal only when clicked. Helpers exit after each requested operation; they add no permanent service. Logs are in `strata-agents.log`.

## Build and package

Install stable Rust with the `x86_64-pc-windows-msvc` toolchain, the Microsoft C++ Build Tools (Desktop development with C++) and Windows SDK (including `rc.exe`). The packaging script additionally requires PowerShell 7 (`pwsh`). From the source folder:

```powershell
cargo build --release --locked
.\target\release\strata-dashboard.exe
```

The executable built directly by Cargo is for local development. To prepare a distributable Windows executable with the static MSVC runtime, remapped build paths, a source ZIP and dependency notices, run in PowerShell 7:

```powershell
pwsh -NoProfile -File .\Build-Package.ps1
```

The script creates a new timestamped folder under `dist`, containing `strata-hub-source/`, `strata-hub-windows-x64/`, their ZIPs and `checksums.json`. It copies only explicitly listed source files and includes third-party license notices. It excludes backups, databases, credentials, local settings, deployment records, debug symbols and build caches. It checks both package folders for recognizable provider tokens, private keys and personal build paths before creating their ZIPs, reporting filenames without printing matching values. These checks complement manual review; they cannot identify every possible secret. `-SkipBuild` packages an existing release executable and performs the same checks; the executable must already have been built with distributable settings.

`--check --config <file>` validates configured paths, the database and every discovered model profile without starting inference. It initializes a new data folder if needed and writes sanitized `native-check.json` and `hub-check.json` reports there. Settings failures are explicit; `--settings` remains available to edit incorrect paths. `--report-only` opens usage; `--background` starts hidden and requires an existing configuration.

Focused Rust checks cover relative path resolution, fresh-data initialization, existing-data preservation, sparse and saved profiles, custom-port metrics and startup diagnostics. Focused Python checks under `tests/test_tools.py` exercise updater, agent configuration and installer preflight logic using temporary directories, without real model downloads or agent installations. `scripts/install_strata.ps1 -Root <path> -Data <path> -CheckOnly` validates a fresh-install plan without network access or filesystem writes.

For a focused agent-configuration check from the source folder, use the configured Python 3.11+ interpreter:

```powershell
python -m unittest discover -s tests -p test_tools.py -k context_sync -k agent_settings_sync -k test_connections_preserve
```

## Publishing

Use the generated `strata-hub-source` folder as the Git repository root, and attach the Windows ZIP to a GitHub release. Keep local runtime data out of the repository. [PUBLISHING.md](PUBLISHING.md) describes the package contents and the review steps before the first push. Test credentials named `fixture`, `fixture-key` and `fixture-only` are deliberate nonfunctional test values.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE). Bundled dependency notices are included in both packages as THIRD_PARTY_NOTICES.txt. Strata and coding agents are separate projects with their own licenses.
