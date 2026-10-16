"""On-demand agent discovery, installation and Strata connection setup."""
import argparse
import datetime
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

CATALOG = {
    "pi": ("Pi", "@earendil-works/pi-coding-agent"),
    "opencode": ("OpenCode", "opencode-ai@1"),
    "codex": ("Codex CLI", "@openai/codex"),
}


def emit(kind, **fields):
    print("HUB_EVENT " + json.dumps({"kind": kind, **fields}), flush=True)


def read_json(path):
    # Existing JSONC files that use comments fail clearly rather than losing those comments.
    return json.loads(path.read_text(encoding="utf-8-sig")) if path.exists() else {}


def save_json(path, value, backup):
    save_text(path, json.dumps(value, indent=2) + "\n", backup)


def save_text(path, text, backup):
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        backup.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, backup / path.name)
    temporary = path.with_suffix(path.suffix + ".hub-tmp")
    temporary.write_text(text, encoding="utf-8", newline="")
    os.replace(temporary, path)


def find_agent(name, report, home):
    candidates = [report / "agents" / "packages" / name / (name + ".cmd")]
    if name == "pi":
        candidates += [home / ".pi" / "agent" / "bin" / "pi.cmd", home / ".pi" / "agent" / "bin" / "pi.ps1"]
    if name == "codex":
        candidates += [home / "AppData" / "Local" / "Programs" / "OpenAI" / "Codex" / "bin" / "codex.exe"]
    candidates += [Path(found) for extension in [".exe", ".cmd", ".ps1"] if (found := shutil.which(name + extension))]
    return next((path for path in candidates if path.is_file()), None)


def config_path(name, home):
    if name == "pi":
        return Path(os.environ.get("PI_CODING_AGENT_DIR", str(home / ".pi" / "agent"))) / "models.json"
    if name == "opencode":
        directory = Path(os.environ.get("XDG_CONFIG_HOME", str(home / ".config"))) / "opencode"
        paths = [directory / "opencode.json", directory / "opencode.jsonc"]
        existing = [path for path in paths if path.exists()]
        if len(existing) > 1:
            raise RuntimeError("Both opencode.json and opencode.jsonc exist. Resolve the config precedence before connecting.")
        return existing[0] if existing else paths[0]
    return Path(os.environ.get("CODEX_HOME", str(home / ".codex"))) / "strata-hub.config.toml"


def local_endpoint(value):
    match = re.fullmatch(r"http://(?:127\.0\.0\.1|localhost|\[::1\]):([0-9]+)/v1/?", value or "")
    return match is not None and 1 <= int(match[1]) <= 65535


def toml_setting(source, table, key, value):
    headers = list(re.finditer(r"(?m)^[ \t]*\[[^\r\n]+\][^\r\n]*(?:\r?\n|$)", source))
    start, end = 0, headers[0].start() if headers else len(source)
    if table:
        header = next((i for i, match in enumerate(headers) if match[0].strip() == f"[{table}]"), None)
        if header is None:
            raise ValueError(f"Unsupported or missing Codex table: {table}")
        start = headers[header].end()
        end = headers[header + 1].start() if header + 1 < len(headers) else len(source)
    section = source[start:end]
    parsed = tomllib.loads(source)
    current = parsed if not table else parsed["model_providers"]["strata-local"]
    if current.get(key) == value and type(current.get(key)) is type(value):
        return source
    literal = json.dumps(value, ensure_ascii=False)
    if key in current:
        pattern = rf'(?m)^([ \t]*{re.escape(key)}[ \t]*=[ \t]*)(?:"(?:[^"\\\r\n]|\\.)*"|\x27[^\x27\r\n]*\x27|[+\-]?[0-9_]+)([ \t]*(?:#[^\r\n]*)?)(\r?$)'
        section, count = re.subn(pattern, lambda match: match[1] + literal + match[2] + match[3], section)
        if count != 1:
            raise ValueError(f"Unsupported Codex setting format: {key}")
    else:
        section = f"{key} = {literal}\n" + section
    updated = source[:start] + section + source[end:]
    parsed = tomllib.loads(updated)
    target = parsed if not table else parsed["model_providers"]["strata-local"]
    if target.get(key) != value or type(target.get(key)) is not type(value):
        raise ValueError(f"Could not update Codex setting: {key}")
    return updated


def codex_model_catalog(path, document, context, vision):
    destination = path.with_name("strata-hub.models.json")
    configured = document.get("model_catalog_json")
    source = (path.parent / configured).resolve() if configured else destination
    if configured or destination.exists():
        catalog = json.loads(source.read_text(encoding="utf-8-sig"))
    else:
        catalog = {"models": []}
    models = catalog["models"]
    model = next((row for row in models if row.get("slug") == "strata"), None)
    if model is None:
        model = {"slug": "strata", "display_name": "Strata local", "description": "Local Strata model",
                 "default_reasoning_level": "high", "supported_reasoning_levels": [{"effort": "high", "description": "High"}],
                 "shell_type": "unified_exec", "visibility": "list", "supported_in_api": True, "priority": 0,
                 "support_verbosity": False, "truncation_policy": {"mode": "bytes", "limit": 10000},
                 "experimental_supported_tools": [],
                 "base_instructions": "You are a coding assistant. Follow the user's instructions, inspect relevant files before editing, preserve unrelated changes, and verify your work with focused checks."}
        models.append(model)
    model["input_modalities"] = ["text", "image"] if vision else ["text"]
    if context is not None:
        model["context_window"] = context
    return destination, json.dumps(catalog, indent=2) + "\n"


def scan(report, home, endpoint):
    agents = []
    for name, (label, package) in CATALOG.items():
        executable = find_agent(name, report, home)
        path = config_path(name, home)
        connected = False
        context = None
        vision = None
        config_error = ""
        try:
            if path.exists():
                if name == "codex":
                    config = tomllib.loads(path.read_text(encoding="utf-8"))
                    connected = config.get("model_providers", {}).get("strata-local", {}).get("base_url") == endpoint
                    context = config.get("model_context_window")
                    if config.get("model_catalog_json"):
                        catalog = read_json(path.parent / config["model_catalog_json"])
                        model = next((row for row in catalog.get("models", []) if row.get("slug") == "strata"), {})
                        if isinstance(model.get("input_modalities"), list):
                            vision = "image" in model["input_modalities"]
                else:
                    config = read_json(path)
                    provider = config.get("providers" if name == "pi" else "provider", {}).get("strata-local", {})
                    connected = (provider.get("baseUrl") if name == "pi" else provider.get("options", {}).get("baseURL")) == endpoint
                    if name == "pi":
                        model = next((model for model in provider.get("models", []) if model.get("id") == "strata"), {})
                        context = model.get("contextWindow")
                        if isinstance(model.get("input"), list):
                            vision = "image" in model["input"]
                    else:
                        model = provider.get("models", {}).get("strata", {})
                        context = model.get("limit", {}).get("context")
                        if type(model.get("attachment")) is bool and isinstance(model.get("modalities", {}).get("input"), list):
                            vision = model["attachment"] and "image" in model["modalities"]["input"]
        except (ValueError, RuntimeError) as error:
            config_error = f"Config needs attention: {error}"
        agents.append({"id": name, "name": label, "package": package, "installed": executable is not None,
                       "path": str(executable) if executable else "Not installed", "connected": connected,
                       "context": context if type(context) is int and context > 0 else None,
                       "vision": vision,
                       "config_error": config_error, "launcher": (report / "agents" / f"{name}-strata.ps1").exists()})
    return agents


def sync_settings(report, home, endpoint, context, vision):
    if context is not None and (type(context) is not int or not 1 <= context <= 2147483647):
        raise ValueError("Context must be a positive 32-bit integer.")
    if not local_endpoint(endpoint) or type(vision) is not bool:
        raise ValueError("A local Strata endpoint and explicit image support are required.")
    plans = []
    for name, (label, _) in CATALOG.items():
        path = config_path(name, home)
        if not path.exists():
            continue
        original = path.read_bytes()
        source = original.decode("utf-8-sig")
        if name == "codex":
            document = tomllib.loads(source)
            provider = document.get("model_providers", {}).get("strata-local", {})
            if (not local_endpoint(provider.get("base_url")) or document.get("model_provider") != "strata-local"
                    or document.get("model") != "strata"):
                continue
            text = toml_setting(source, "model_providers.strata-local", "base_url", endpoint)
            if context is not None:
                text = toml_setting(text, "", "model_context_window", context)
            catalog_path, catalog_text = codex_model_catalog(path, document, context, vision)
            catalog_original = catalog_path.read_bytes() if catalog_path.exists() else None
            if catalog_original is None or json.loads(catalog_original) != json.loads(catalog_text):
                plans.append((label, catalog_path, catalog_original, catalog_text))
            text = toml_setting(text, "", "model_catalog_json", catalog_path.as_posix())
            if text == source:
                continue
        else:
            document = json.loads(source)
            provider = document.get("providers" if name == "pi" else "provider", {}).get("strata-local", {})
            url = provider.get("baseUrl") if name == "pi" else provider.get("options", {}).get("baseURL")
            if not local_endpoint(url):
                continue
            before = json.dumps(document, sort_keys=True)
            if name == "pi":
                model = next((row for row in provider.get("models", []) if row.get("id") == "strata"), None)
                if model is None:
                    continue
                target, context_key, output_key = model, "contextWindow", "maxTokens"
                provider["baseUrl"] = endpoint
                model["input"] = ["text", "image"] if vision else ["text"]
            else:
                if "providers" in document:
                    raise ValueError("The OpenCode v2 schema is not supported by this connector.")
                model = provider.get("models", {}).get("strata")
                if model is None:
                    continue
                target, context_key, output_key = model.setdefault("limit", {}), "context", "output"
                provider["options"]["baseURL"] = endpoint
                model["attachment"] = vision
                model.setdefault("modalities", {})["input"] = ["text", "image"] if vision else ["text"]
            output = target.get(output_key)
            if output is not None and (type(output) is not int or output <= 0):
                raise ValueError(f"{label} has an invalid output token limit.")
            if context is not None:
                target[context_key] = context
                if output is not None and output > context:
                    target[output_key] = context
            if json.dumps(document, sort_keys=True) == before:
                continue
            text = json.dumps(document, indent=2) + "\n"
        plans.append((label, path, original, text))
    # Parse every candidate before writing any configuration.
    backup = report / "agents" / "backups" / datetime.datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    updated = []
    for label, path, original, text in plans:
        if (path.read_bytes() if path.exists() else None) != original:
            raise RuntimeError(f"{label} configuration changed during synchronization. Updated so far: {', '.join(updated) or 'none'}.")
        save_text(path, text, backup)
        if label not in updated:
            updated.append(label)
        emit("progress", message=f"{label} Strata connection updated")
    return updated


def node_runtime():
    node = shutil.which("node.exe")
    if node is None:
        raise RuntimeError("Node.js 22.19 or newer is required. Install Node.js, then scan agents again.")
    executable = Path(node)
    npm = executable.parent / "node_modules" / "npm" / "bin" / "npm-cli.js"
    if not npm.is_file():
        raise RuntimeError("npm-cli.js was not found beside Node.js. Install a complete Node.js distribution.")
    actual = subprocess.check_output([node, "--version"], text=True).strip().removeprefix("v")
    if tuple(map(int, actual.split(".")[:2])) < (22, 19):
        raise RuntimeError(f"Node.js {actual} is too old; 22.19 or newer is required.")
    return executable, npm


def install(name, report, home):
    if find_agent(name, report, home):
        raise RuntimeError("This agent is already installed. Use Connect to Strata.")
    node, npm = node_runtime()
    emit("progress", message=f"Installing {CATALOG[name][0]}")
    arguments = [str(node), str(npm), "install", "--global", "--prefix", str(report / "agents" / "packages" / name)]
    if name == "pi":
        arguments.append("--ignore-scripts")
    arguments.append(CATALOG[name][1])
    subprocess.run(arguments, stdin=subprocess.DEVNULL, check=True)
    if find_agent(name, report, home) is None:
        raise RuntimeError("Installation finished without creating the expected agent launcher.")


def ps_literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def connect(name, report, home, source, port, context, vision):
    executable = find_agent(name, report, home)
    if executable is None:
        raise RuntimeError("Install the agent before connecting it to Strata.")
    config = read_json(source)
    key = config["api_key"][0] if isinstance(config["api_key"], list) else config["api_key"]
    if not isinstance(key, str) or not key:
        raise RuntimeError("The selected profile has no usable API key.")
    endpoint = f"http://127.0.0.1:{port}/v1"
    path = config_path(name, home)
    backup = report / "agents" / "backups" / datetime.datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    inputs = ["text", "image"] if vision else ["text"]
    if name == "pi":
        document = read_json(path)
        provider = document.setdefault("providers", {}).setdefault("strata-local", {})
        provider.update({"baseUrl": endpoint, "api": "openai-completions", "apiKey": key})
        provider.setdefault("compat", {}).update({"supportsStore": False, "supportsDeveloperRole": False,
                                                  "supportsReasoningEffort": True, "maxTokensField": "max_tokens"})
        models = provider.setdefault("models", [])
        model = next((model for model in models if model.get("id") == "strata"), None)
        if model is None:
            model = {"id": "strata"}
            models.append(model)
        model.update({"name": "Strata local", "reasoning": True, "input": inputs,
                      "contextWindow": context, "maxTokens": min(context, 16384),
                      "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}})
        save_json(path, document, backup)
        command = " --provider strata-local --model strata"
    elif name == "opencode":
        document = read_json(path)
        if "providers" in document:
            raise RuntimeError("This OpenCode config uses the v2 schema; this connector targets the installed v1 client.")
        provider = document.setdefault("provider", {}).setdefault("strata-local", {})
        provider.update({"npm": "@ai-sdk/openai-compatible", "name": "Strata (local)"})
        provider.setdefault("options", {}).update({"baseURL": endpoint, "apiKey": key})
        provider.setdefault("models", {}).setdefault("strata", {}).update({"name": "Strata local", "attachment": vision, "reasoning": True,
            "temperature": True, "tool_call": True, "interleaved": {"field": "reasoning_content"},
            "limit": {"context": context, "output": min(context, 16384)}, "modalities": {"input": inputs, "output": ["text"]},
            "cost": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0}})
        save_json(path, document, backup)
        command = " --model strata-local/strata"
    else:
        catalog_path, catalog_text = codex_model_catalog(path, {}, context, vision)
        save_text(catalog_path, catalog_text, backup)
        text = (f'model = "strata"\nmodel_provider = "strata-local"\nmodel_context_window = {context}\n'
                f'model_catalog_json = {json.dumps(catalog_path.as_posix())}\n'
                'model_reasoning_effort = "high"\nweb_search = "disabled"\n\n'
                '[model_providers.strata-local]\nname = "Strata (local)"\n'
                f'base_url = "{endpoint}"\nwire_api = "responses"\nenv_key = "STRATA_API_KEY"\n'
                'requires_openai_auth = false\nsupports_websockets = false\nstream_idle_timeout_ms = 600000\n\n'
                '[features]\napps = false\n')
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.exists():
            backup.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, backup / path.name)
        path.write_text(text, encoding="utf-8")
        command = " --profile strata-hub"
    launcher = report / "agents" / f"{name}-strata.ps1"
    launcher.parent.mkdir(parents=True, exist_ok=True)
    credential = (f"$profile = Get-Content -LiteralPath {ps_literal(source)} -Raw | ConvertFrom-Json\n"
                  "$env:STRATA_API_KEY = @($profile.api_key)[0]\n") if name == "codex" else ""
    launcher.write_text("$ErrorActionPreference = 'Stop'\n" + credential + f"& {ps_literal(executable)}{command} @args\nexit $LASTEXITCODE\n", encoding="utf-8-sig")
    emit("progress", message=f"{CATALOG[name][0]} connected to {endpoint}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["scan", "install", "connect", "launch", "sync-settings"])
    parser.add_argument("--agent", choices=list(CATALOG))
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--profile", type=Path)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--context", type=int)
    parser.add_argument("--vision", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.port <= 65535 or (args.context is not None and args.context <= 0):
        raise ValueError("Invalid Strata endpoint or context")
    if args.mode != "sync-settings" and args.context is None:
        raise ValueError("An explicit context is required for this operation.")
    home = Path.home()
    if args.mode not in ("scan", "sync-settings") and args.agent is None:
        raise ValueError("An agent is required")
    if args.mode == "install":
        install(args.agent, args.report, home)
    if args.mode == "connect":
        if args.profile is None:
            raise ValueError("A Strata profile is required to connect an agent.")
        connect(args.agent, args.report, home, args.profile, args.port, args.context, args.vision)
    if args.mode == "launch":
        launcher = args.report / "agents" / f"{args.agent}-strata.ps1"
        if not launcher.is_file():
            raise RuntimeError("Connect the agent before launching it.")
        subprocess.Popen(["powershell.exe", "-NoExit", "-File", str(launcher)], creationflags=subprocess.CREATE_NEW_CONSOLE)
    endpoint = f"http://127.0.0.1:{args.port}/v1"
    if args.mode == "sync-settings":
        updated = sync_settings(args.report, home, endpoint, args.context, args.vision)
        message = (f"Connection updated for {', '.join(updated)}. Reload open clients if needed." if updated
                   else "Connected agent settings are up to date; no configuration changes needed.")
    else:
        message = {"scan": "Agent scan complete.", "install": "Agent installed. Connect it to Strata next.",
                   "connect": "Strata connection saved. Existing agents may need to reload their models.", "launch": "Agent launched."}[args.mode]
    emit("agents", agents=scan(args.report, home, endpoint), endpoint=endpoint,
         message=message)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        emit("error", message=str(error))
        sys.exit(1)
