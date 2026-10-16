"""Check stable Strata releases and run the official updater without starting inference."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import urllib.request

REPOSITORY = "Niko1221/Strata"
PREFIX = "HUB_EVENT "


def emit(kind, **fields):
    print(PREFIX + json.dumps({"kind": kind, **fields}), flush=True)


def version(value):
    if not re.fullmatch(r"v?\d+\.\d+\.\d+(?:\.\d+)?", value):
        raise ValueError(f"Unsupported stable version: {value}")
    return tuple(map(int, value.removeprefix("v").split("."))) + ((0,) if value.count(".") == 2 else ())


def installed(root):
    return str(json.loads((root / "engine" / "BUILD.json").read_text(encoding="utf-8"))["version"])


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


def install(root, data, report, tag, profiles):
    version(tag)
    if version(tag) <= version(installed(root)):
        raise RuntimeError("The selected release is already installed or older than this engine.")
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
    manifest = {"commit": commit, "engine_version": installed(root), "target": tag,
                "models": models_before, "protected": protected_before}
    (backup / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    (backup / "RESTORE.txt").write_text(
        f"Stop Strata before restoring. This is a manual backup; no automatic rollback is performed.\n"
        f"Source commit: {commit}\nRestore source with: git -C \"{root}\" switch --detach {commit}\n"
        "Restore the backed-up engine directory and the profile files listed in manifest.json.\n"
        "Python packages are updated by official setup and are not included in this backup.\n", encoding="utf-8")
    emit("progress", message="Fetching the stable release", backup=str(backup))
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
    emit("progress", message="Updating engine and Python dependencies")
    run([sys.executable, "-u", str(root / "setup.py"), "--update", "--data-dir", str(data), "--yes"], root)
    emit("progress", message="Verifying the installed version and preserved data")
    if version(installed(root)) < version(tag):
        raise RuntimeError(f"The official updater kept engine {installed(root)}; expected {tag}. See the update log.")
    if protected_files(report, profiles) != protected_before:
        raise RuntimeError("A protected hub profile or launcher changed. Review the backup before starting a model.")
    if model_manifest(data) != models_before:
        raise RuntimeError("Model weight metadata changed. Review the backup manifest before starting a model.")
    emit("complete", current=installed(root), message="Strata updated. Start your model when ready.", backup=str(backup))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["check", "install"])
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, required=True)
    parser.add_argument("--tag")
    args = parser.parse_args()
    if args.mode == "check":
        tag, url = release()
        emit("release", current=installed(args.root), latest=tag, url=url,
             available=version(tag) > version(installed(args.root)),
             message=(f"Update available: {tag}" if version(tag) > version(installed(args.root)) else "Strata is up to date."))
    else:
        if args.tag is None:
            raise ValueError("A release tag is required for installation")
        install(args.root, args.data, args.report, args.tag, args.profiles)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        emit("error", message=str(error))
        sys.exit(1)
