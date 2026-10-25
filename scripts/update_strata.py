"""Update Strata source and its independently versioned, signed engine without inference."""
import argparse
import datetime
import hashlib
import importlib
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import urllib.request

REPOSITORY = "Niko1221/Strata"
PREFIX = "HUB_EVENT "
FEED = f"https://github.com/{REPOSITORY}/releases/latest/download/strata-feed.json"


def emit(kind, **fields):
    print(PREFIX + json.dumps({"kind": kind, **fields}), flush=True)


def version(value):
    if not re.fullmatch(r"v?\d+\.\d+\.\d+(?:\.\d+)?", value):
        raise ValueError(f"Unsupported stable version: {value}")
    return tuple(map(int, value.removeprefix("v").split("."))) + ((0,) if value.count(".") == 2 else ())


def installed(root):
    return str(json.loads((root / "engine" / "BUILD.json").read_text(encoding="utf-8"))["version"])


def file_hash(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def source_version(root):
    value = json.loads((root / "apps" / "host" / "package.json").read_text(encoding="utf-8"))["version"]
    version(value)
    return value


def engine_context(root):
    if not (root / "strata_control" / "updater.py").is_file():
        raise RuntimeError("This Strata checkout lacks signed engine updates. Update its source before using this Hub updater.")
    sys.path.insert(0, str(root))
    official = importlib.import_module("strata_control.updater")
    components = importlib.import_module("strata_control.engine_component")
    signing = importlib.import_module("strata_control.signing")

    def progress(event):
        if event.get("event") == "phase":
            emit("progress", message=event["message"])
        elif event.get("event") == "progress":
            emit("progress", message=f"Downloading engine: {event['percent']}%")

    def binary_test(stage, component, context):
        result = official.binary_smoke(component)
        result["detail"] = "Engine version and DLL loading checked; inference was not started."
        return result

    context = official.Context(components=root / "engine-feed", source=FEED,
        trusted=signing.load_trusted_keys(), hardware=official.detect_hardware(),
        emit=progress, test_start=binary_test)
    return official, components, context


def offered_engine(report):
    engine = report["engine"]
    package = engine["offered"]
    if package is None or engine["status"] not in ("available", "install", "current"):
        detail = engine.get("note") or "; ".join(engine.get("reasons", [])) or engine["status"]
        raise RuntimeError(f"No usable signed engine update: {detail}")
    version(package["version"])
    return package


def check(root):
    tag, url = release()
    source = source_version(root)
    official, _, context = engine_context(root)
    package = offered_engine(official.check(context))
    current = installed(root)
    latest = package["version"]
    engine_available = version(latest) > version(current)
    source_available = version(tag) > version(source)
    available = engine_available or source_available
    message = f"Engine {current} / latest {latest}. Source {source} / latest {tag.removeprefix('v')}."
    emit("release", current=current, latest=latest, url=url, source_current=source,
         source_latest=tag, available=available,
         message=("Update available. " if available else "Strata is up to date. ") + message)


def release():
    request = urllib.request.Request(
        f"https://api.github.com/repos/{REPOSITORY}/releases/latest",
        headers={"User-Agent": "StrataHub/1.0", "Accept": "application/vnd.github+json"},
    )
    with urllib.request.urlopen(request, timeout=15) as response:
        raw = response.read(1024 * 1024 + 1)
    if len(raw) > 1024 * 1024:
        raise ValueError("GitHub release response exceeds 1 MiB")
    result = json.loads(raw)
    tag = result["tag_name"]
    version(tag)
    if result["draft"] or result["prerelease"]:
        raise ValueError("Expected a published stable release")
    url = f"https://github.com/{REPOSITORY}/releases/tag/{tag}"
    if result["html_url"] != url:
        raise ValueError("Unexpected release URL")
    return tag, url


def run(arguments, cwd, capture=False):
    result = subprocess.run(arguments, cwd=cwd, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE if capture else None, stderr=subprocess.STDOUT,
                            text=True, encoding="utf-8", errors="replace", check=True)
    return result.stdout.strip() if capture else None


def require_idle(root, report, profiles):
    # Python can be loading before the HTTP listener or engine exists.
    record_path = report / "owned-strata.json"
    if record_path.exists():
        record = json.loads(record_path.read_text(encoding="utf-8"))
        import psutil  # Already required by the installed Strata environment.
        pid = int(record["pid"])
        if psutil.pid_exists(pid):
            process = psutil.Process(pid)
            created = process.create_time() * 10_000_000 + 116444736000000000
            if abs(created - int(record["created"])) < 10_000:
                raise RuntimeError("Strata is running or starting. Stop the model before updating.")
    ports = set()
    for path in list(profiles.glob("strata-*.json")) + list((report / "profiles-run").glob("*.json")):
        config = json.loads(path.read_text(encoding="utf-8-sig"))
        if "args" in config:
            ports.add(int(config["port"]))
    for port in ports:
        with socket.socket() as connection:
            connection.settimeout(0.5)
            if connection.connect_ex(("127.0.0.1", port)) == 0:
                raise RuntimeError(f"Port {port} is in use. Stop the server before updating.")
    # Windows refuses write access to a running engine, including external launchers.
    for engine in root.glob("engine*/strata*.exe"):
        with engine.open("r+b"):
            pass


def model_manifest(data):
    if not data.is_dir():
        raise RuntimeError(f"Model data directory is missing: {data}")
    # Metadata only: never read hundreds of GB of weights just to check an update.
    # Draft vocabulary files are derived assets that official setup may refresh.
    return {str(p.relative_to(data)): [p.stat().st_size, p.stat().st_mtime_ns]
            for p in data.rglob("*") if p.is_file() and (p.suffix == ".gguf" or p.stat().st_size >= 100 * 1024 * 1024)}


def protected_files(report, profiles):
    files = list(profiles.glob("strata-*.json")) + list(profiles.glob("*.bat"))
    files += list((report / "profiles-run").glob("*.json"))
    files += [report / "hub-profiles.json"]
    return {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in files if p.is_file()}


def install(root, data, report, tag, profiles, engine_version):
    version(tag)
    version(engine_version)
    if version(engine_version) < version(installed(root)) or version(tag) < version(source_version(root)):
        raise RuntimeError("The selected update would downgrade Strata. Check for updates again.")
    require_idle(root, report, profiles)
    origin = run(["git", "remote", "get-url", "origin"], root, True).removesuffix(".git")
    if origin != f"https://github.com/{REPOSITORY}":
        raise RuntimeError("The Strata checkout does not use the official GitHub origin.")
    if run(["git", "status", "--porcelain", "--untracked-files=no"], root, True):
        raise RuntimeError("Strata has local tracked changes. Resolve them before updating.")
    commit = run(["git", "rev-parse", "HEAD"], root, True)
    emit("progress", message="Backing up the engine and configuration")
    backup = report / "update-backups" / datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    backup.mkdir(parents=True, exist_ok=False)
    models_before = model_manifest(data)
    protected_before = protected_files(report, profiles)
    shutil.copytree(root / "engine", backup / "engine", ignore=shutil.ignore_patterns(".previous", ".swap"))
    for index, name in enumerate(protected_before):
        shutil.copy2(name, backup / f"profile-{index:03d}-{Path(name).name}")
    for path in root.glob("strata-*.json"):
        shutil.copy2(path, backup / ("source-" + path.name))
    manifest = {"commit": commit, "engine_version": installed(root), "target": tag, "target_engine": engine_version,
                "models": models_before, "protected": protected_before}
    (backup / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    (backup / "RESTORE.txt").write_text(
        f"Stop Strata before restoring. This is a manual backup; no automatic rollback is performed.\n"
        f"Source commit: {commit}\nRestore source with: git -C \"{root}\" switch --detach {commit}\n"
        "Restore the backed-up engine directory and the profile files listed in manifest.json.\n"
        "Python packages are updated by official setup and are not included in this backup.\n", encoding="utf-8")
    source_changed = version(tag) > version(source_version(root))
    if source_changed:
        emit("progress", message="Fetching the stable source release", backup=str(backup))
        run(["git", "fetch", "--no-tags", "origin", f"refs/tags/{tag}"], root)
        target = run(["git", "rev-parse", "FETCH_HEAD"], root, True)
        try:
            run(["git", "merge-base", "--is-ancestor", commit, target], root, True)
        except subprocess.CalledProcessError as error:
            raise RuntimeError("The checkout cannot fast-forward to this stable release. Review its source history before updating.") from error
        # Keep a reachable reference to the old source. Never reset, stash, or force a merge.
        run(["git", "branch", "hub-backup-" + backup.name, commit], root)
        require_idle(root, report, profiles)
        emit("progress", message="Updating Strata source")
        run(["git", "merge", "--ff-only", target], root)
        if version(source_version(root)) != version(tag):
            raise RuntimeError("The source release version does not match its selected tag.")
        emit("progress", message="Updating official Python dependencies")
        # Use setup's dependency selection without its legacy engine/config update path.
        run([sys.executable, "-u", "-c", "import setup; setup.pip_install("
             "setup.requirement_lines() if setup.REQUIREMENTS.exists() else setup.PY_PACKAGES, "
             "'Strata Python dependencies')"], root)
    update_engine(root, report, profiles, engine_version)
    emit("progress", message="Verifying the installed version and preserved data")
    if version(installed(root)) != version(engine_version):
        raise RuntimeError(f"Engine {installed(root)} is installed; expected {engine_version}. See the update log.")
    if protected_files(report, profiles) != protected_before:
        raise RuntimeError("A protected hub profile or launcher changed. Review the backup before starting a model.")
    if model_manifest(data) != models_before:
        raise RuntimeError("Model weight metadata changed. Review the backup manifest before starting a model.")
    emit("complete", current=installed(root), latest=engine_version, source_current=source_version(root),
         source_latest=tag, message="Strata updated. Start your model when ready.", backup=str(backup))


def update_engine(root, report, profiles, expected):
    official, components, context = engine_context(root)
    with official.update_lock(context.components):
        state = official.State(context.components)
        feed = official.open_feed(context, state)
        package = offered_engine({"engine": official.decide_engine(feed, context, state)})
        if version(package["version"]) != version(expected):
            raise RuntimeError("The offered engine changed since the update check. Check for updates again.")
        state.save()
        result = official.update_engine(context, feed, state)
        state.save()
        if result["status"] not in ("updated", "current"):
            raise RuntimeError(f"The official engine updater did not install the selected engine: {result['status']}")
        component = components.resolve(context.components, full=True)
        if version(component.version) != version(expected):
            raise RuntimeError("The signed component does not match the selected engine.")
        if component.entrypoint != "strata.exe":
            raise RuntimeError("The signed engine uses an unsupported executable layout for these Hub profiles.")
        # A cached component can be current while the legacy profile executable is old.
        official.binary_smoke(component)
        require_idle(root, report, profiles)
        eng = root / "engine"
        if version(installed(root)) != version(expected) or file_hash(eng / "strata.exe") != file_hash(component.exe):
            setup = importlib.import_module("setup")
            tmp = eng / "_unpack"
            if tmp.exists():
                raise RuntimeError(f"An earlier engine staging folder still exists: {tmp}")
            shutil.copytree(component.root, tmp)
            meta = json.loads((tmp / "BUILD.json").read_text(encoding="utf-8"))
            meta["source"] = "prebuilt"
            (tmp / "BUILD.json").write_text(json.dumps(meta, indent=1), encoding="utf-8")
            (eng / "BUILD.json").replace(eng / "BUILD.json.prev")
            setup.install_unpacked(tmp, eng)
            tmp.rmdir()
        # Verify the actual executable used by existing Hub profiles, not only the component cache.
        env = dict(os.environ)
        env["PATH"] = os.pathsep.join(components.package_lib_dirs(eng)) + os.pathsep + env.get("PATH", "")
        result = subprocess.run([str(eng / "strata.exe"), "--version"], cwd=root, env=env,
                                stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=60, check=True)
        if result.stdout.strip() != f"strata {component.version}":
            raise RuntimeError(f"Unexpected installed engine version: {result.stdout.strip()}")
        for relative, digest, size in component.files:
            if relative == component.entrypoint:
                exe = eng / relative
                if exe.stat().st_size != size or file_hash(exe) != digest:
                    raise RuntimeError("The installed executable differs from the signed package.")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["check", "install"])
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--tag")
    parser.add_argument("--engine-version")
    args = parser.parse_args()
    if args.mode == "check":
        check(args.root)
    else:
        if args.tag is None or args.engine_version is None:
            raise ValueError("A source release tag and an engine version are required for installation")
        install(args.root, args.data, args.report, args.tag, args.profiles, args.engine_version)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        emit("error", message=str(error))
        sys.exit(1)
