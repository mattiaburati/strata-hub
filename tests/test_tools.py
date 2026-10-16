import importlib.util
import json
import os
import hashlib
import io
import subprocess
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch


def module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).parents[1] / "scripts" / f"{name}.py")
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


agents = module("agents")
updater = module("update_strata")
model_library = module("model_library")


class ToolTests(unittest.TestCase):
    def test_model_preparation_publishes_own_profile_only_after_success(self):
        lib = model_library
        with tempfile.TemporaryDirectory() as temporary, patch.object(lib, "require_idle"), patch.object(lib, "emit"), patch.object(lib, "MTP_EXPERT_BYTES", 1):
            base = Path(temporary)
            root, models, profiles, report = [base / name for name in ("Strata", "models", "profiles", "report")]
            for path in (root, models, profiles, report): path.mkdir()
            (root / "tools").mkdir(); (root / "engine").mkdir(); (root / "data").mkdir()
            for tool in ("iq_pack.py", "mtp_fetch.py", "mtp_pack.py", "mtp_rt.py"):
                (root / "tools" / tool).touch()
            (root / "engine/strata.exe").touch()
            lib.write_json(root / "engine/BUILD.json", {"cuda": "13.0"})
            (root / "data/expert-profile.bin").write_bytes(b"profile")
            folder = models / "models/fixture"; folder.mkdir(parents=True)
            shard = folder / "model.gguf"; shard.write_bytes(b"GGUF")
            identity = "a" * 20
            entry = {"id": identity, "name": "My fine-tune", "state": "downloaded", "path": str(folder),
                     "files": [{"name": shard.name, "size": 4}], "size": 4}
            lib.write_json(report / "model-library.json", {"version": 1, "models": [entry]})
            rt = models / "mtp/rt"; rt.mkdir(parents=True)
            for name in ("experts.bin", "dense.bin", "draft_vocab.bin"): (rt / name).write_bytes(b"x")
            (rt / "dense.txt").write_text("projection bf16 1 1 0 1\n")
            template = report / "template.json"
            lib.write_json(template, {"exe": str(root / "engine/strata.exe"), "lib_dirs": [str(root / "engine")],
                "port": 8082, "api_key": "fixture-only", "env": {"STRATA_PIN_BY_NEED": "1", "STEERING": "do-not-copy"},
                "args": ["--max-context", "65536", "--prefill", "auto", "--adapt-async", "1", "--control-vector-scaled", "other-model:1"]})
            tensor = {"name": "per_layer_token_embd.weight"}
            inspector = SimpleNamespace(read_headers=lambda sources: ("qwen4exp", [tensor]),
                fingerprint=lambda arch, ts: {"expert_bytes": {"IQ3_S": 10}},
                load_fingerprints=lambda: {"qwen/IQ3_S": {"expert_bytes": {"IQ3_S": 10}}},
                verdict=lambda fp, table: ("layout", "fine-tune"), qs=SimpleNamespace(read_gguf=lambda source: ({}, [tensor])))
            setup = SimpleNamespace(EXE="strata.exe", get_llama_cpp=lambda: root / "third_party/llama.cpp")
            profile = profiles / f"strata-hub-{identity}.json"
            pack = models / f"packs/hub-{identity}"
            def pack_tool(root, tool, *args, env=None):
                self.assertEqual(tool, "iq_pack.py")
                self.assertIn("--compat-bf16", args)
                self.assertNotIn("--base", args)
                self.assertNotIn("--experts-bin", args)
                self.assertEqual(args[args.index("--gguf") + 1], shard)
                for name in ("dense.bin", "index.txt", "native_experts.txt", "tokenizer/vocab.json", "tokenizer/chat_template.jinja"):
                    file = pack / name; file.parent.mkdir(parents=True, exist_ok=True); file.write_bytes(b"own model asset")
            with patch.object(lib, "inspector", return_value=(inspector, "signature")), patch.object(lib, "load_setup", return_value=setup):
                with patch.object(lib, "run_tool", side_effect=RuntimeError("packing failed")):
                    with self.assertRaisesRegex(RuntimeError, "packing failed"):
                        lib.prepare(root, report, models, profiles, identity, None, template)
                self.assertFalse(profile.exists())
                self.assertFalse(lib.read_json(pack / "manifest.json")["complete"])
                with patch.object(lib, "run_tool", side_effect=pack_tool) as tools:
                    lib.prepare(root, report, models, profiles, identity, None, template)
                    self.assertEqual(tools.call_count, 1)
                config = lib.read_json(profile)
                self.assertEqual(config["tokenizer"], str(pack / "tokenizer"))
                self.assertEqual(config["port"], 8082)
                self.assertEqual(config["api_key"], "fixture-only")
                self.assertEqual(config["args"][config["args"].index("--max-context") + 1], "65536")
                self.assertNotIn("--control-vector-scaled", config["args"])
                self.assertNotIn("STEERING", config["env"])
                self.assertTrue(lib.read_json(pack / "manifest.json")["complete"])
                self.assertEqual(lib.library(report)["models"][0]["state"], "ready")
                with patch.object(lib, "run_tool") as tools:
                    lib.prepare(root, report, models, profiles, identity, None, template)
                    tools.assert_not_called()
                profile.unlink()
                (rt / "experts.bin").unlink()
                (root / "data/draft_vocab.bin").write_bytes(b"vocab")
                calls = []
                def first_mtp(root, tool, *args, env=None):
                    calls.append(tool)
                    if tool == "iq_pack.py":
                        pack_tool(root, tool, *args, env=env)
                    elif tool == "mtp_rt.py":
                        (rt / "experts.bin").write_bytes(b"x")
                with patch.object(lib, "run_tool", side_effect=first_mtp):
                    lib.prepare(root, report, models, profiles, identity, None, template)
                self.assertEqual(calls, ["iq_pack.py", "mtp_fetch.py", "mtp_fetch.py", "mtp_pack.py", "mtp_rt.py"])
                self.assertEqual((rt / "draft_vocab.bin").read_bytes(), b"vocab")
                self.assertTrue(profile.is_file())

    def test_gguf_groups_require_complete_shards_and_safe_repository_paths(self):
        lib = model_library
        rows = [{"rfilename": f"IQ3_S/model-0000{i}-of-00002.gguf", "size": 10, "lfs": {"sha256": "a" * 64}} for i in (1, 2)]
        groups = lib.group_files(rows + [{"rfilename": "mmproj.gguf"}])
        self.assertEqual(len(groups), 1)
        self.assertTrue(lib.complete_shards(next(iter(groups.values()))))
        self.assertFalse(lib.complete_shards(next(iter(groups.values()))[:1]))
        self.assertEqual(lib.repo_id("https://huggingface.co/owner/repo"), "owner/repo")
        for path in ("../weights.gguf", "C:/weights.gguf", "folder\\weights.gguf", "NUL.gguf"):
            with self.assertRaises(ValueError):
                lib.relative_file(path)

    def test_model_download_resumes_only_on_user_request_and_verifies_sha256(self):
        lib = model_library
        content = b"GGUF fixture bytes for a streamed model"
        class Response(io.BytesIO):
            status = 206
            def __init__(self, body, start):
                super().__init__(body)
                self.headers = {"Content-Range": f"bytes {start}-{len(content)-1}/{len(content)}"}
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            report, destination = base / "report", base / "downloads"
            report.mkdir(); destination.mkdir()
            file = {"name": "IQ3_S/model.gguf", "size": len(content), "sha256": hashlib.sha256(content).hexdigest()}
            row = {"id": "variant", "name": "IQ3_S/model", "quant": "IQ3_S", "size": len(content), "files": [file], "downloadable": True, "verdict": "layout"}
            lib.write_json(report / "model-catalog.json", {"repo": "owner/repo", "revision": "a" * 40, "inspector": "signature", "variants": [row]})
            with patch.object(lib, "inspector", return_value=(None, "signature")), patch.object(lib, "emit"), patch.object(lib, "open_url", return_value=Response(content[:8], 0)):
                with self.assertRaises(EOFError):
                    lib.download(base, report, base, "variant", destination)
            entry = lib.library(report)["models"][0]
            folder = Path(entry["path"])
            self.assertEqual((folder / "IQ3_S/model.gguf.part").read_bytes(), content[:8])
            with patch.object(lib, "inspector", return_value=(None, "signature")), patch.object(lib, "emit"), patch.object(lib, "open_url", return_value=Response(content[8:], 8)) as network:
                lib.download(base, report, base, "variant", destination)
                self.assertEqual(network.call_args.args[1], 8)
            self.assertEqual((folder / "IQ3_S/model.gguf").read_bytes(), content)
            self.assertEqual(lib.library(report)["models"][0]["state"], "downloaded")

    def test_model_move_and_delete_preserve_other_data_and_update_profiles(self):
        lib = model_library
        with tempfile.TemporaryDirectory() as temporary, patch.object(lib, "require_idle"), patch.object(lib, "emit"):
            base = Path(temporary)
            root, models, profiles, report, destination = [base / name for name in ("Strata", "data", "profiles", "report", "destination")]
            for path in (root, models, profiles, report, destination): path.mkdir()
            folder = models / "models" / "existing"
            folder.mkdir(parents=True)
            (folder / "weights.gguf").write_bytes(b"model fixture")
            pack = folder / "pack"
            pack.mkdir()
            lib.write_json(pack / "manifest.json", {"source": str(folder / "weights.gguf")})
            profile = profiles / "strata-fixture.json"
            lib.write_json(profile, {"args": ["--native", str(folder / "weights.gguf"), "--pack", str(pack)], "api_key": ["fixture-key"]})
            lib.write_json(report / "hub-profiles.json", {"selected": profile.name, "profiles": {profile.name: {"extra_args": json.dumps(['--native', str(folder / 'weights.gguf')])}}})
            launcher = profiles / 'run-fixture.bat'
            launcher.write_text('python server.py --config ' + str(profile), encoding='utf-8')
            unrelated = base / 'keep.txt'
            unrelated.write_text('preserve', encoding='utf-8')
            identifier = lib.existing_models(models, [])[0]["id"]
            plan = lib.operation_plan(root, report, profiles, identifier, "move", destination, models)
            self.assertEqual(len(plan["changes"]), 3)
            with patch.object(lib, "same_volume", return_value=False):
                lib.mutate(root, report, profiles, identifier, "move", destination, plan["digest"], None, models)
            moved = destination / folder.name
            self.assertFalse(folder.exists())
            self.assertEqual((moved / "weights.gguf").read_bytes(), b"model fixture")
            self.assertEqual(lib.read_json(moved / "pack/manifest.json")["source"], str(moved / "weights.gguf"))
            updated = lib.read_json(profile)
            self.assertEqual(updated["args"][1], str(moved / "weights.gguf"))
            self.assertEqual(updated["api_key"], ["fixture-key"])
            self.assertTrue(list((report / "model-backups").glob("*/operation.json")))
            plan = lib.operation_plan(root, report, profiles, identifier, "delete", models=models)
            self.assertEqual({Path(file['path']) for file in plan['deletions']}, {profile, launcher})
            lib.mutate(root, report, profiles, identifier, "delete", None, plan["digest"], None, models)
            self.assertFalse(moved.exists())
            self.assertFalse(profile.exists())
            self.assertFalse(launcher.exists())
            self.assertEqual(lib.read_json(report / 'hub-profiles.json'), {'profiles': {}})
            self.assertEqual(unrelated.read_text(encoding='utf-8'), 'preserve')
            self.assertTrue(list((report / 'model-backups').glob('*/deleted-*.json')))
            self.assertEqual(lib.library(report)["models"], [])

    @unittest.skipUnless(os.name == "nt", "Windows installer")
    def test_installer_uses_stable_source_and_official_setup_without_starting(self):
        script = Path(__file__).parents[1] / "scripts" / "install_strata.ps1"
        shell = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            wrapper = base / "mock-installer.ps1"
            wrapper.write_text(r'''
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$Root = Join-Path $env:HUB_TEST_BASE 'Strata & fixture'
$Data = Join-Path $env:HUB_TEST_BASE 'Models with spaces'
$Model = 'qwen-iq3s'
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($env:HUB_TEST_INSTALLER, [ref]$null, [ref]$errors)
if ($errors) { throw 'Installer syntax error' }
foreach ($function in $ast.FindAll({param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst]}, $false)) {
    Invoke-Expression $function.Extent.Text
}
function Get-Command { return @{Source = 'fixture-git'} }
function Invoke-RestMethod { return @{tag_name = 'v0.1.44.1'; draft = $false; prerelease = $false; html_url = 'https://github.com/Niko1221/Strata/releases/tag/v0.1.44.1'} }
function Find-Python { return 'fixture-python' }
$script:calls = [Collections.Generic.List[object]]::new()
function Invoke-Native([string]$Program, [string[]]$Arguments) {
    $script:calls.Add(@{program = $Program; arguments = $Arguments})
    if ($Program -eq 'fixture-git') {
        New-Item -ItemType Directory -Path (Join-Path $Root 'serve') -Force | Out-Null
        foreach ($name in @('setup.py', 'START-HERE.bat', 'serve/server.py')) { Set-Content -LiteralPath (Join-Path $Root $name) -Value 'fixture' }
    } elseif ($Program -eq 'fixture-python') {
        New-Item -ItemType Directory -Path (Join-Path $Root '.venv/Scripts') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $Root '.venv/Scripts/python.exe') -Value 'fixture'
    } else {
        New-Item -ItemType Directory -Path (Join-Path $Root 'engine') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $Root 'engine/BUILD.json') -Value '{"version":"0.1.44.1"}'
        Set-Content -LiteralPath (Join-Path $Root 'strata-fixture.json') -Value '{"args":["--fixture"],"model_name":"fixture"}'
    }
}
Install-Strata (Get-InstallPlan)
Write-Output ('HUB_CALLS ' + (ConvertTo-Json -InputObject @($script:calls.ToArray()) -Depth 5 -Compress))
''', encoding="utf-8")
            environment = dict(os.environ, HUB_TEST_BASE=str(base), HUB_TEST_INSTALLER=str(script))
            result = subprocess.run([str(shell), "-NoProfile", "-NonInteractive", "-File", str(wrapper)],
                                    env=environment, capture_output=True, encoding="utf-8", timeout=15)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            calls = json.loads(next(line.removeprefix("HUB_CALLS ") for line in result.stdout.splitlines() if line.startswith("HUB_CALLS ")))
            clone = calls[0]["arguments"]
            self.assertIn("v0.1.44.1", clone)
            self.assertIn("https://github.com/Niko1221/Strata.git", clone)
            setup = calls[-1]["arguments"]
            self.assertIn(str(base / "Strata & fixture" / "setup.py"), setup)
            self.assertIn("--no-start", setup)
            self.assertIn("--no-browser", setup)
            self.assertIn("IQ3_S", setup)
            self.assertEqual(len(setup[setup.index("--api-key") + 1]), 64)
            self.assertIn('"kind":"complete"', result.stdout)

    @unittest.skipUnless(os.name == "nt", "Windows installer")
    def test_installer_preflight_preserves_paths_and_refuses_existing_installs(self):
        script = Path(__file__).parents[1] / "scripts" / "install_strata.ps1"
        shell = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / "Strata & test"
            data = base / "Models with spaces"
            arguments = [str(shell), "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", str(script),
                         "-Root", str(root), "-Data", str(data), "-Model", "qwen-iq3s", "-CheckOnly"]
            result = subprocess.run(arguments, capture_output=True, encoding="utf-8", check=True, timeout=15)
            plan = json.loads(result.stdout)
            self.assertEqual(Path(plan["root"]), root)
            self.assertEqual(Path(plan["data"]), data)
            self.assertIn("--no-start", plan["arguments"])
            self.assertIn("--no-browser", plan["arguments"])
            self.assertIn("IQ3_S", plan["arguments"])
            self.assertEqual(plan["arguments"][plan["arguments"].index("--data-dir") + 1], str(data))
            self.assertFalse(root.exists())
            self.assertFalse(data.exists())
            root.mkdir()
            marker = root / "setup.py"
            marker.write_text("existing installation", encoding="utf-8")
            refused = subprocess.run(arguments, capture_output=True, encoding="utf-8", timeout=15)
            self.assertEqual(refused.returncode, 1)
            self.assertIn("not overwritten", refused.stdout)
            self.assertEqual(marker.read_text(encoding="utf-8"), "existing installation")

    def test_update_backs_up_and_uses_official_update_without_starting(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root, data, report = base / "Strata", base / "models", base / "hub" / "report"
            engine = root / "engine"
            engine.mkdir(parents=True)
            data.mkdir()
            report.mkdir(parents=True)
            (engine / "BUILD.json").write_text('{"version":"0.1.43"}', encoding="utf-8")
            (engine / "engine.bin").write_bytes(b"old engine")
            (data / "model.gguf").write_bytes(b"model weights")
            (report / "hub-profiles.json").write_text('{}', encoding="utf-8")
            before = updater.model_manifest(data)
            calls = []

            def command(arguments, cwd, capture=False):
                calls.append(arguments)
                if arguments[1:4] == ["remote", "get-url", "origin"]:
                    return "https://github.com/Niko1221/Strata.git"
                if arguments[1] == "status":
                    return ""
                if arguments[1] == "rev-parse":
                    return "a" * 40 if arguments[2] == "HEAD" else "b" * 40
                if "--update" in arguments:
                    (engine / "BUILD.json").write_text('{"version":"0.1.44.1"}', encoding="utf-8")
                return "" if capture else None

            with patch.object(updater, "require_idle"), patch.object(updater, "run", side_effect=command):
                updater.install(root, data, report, "v0.1.44.1", report.parent)
            self.assertTrue(list((report / "update-backups").glob("*/engine/engine.bin")))
            self.assertEqual(updater.model_manifest(data), before)
            self.assertTrue(any("--update" in command and "--yes" in command for command in calls))
            self.assertFalse(any("--start" in command for command in calls))
            self.assertTrue(any(command[1:3] == ["merge", "--ff-only"] for command in calls))

    def test_stable_version_comparison_and_input_validation(self):
        self.assertGreater(updater.version("v0.1.44.1"), updater.version("0.1.43"))
        self.assertEqual(updater.version("v0.1.44"), updater.version("0.1.44.0"))
        for invalid in ["main", "v0.1.44-rc1", "--upload-pack=bad"]:
            with self.assertRaises(ValueError):
                updater.version(invalid)

    def test_agent_settings_sync_migrates_local_ports_and_image_capabilities(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {}, clear=True), patch.object(agents, "emit"):
            home = Path(temporary)
            report = home / "report"
            profile = home / "profile.json"
            profile.write_text(json.dumps({"api_key": ["fixture-key"]}), encoding="utf-8")
            with patch.object(agents, "find_agent", return_value=home / "fixture.exe"):
                for name in agents.CATALOG:
                    agents.connect(name, report, home, profile, 8097, 65536, True)
                originals = {name: agents.config_path(name, home).read_bytes() for name in ("pi", "opencode")}
                codex_path = agents.config_path("codex", home)
                catalog_path = codex_path.with_name("strata-hub.models.json")
                catalog = agents.read_json(catalog_path)
                catalog["models"][0]["base_instructions"] = "Keep existing model instructions."
                catalog["models"].append({"slug": "unrelated", "keep": True})
                catalog_path.write_text(json.dumps(catalog), encoding="utf-8")
                self.assertEqual(agents.sync_settings(report, home, "http://127.0.0.1:8098/v1", None, False),
                                 ["Pi", "OpenCode", "Codex CLI"])
                for name, content in originals.items():
                    expected = json.loads(content)
                    provider = expected["providers" if name == "pi" else "provider"]["strata-local"]
                    if name == "pi":
                        provider["baseUrl"] = "http://127.0.0.1:8098/v1"
                        provider["models"][0]["input"] = ["text"]
                    else:
                        provider["options"]["baseURL"] = "http://127.0.0.1:8098/v1"
                        provider["models"]["strata"]["attachment"] = False
                        provider["models"]["strata"]["modalities"]["input"] = ["text"]
                    self.assertEqual(agents.read_json(agents.config_path(name, home)), expected)
                catalog["models"][0]["input_modalities"] = ["text"]
                self.assertEqual(agents.read_json(catalog_path), catalog)
                self.assertIn('base_url = "http://127.0.0.1:8098/v1"', codex_path.read_text(encoding="utf-8"))
                scan = agents.scan(report, home, "http://127.0.0.1:8098/v1")
                self.assertTrue(all(row["connected"] and row["vision"] is False and row["context"] == 65536 for row in scan))
                agents.sync_settings(report, home, "http://127.0.0.1:8097/v1", 131072, True)
                self.assertTrue(all(row["vision"] is True for row in agents.scan(report, home, "http://127.0.0.1:8097/v1")))
                # The reserved local connection moves; remote providers remain untouched.
                pi_path = agents.config_path("pi", home)
                pi = agents.read_json(pi_path)
                pi["providers"]["strata-local"]["baseUrl"] = "https://remote.example/v1"
                pi_path.write_text(json.dumps(pi), encoding="utf-8")
                original = pi_path.read_bytes()
                agents.sync_settings(report, home, "http://127.0.0.1:8099/v1", 32768, False)
                self.assertEqual(pi_path.read_bytes(), original)

    def test_context_sync_preserves_unrelated_settings_and_skips_other_endpoints(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {}, clear=True), patch.object(agents, "emit"):
            home = Path(temporary)
            report = home / "report"
            profile = home / "profile.json"
            profile.write_text(json.dumps({"api_key": ["fixture-key"]}), encoding="utf-8")
            with patch.object(agents, "find_agent", return_value=home / "fixture.exe"):
                for name in agents.CATALOG:
                    agents.connect(name, report, home, profile, 8097, 65536, True)
                originals = {name: agents.config_path(name, home).read_bytes() for name in agents.CATALOG}
                launchers = {path: path.read_bytes() for path in (report / "agents").glob("*.ps1")}
                backup_count = len(list((report / "agents/backups").glob("*")))
                self.assertEqual(agents.sync_settings(report, home, "http://127.0.0.1:8097/v1", 65536, True), [])
                self.assertEqual(len(list((report / "agents/backups").glob("*"))), backup_count)
                for name, content in originals.items():
                    self.assertEqual(agents.config_path(name, home).read_bytes(), content)
                codex = agents.config_path("codex", home)
                codex.write_text(codex.read_text(encoding="utf-8").replace("model_context_window = 65536", "model_context_window = 65536 # keep comment")
                                 + '\n[custom]\nkeep = "untouched"\n', encoding="utf-8")
                changed = agents.sync_settings(report, home, "http://127.0.0.1:8097/v1", 4096, True)
                self.assertEqual(changed, ["Pi", "OpenCode", "Codex CLI"])
                for name in ("pi", "opencode"):
                    expected = json.loads(originals[name])
                    if name == "pi":
                        model = expected["providers"]["strata-local"]["models"][0]
                        model.update(contextWindow=4096, maxTokens=4096)
                    else:
                        expected["provider"]["strata-local"]["models"]["strata"]["limit"].update(context=4096, output=4096)
                    self.assertEqual(agents.read_json(agents.config_path(name, home)), expected)
                codex_content = codex.read_text(encoding="utf-8")
                self.assertIn("model_context_window = 4096 # keep comment", codex_content)
                self.assertIn('[custom]\nkeep = "untouched"', codex_content)
                self.assertEqual([row["context"] for row in agents.scan(report, home, "http://127.0.0.1:8097/v1")], [4096] * 3)
                count = len(list((report / "agents/backups").glob("*")))
                self.assertEqual(agents.sync_settings(report, home, "http://127.0.0.1:8097/v1", 4096, True), [])
                self.assertEqual(len(list((report / "agents/backups").glob("*"))), count)
                agents.sync_settings(report, home, "http://127.0.0.1:8097/v1", 131072, True)
                self.assertEqual(agents.read_json(agents.config_path("pi", home))["providers"]["strata-local"]["models"][0]["maxTokens"], 4096)
                for path, content in launchers.items():
                    self.assertEqual(path.read_bytes(), content)
                self.assertFalse((home / ".codex/config.toml").exists())

    def test_context_sync_does_not_create_connections_and_preflights_invalid_files(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {}, clear=True), patch.object(agents, "emit"):
            home = Path(temporary)
            report = home / "report"
            self.assertEqual(agents.sync_settings(report, home, "http://127.0.0.1:8097/v1", 65536, True), [])
            self.assertEqual(list(home.iterdir()), [])
            pi = agents.config_path("pi", home)
            pi.parent.mkdir(parents=True)
            pi.write_text(json.dumps({"providers": {"strata-local": {"baseUrl": "http://127.0.0.1:8097/v1",
                          "models": [{"id": "strata", "contextWindow": 32768, "maxTokens": 1000}], "apiKey": "fixture"}}}), encoding="utf-8")
            original = pi.read_bytes()
            opencode = agents.config_path("opencode", home)
            opencode.parent.mkdir(parents=True)
            opencode.write_text('{/* unsupported JSONC */}', encoding="utf-8")
            with self.assertRaises(json.JSONDecodeError):
                agents.sync_settings(report, home, "http://127.0.0.1:8097/v1", 65536, True)
            self.assertEqual(pi.read_bytes(), original)
            opencode.write_text('{}', encoding="utf-8")
            agents.sync_settings(report, home, "http://127.0.0.1:8097/v1", 65536, True)
            self.assertEqual(agents.read_json(pi)["providers"]["strata-local"]["models"][0]["maxTokens"], 1000)
            self.assertEqual(opencode.read_text(encoding="utf-8"), '{}')
            self.assertFalse(agents.config_path("codex", home).exists())

    def test_connections_preserve_other_providers_and_use_custom_port(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {}, clear=True):
            home = Path(temporary)
            report = home / "report"
            profile = home / "profile.json"
            profile.write_text(json.dumps({"api_key": ["fixture-key"]}), encoding="utf-8")
            executable = home / "agent.exe"
            executable.touch()
            with patch.object(agents, "find_agent", return_value=executable):
                for name in ["pi", "opencode"]:
                    path = agents.config_path(name, home)
                    path.parent.mkdir(parents=True)
                    key = "providers" if name == "pi" else "provider"
                    original = {key: {"unrelated": {"keep": True}}, "custom_setting": 42}
                    path.write_text(json.dumps(original), encoding="utf-8")
                    agents.connect(name, report, home, profile, 8097, 65536, True)
                    connected = json.loads(path.read_text(encoding="utf-8"))
                    self.assertEqual(connected[key]["unrelated"], original[key]["unrelated"])
                    self.assertEqual(connected["custom_setting"], 42)
                    provider = connected[key]["strata-local"]
                    self.assertEqual(provider["baseUrl"] if name == "pi" else provider["options"]["baseURL"], "http://127.0.0.1:8097/v1")
                    self.assertTrue(list((report / "agents" / "backups").glob("*/" + path.name)))
                agents.connect("codex", report, home, profile, 8097, 65536, False)
                codex = agents.config_path("codex", home).read_text(encoding="utf-8")
                self.assertIn('env_key = "STRATA_API_KEY"', codex)
                self.assertNotIn("fixture-key", codex)
                self.assertIn("8097", codex)
                launcher = (report / "agents" / "codex-strata.ps1").read_text(encoding="utf-8-sig")
                self.assertIn("--profile strata-hub", launcher)
                self.assertNotIn("fixture-key", launcher)
                self.assertFalse((home / ".codex" / "config.toml").exists())
                before = {name: agents.config_path(name, home).read_bytes() for name in agents.CATALOG}
                scanned = agents.scan(report, home, "http://127.0.0.1:8097/v1")
                self.assertEqual([row["context"] for row in scanned], [65536] * 3)
                self.assertTrue(all(row["connected"] for row in scanned))
                self.assertTrue(all(not row["connected"] for row in agents.scan(report, home, "http://127.0.0.1:8098/v1")))
                for name, content in before.items():
                    self.assertEqual(agents.config_path(name, home).read_bytes(), content)
                agents.connect("pi", report, home, profile, 8097, 131072, False)
                self.assertEqual([row["context"] for row in agents.scan(report, home, "http://127.0.0.1:8097/v1")],
                                 [131072, 65536, 65536])


if __name__ == "__main__":
    unittest.main()
