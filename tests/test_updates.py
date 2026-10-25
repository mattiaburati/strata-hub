import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from contextlib import nullcontext
from types import SimpleNamespace
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("update_strata", Path(__file__).parents[1] / "scripts/update_strata.py")
updater = importlib.util.module_from_spec(spec)
spec.loader.exec_module(updater)


class UpdateTests(unittest.TestCase):
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
            package = root / "apps" / "host" / "package.json"
            package.parent.mkdir(parents=True)
            package.write_text('{"version":"0.2.1"}', encoding="utf-8")
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
                if arguments[1:3] == ["merge", "--ff-only"]:
                    package.write_text('{"version":"0.2.2"}', encoding="utf-8")
                return "" if capture else None

            def update_engine(root, report, profiles, expected):
                self.assertEqual(expected, "0.1.47")
                (engine / "BUILD.json").write_text('{"version":"0.1.47"}', encoding="utf-8")
            with patch.object(updater, "require_idle"), patch.object(updater, "run", side_effect=command), \
                 patch.object(updater, "update_engine", side_effect=update_engine):
                updater.install(root, data, report, "v0.2.2", report.parent, "0.1.47")
            self.assertTrue(list((report / "update-backups").glob("*/engine/engine.bin")))
            self.assertEqual(updater.model_manifest(data), before)
            self.assertTrue(any("setup.pip_install" in str(command) for command in calls))
            self.assertFalse(any("--update" in command for command in calls))
            self.assertFalse(any("--start" in command for command in calls))
            self.assertTrue(any(command[1:3] == ["merge", "--ff-only"] for command in calls))

    def test_update_check_compares_source_and_engine_versions_separately(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "engine").mkdir()
            (root / "engine/BUILD.json").write_text('{"version":"0.1.47"}')
            source = root / "apps/host/package.json"
            source.parent.mkdir(parents=True)
            source.write_text('{"version":"0.2.2"}')
            official = SimpleNamespace(check=lambda ctx: {"engine": {
                "status": "current", "offered": {"version": "0.1.47"}}})
            with patch.object(updater, "release", return_value=("v0.2.2", "fixture-url")), \
                 patch.object(updater, "engine_context", return_value=(official, None, None)), \
                 patch.object(updater, "emit") as emit:
                updater.check(root)
                self.assertFalse(emit.call_args.kwargs["available"])
                self.assertEqual(emit.call_args.kwargs["latest"], "0.1.47")
                self.assertEqual(emit.call_args.kwargs["source_latest"], "v0.2.2")
                (root / "engine/BUILD.json").write_text('{"version":"0.1.45"}')
                updater.check(root)
                self.assertTrue(emit.call_args.kwargs["available"])
                (root / "engine/BUILD.json").write_text('{"version":"0.1.47"}')
                source.write_text('{"version":"0.2.1"}')
                updater.check(root)
                self.assertTrue(emit.call_args.kwargs["available"])
                official.check = lambda ctx: {"engine": {"status": "none-for-this-pc", "offered": None,
                                                       "reasons": ["unsupported hardware"]}}
                with self.assertRaisesRegex(RuntimeError, "unsupported hardware"):
                    updater.check(root)

    def test_update_installs_signed_cached_component_into_the_profile_engine_path(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            engine = root / "engine"
            component_root = root / "engine-feed/component"
            for path in (engine, component_root): path.mkdir(parents=True)
            (engine / "BUILD.json").write_text('{"version":"0.1.45"}')
            (engine / "strata.exe").write_bytes(b"old engine")
            (component_root / "BUILD.json").write_text('{"version":"0.1.47","source":"release"}')
            payload = b"signed new engine"
            (component_root / "strata.exe").write_bytes(payload)
            component = SimpleNamespace(root=component_root, version="0.1.47", entrypoint="strata.exe",
                exe=component_root / "strata.exe",
                files=[("strata.exe", hashlib.sha256(payload).hexdigest(), len(payload))])
            offered = {"engine": {"status": "current", "offered": {"version": "0.1.47"}}}
            context = SimpleNamespace(components=root / "engine-feed")
            official = SimpleNamespace(update_lock=lambda path: nullcontext(), State=Mock(),
                open_feed=Mock(), decide_engine=lambda *args: offered["engine"],
                update_engine=Mock(return_value={"status": "current"}), binary_smoke=Mock())
            components = SimpleNamespace(resolve=Mock(return_value=component), package_lib_dirs=lambda path: [str(path)])

            def install_unpacked(stage, destination):
                previous = destination / ".previous"
                previous.mkdir()
                shutil.move(str(destination / "BUILD.json.prev"), previous / "BUILD.json")
                shutil.move(str(destination / "strata.exe"), previous / "strata.exe")
                for item in stage.iterdir(): shutil.move(str(item), destination / item.name)

            import shutil
            setup = SimpleNamespace(install_unpacked=Mock(side_effect=install_unpacked))
            with patch.object(updater, "engine_context", return_value=(official, components, context)), \
                 patch.object(updater, "require_idle"), \
                 patch.object(updater.importlib, "import_module", return_value=setup), \
                 patch.object(updater.subprocess, "run", return_value=SimpleNamespace(stdout="strata 0.1.47\n")) as binary:
                offered["engine"]["offered"]["version"] = "0.1.48"
                with self.assertRaisesRegex(RuntimeError, "changed since"):
                    updater.update_engine(root, root, root, "0.1.47")
                official.update_engine.assert_not_called()
                offered["engine"]["offered"]["version"] = "0.1.47"
                official.update_engine.side_effect = RuntimeError("signature rejected")
                with self.assertRaisesRegex(RuntimeError, "signature rejected"):
                    updater.update_engine(root, root, root, "0.1.47")
                self.assertEqual((engine / "strata.exe").read_bytes(), b"old engine")
                setup.install_unpacked.assert_not_called()
                official.update_engine.side_effect = None
                updater.update_engine(root, root, root, "0.1.47")
                self.assertEqual((engine / "strata.exe").read_bytes(), payload)
                self.assertEqual((engine / ".previous/strata.exe").read_bytes(), b"old engine")
                self.assertEqual(binary.call_args.args[0][0], str(engine / "strata.exe"))
                # Source-only updates keep the existing rollback engine when the package is already installed.
                updater.update_engine(root, root, root, "0.1.47")
                self.assertEqual(setup.install_unpacked.call_count, 1)

    def test_stable_version_comparison_and_input_validation(self):
        self.assertGreater(updater.version("v0.1.44.1"), updater.version("0.1.43"))
        self.assertEqual(updater.version("v0.1.44"), updater.version("0.1.44.0"))
        for invalid in ["main", "v0.1.44-rc1", "--upload-pack=bad"]:
            with self.assertRaises(ValueError):
                updater.version(invalid)
