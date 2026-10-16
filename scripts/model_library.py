"""On-demand GGUF discovery, installed-Strata inspection and a bounded-memory model library."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import time
import urllib.parse
import urllib.request

CHUNK = 1024 * 1024
HEADER_LIMIT = 64 * CHUNK
MTP_EXPERT_BYTES = 707788800
SHARD = re.compile(r"^(.*)-(\d{5})-of-(\d{5})\.gguf$", re.I)


def emit(kind, message, **fields):
    print("HUB_EVENT " + json.dumps({"kind": kind, "message": message, **fields}), flush=True)


def read_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("w", encoding="utf-8") as output:
        json.dump(value, output, indent=2)
        output.flush()
        os.fsync(output.fileno())
    os.replace(temporary, path)


def repo_id(value):
    value = value.strip().rstrip("/")
    if value.startswith("https://"):
        parsed = urllib.parse.urlsplit(value)
        if parsed.netloc != "huggingface.co" or parsed.query or parsed.fragment:
            raise ValueError("Use a Hugging Face model repository URL or owner/repository.")
        value = parsed.path.lstrip("/")
    if not re.fullmatch(r"[A-Za-z0-9_-]+/[A-Za-z0-9_.-]+", value) or ".." in value:
        raise ValueError("Use a repository root, for example https://huggingface.co/owner/model.")
    return value


def relative_file(name):
    path = PurePosixPath(name)
    if not name or path.is_absolute() or "\\" in name or ":" in name or any(ord(c) < 32 or c in '<>"|?*' for c in name):
        raise ValueError("Unsafe model filename in repository metadata.")
    if any(p in ("", ".", "..") or p.endswith((".", " ")) or
           re.fullmatch(r"(?i)(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?", p) for p in name.split("/")):
        raise ValueError("Unsupported model filename in repository metadata.")
    return path


def token():
    if os.environ.get("HF_TOKEN"):
        return os.environ["HF_TOKEN"].strip()
    home = Path(os.environ["HF_HOME"]) if "HF_HOME" in os.environ else Path.home() / ".cache/huggingface"
    path = Path(os.environ["HF_TOKEN_PATH"]) if "HF_TOKEN_PATH" in os.environ else home / "token"
    return path.read_text(encoding="utf-8").strip() if path.is_file() else None


class Redirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        if urllib.parse.urlsplit(new_url).scheme != "https":
            raise ValueError("Hugging Face redirected to a non-HTTPS URL.")
        redirected = super().redirect_request(request, response, code, message, headers, new_url)
        if urllib.parse.urlsplit(new_url).netloc != urllib.parse.urlsplit(request.full_url).netloc:
            redirected.remove_header("Authorization")
        return redirected


def open_url(url, start=None, length=None):
    headers = {"User-Agent": "StrataHub/1.0", "Accept-Encoding": "identity"}
    credential = token()
    if credential:
        headers["Authorization"] = "Bearer " + credential
    if start is not None:
        headers["Range"] = f"bytes={start}-" + (str(start + length - 1) if length is not None else "")
    return urllib.request.build_opener(Redirect()).open(urllib.request.Request(url, headers=headers), timeout=60)


def file_url(repo, revision, name):
    relative_file(name)
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("Expected an immutable Hugging Face commit.")
    return f"https://huggingface.co/{repo}/resolve/{revision}/{urllib.parse.quote(name, safe='/')}"


def require_range(response, start, length, total):
    actual = response.headers.get("Content-Range", "")
    expected = f"bytes {start}-{start + length - 1}/{total}"
    if response.status != 206 or actual != expected:
        raise ValueError("Server did not honor the requested byte range; refusing an unbounded transfer.")


class HeaderSource:
    def __init__(self, repo, revision, file, budget=None):
        self.name, self.size = file["name"], file["size"]
        self.url = file_url(repo, revision, self.name)
        self.budget = budget if budget is not None else [HEADER_LIMIT]

    def read(self, start, length):
        length = min(length, self.size - start)
        if start < 0 or length <= 0 or start + length > HEADER_LIMIT or length > self.budget[0]:
            raise ValueError("GGUF header exceeds the 64 MiB inspection limit.")
        with open_url(self.url, start, length) as response:
            require_range(response, start, length, self.size)
            result = response.read(length + 1)
        if len(result) != length:
            raise ValueError("GGUF header range has an unexpected size.")
        self.budget[0] -= length
        return result


def inspector(root):
    path = root / "tools/strata_inspect.py"
    table = root / "data/gguf_fingerprints.json"
    if not path.is_file() or not table.is_file():
        raise RuntimeError("This Strata installation has no GGUF inspector. Update Strata first.")
    spec = importlib.util.spec_from_file_location("hub_strata_inspect", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    signature = hashlib.sha256(path.read_bytes() + table.read_bytes() + (root / "engine/BUILD.json").read_bytes()).hexdigest()
    return module, signature


def group_files(siblings):
    groups = {}
    for item in siblings:
        name = item["rfilename"]
        if not is_model_file(name):
            continue
        relative_file(name)
        match = SHARD.fullmatch(name)
        variant = match[1] if match else name[:-5]
        lfs = item.get("lfs") or {}
        file = {"name": name, "size": int(item.get("size") or lfs.get("size") or 0), "sha256": lfs.get("sha256", "")}
        groups.setdefault(variant, []).append(file)
    if len(groups) > 128:
        raise ValueError("Repository has more than 128 GGUF variants; use a smaller repository.")
    return groups


def is_model_file(name):
    return name.lower().endswith(".gguf") and not any(part in name.lower() for part in ("mmproj", "mtp-", "draft-", "refusal-direction"))


def complete_shards(files):
    matches = [SHARD.fullmatch(f["name"]) for f in files]
    if not any(matches):
        return len(files) == 1
    return all(matches) and len({m[3] for m in matches}) == 1 and sorted(int(m[2]) for m in matches) == list(range(1, int(matches[0][3]) + 1))


def hash_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for block in iter(lambda: file.read(CHUNK), b""):
            digest.update(block)
    return digest.hexdigest()


def scan_repo(root, report, value):
    repo = repo_id(value)
    module, signature = inspector(root)
    emit("progress", f"Reading {repo} metadata")
    with open_url(f"https://huggingface.co/api/models/{repo}?blobs=true") as response:
        raw = response.read(8 * CHUNK + 1)
    if len(raw) > 8 * CHUNK:
        raise ValueError("Repository metadata exceeds 8 MiB.")
    metadata = json.loads(raw)
    revision = metadata["sha"]
    groups = group_files(metadata["siblings"])
    if not groups:
        raise ValueError("No model GGUF files found in this repository.")
    catalog = {"version": 1, "repo": repo, "revision": revision, "inspector": signature,
               "engine": read_json(root / "engine/BUILD.json")["version"], "variants": []}
    for index, (name, files) in enumerate(sorted(groups.items())):
        emit("progress", f"Inspecting {index + 1}/{len(groups)}: {name}")
        files.sort(key=lambda f: f["name"])
        quant = re.search(r"(?i)(UD-)?(?:IQ\d_[A-Z0-9]+|Q\d_[A-Z0-9]+(?:_[A-Z]+)?|BF16|F16|F32)", name)
        row = {"id": hashlib.sha256(name.encode()).hexdigest()[:16], "name": name, "quant": quant[0].upper() if quant else "Mixed/unknown",
               "files": files, "size": sum(f["size"] for f in files), "downloadable": False}
        try:
            if not complete_shards(files):
                raise ValueError("Incomplete or inconsistent GGUF shard set.")
            if any(f["size"] <= 0 or not re.fullmatch(r"[a-f0-9]{64}", f["sha256"]) for f in files):
                raise ValueError("Missing file sizes or SHA-256 metadata.")
            budget = [HEADER_LIMIT]
            arch, tensors = module.read_headers([HeaderSource(repo, revision, f, budget) for f in files])
            fp = module.fingerprint(arch, tensors)
            kind, detail = module.verdict(fp, module.load_fingerprints())
            row.update(verdict=kind, detail=detail, arch=arch, expert_quants=list(fp["expert_bytes"]),
                       downloadable=kind in ("known", "layout"))
            del tensors, fp
        except (OSError, ValueError, EOFError, KeyError) as error:
            row.update(verdict="error", detail=str(error))
        catalog["variants"].append(row)
        write_json(report / "model-catalog.json", catalog)
        emit("catalog", f"Inspected {index + 1}/{len(groups)} variants", catalog_generation=time.time_ns())
    errors = sum(row["verdict"] == "error" for row in catalog["variants"])
    emit("complete", f"{len(groups)} variants inspected; {errors} inspection errors. Compatibility uses Strata {catalog['engine']}.")


def library(report):
    file = report / "model-library.json"
    return read_json(file) if file.exists() else {"version": 1, "models": []}


def safe_path(folder, name):
    relative_file(name)
    folder = folder.absolute()
    path = folder.joinpath(*PurePosixPath(name).parts)
    if path.resolve() != path.absolute() or not path.resolve().is_relative_to(folder.resolve()):
        raise ValueError("Model paths cannot contain symlinks, junctions or parent traversal.")
    return path


def model_folder(folder, root, report, profiles):
    folder = folder.absolute()
    if folder.resolve() != folder or folder == Path(folder.anchor) or any(path.resolve().is_relative_to(folder) for path in (root, report, profiles)):
        raise ValueError("Choose a dedicated model folder without junctions or symlinks.")
    return folder


def refresh(report):
    emit("library", "Model library updated", library_generation=time.time_ns())


def download(root, report, models, variant_id, destination):
    catalog = read_json(report / "model-catalog.json")
    _, signature = inspector(root)
    if catalog["inspector"] != signature:
        raise ValueError("Strata changed since the scan. Analyze the repository again.")
    row = next((r for r in catalog["variants"] if r["id"] == variant_id), None)
    if row is None:
        raise ValueError("The selected GGUF is not in the current catalog. Analyze the repository again.")
    if not row["downloadable"] or row["verdict"] not in ("known", "layout"):
        raise ValueError("This GGUF has no supported Strata layout; download is disabled.")
    base = destination.absolute()
    if not base.is_dir() or base.resolve() != base:
        raise ValueError("Choose an existing download directory without junctions or symlinks.")
    identity = hashlib.sha256((catalog["repo"] + catalog["revision"] + row["name"]).encode()).hexdigest()[:20]
    folder = base / (re.sub(r"[^A-Za-z0-9_.-]", "_", PurePosixPath(row["name"]).name)[:80] + "-" + identity[:8])
    state = library(report)
    entry = next((e for e in state["models"] if e["id"] == identity), None)
    if entry is None:
        if folder.exists():
            raise ValueError("Download folder already exists outside this library; it will not be overwritten.")
        entry = {"id": identity, "name": row["name"], "repo": catalog["repo"], "revision": catalog["revision"],
                 "path": str(folder), "files": row["files"], "size": row["size"], "quant": row["quant"], "state": "partial"}
        state["models"].append(entry)
        folder.mkdir()
        write_json(report / "model-library.json", state)
    elif Path(entry["path"]) != folder:
        raise ValueError("This model is registered in another location. Select that location to resume it.")
    refresh(report)
    remaining = sum(f["size"] - min(safe_path(folder, f["name"] + ".part").stat().st_size if safe_path(folder, f["name"] + ".part").exists() else 0, f["size"])
                    for f in entry["files"] if not safe_path(folder, f["name"]).exists())
    if shutil.disk_usage(folder).free < remaining:
        raise ValueError("Insufficient free space for the selected GGUF shards.")
    completed = 0
    for file in entry["files"]:
        path = safe_path(folder, file["name"])
        if path.exists():
            if path.stat().st_size != file["size"] or hash_file(path) != file["sha256"]:
                raise ValueError("A completed model file has changed; refusing to overwrite it.")
            completed += file["size"]
            continue
        path.parent.mkdir(parents=True, exist_ok=True)
        part = safe_path(folder, file["name"] + ".part")
        offset = part.stat().st_size if part.exists() else 0
        if offset > file["size"]:
            raise ValueError("Partial download is larger than the expected file.")
        digest = hashlib.sha256()
        if offset:
            emit("progress", f"Checking partial download: {file['name']}")
            with part.open("rb") as previous:
                for block in iter(lambda: previous.read(CHUNK), b""):
                    digest.update(block)
        if offset < file["size"]:
            url = file_url(entry["repo"], entry["revision"], file["name"])
            with open_url(url, offset, file["size"] - offset) as response:
                require_range(response, offset, file["size"] - offset, file["size"])
                with part.open("ab") as output:
                    last = 0.0
                    while offset < file["size"]:
                        block = response.read(min(CHUNK, file["size"] - offset))
                        if not block:
                            raise EOFError("Download ended early. Select Download again to resume the partial files.")
                        output.write(block)
                        digest.update(block)
                        offset += len(block)
                        if time.monotonic() - last >= 1:
                            emit("progress", f"Downloading {100 * (completed + offset) / entry['size']:.1f}%: {file['name']}",
                                 downloaded=completed + offset, total=entry["size"])
                            last = time.monotonic()
                    output.flush()
                    os.fsync(output.fileno())
        if digest.hexdigest() != file["sha256"]:
            raise ValueError("GGUF SHA-256 mismatch. The partial file was retained for manual inspection.")
        os.replace(part, path)
        completed += file["size"]
    entry["state"] = "downloaded"
    entry["verified_files"] = [{"name": file["name"], "size": file["size"],
                                "mtime_ns": safe_path(folder, file["name"]).stat().st_mtime_ns} for file in entry["files"]]
    write_json(report / "model-library.json", state)
    refresh(report)
    emit("progress", "Download verified. GGUF files are ready for preparation.")
    return identity


def run_tool(root, tool, *arguments, env=None):
    emit("progress", f"Preparing model: {tool}")
    subprocess.run([sys.executable, "-u", str(root / "tools" / tool), *map(str, arguments)],
                   cwd=root, env=env, check=True)


def load_setup(root):
    spec = importlib.util.spec_from_file_location("hub_strata_setup", root / "setup.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def require_files(folder, names):
    for name in names:
        path = safe_path(folder, name)
        if not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"Prepared model asset is missing or empty: {path}")


def validate_mtp(rt):
    require_files(rt, ("experts.bin", "dense.bin", "dense.txt", "draft_vocab.bin"))
    if (rt / "experts.bin").stat().st_size != MTP_EXPERT_BYTES:
        raise ValueError("The shared MTP expert file has an invalid size; inspect it before preparing again.")
    end = 0
    for line in (rt / "dense.txt").read_text(encoding="utf-8").splitlines():
        name, kind, rows, cols, offset, length = line.split()
        rows, cols, offset, length = map(int, (rows, cols, offset, length))
        if kind not in ("q8_0", "bf16", "f32") or min(rows, cols, length) <= 0 or offset < end:
            raise ValueError("The shared MTP dense index is invalid.")
        end = offset + length
    if not end or end != (rt / "dense.bin").stat().st_size:
        raise ValueError("The shared MTP dense file is incomplete or does not match its index.")


def prepare(root, report, models, profiles, model_id, helper, template=None):
    require_idle(root, report, profiles, helper)
    if not re.fullmatch(r"(?:existing-)?[a-f0-9]{20}", model_id):
        raise ValueError("Invalid local model identifier.")
    state = library(report)
    entry = next((row for row in state["models"] if row["id"] == model_id), None)
    if entry is None:
        entry = next((row for row in existing_models(models, state["models"], profiles, root)
                      if row["id"] == model_id), None)
        if entry is None:
            raise ValueError("The selected local model no longer exists. Scan local again.")
        state["models"].append(entry)
    if entry["state"] == "partial":
        raise ValueError("Complete the GGUF download before preparing it.")
    folder = model_folder(Path(entry["path"]), root, report, profiles)
    references = referenced(folder, profiles, report, root)
    if references:
        entry["state"] = "ready"
        write_json(report / "model-library.json", state)
        inventory(report, models, profiles, root)
        emit("complete", "This model already has a Strata profile. Select it in Models.", profiles_generation=time.time_ns())
        return
    files = sorted(entry["files"], key=lambda row: row["name"])
    if not complete_shards(files):
        raise ValueError("The model requires a complete GGUF shard set.")
    paths = [safe_path(folder, row["name"]) for row in files]
    for row, path in zip(files, paths):
        if not path.is_file() or path.stat().st_size != row["size"]:
            raise ValueError(f"Model shard is missing or has changed: {path}")
    if "verified_files" in entry:
        current = [{"name": row["name"], "size": path.stat().st_size, "mtime_ns": path.stat().st_mtime_ns} for row, path in zip(files, paths)]
        if sorted(current, key=lambda row: row["name"]) != sorted(entry["verified_files"], key=lambda row: row["name"]):
            raise ValueError("GGUF files changed after download verification. Select Download & prepare to verify them again.")
    module, signature = inspector(root)
    budget = [HEADER_LIMIT]
    class LocalHeader:
        def __init__(self, path):
            self.path, self.name, self.size = path, path.name, path.stat().st_size

        def read(self, start, length):
            length = min(length, self.size - start)
            if start < 0 or length <= 0 or start + length > HEADER_LIMIT or length > budget[0]:
                raise ValueError("GGUF header exceeds the 64 MiB inspection limit.")
            with self.path.open("rb") as source:
                source.seek(start)
                data = source.read(length)
            budget[0] -= length
            return data
    emit("progress", "Inspecting downloaded GGUF headers")
    sources = [LocalHeader(path) for path in paths]
    arch, tensors = module.read_headers(sources)
    fp = module.fingerprint(arch, tensors)
    table = module.load_fingerprints()
    kind, detail = module.verdict(fp, table)
    if arch != "qwen4exp" or kind not in ("known", "layout"):
        raise ValueError(f"Automatic preparation is unavailable for this layout: {detail}")
    matching = [name for name, row in table.items() if not name.startswith("_") and row["expert_bytes"] == fp["expert_bytes"]]
    coder = bool(matching) and all(name.startswith("coder/") for name in matching)
    ple = next((source.path for source in sources if any(t["name"] == "per_layer_token_embd.weight"
                for t in module.qs.read_gguf(source)[1])), None)
    if ple is None:
        raise ValueError("The model has no per-layer embedding table required by Strata.")
    # Never reuse another fine-tune's dense weights or tokenizer, even if its geometry matches.
    pack = models / "packs" / ("hub-" + model_id)
    profile = profiles / ("strata-hub-" + model_id + ".json")
    marker = pack / "manifest.json"
    source_stamp = [{"path": str(path), "size": path.stat().st_size, "mtime_ns": path.stat().st_mtime_ns} for path in paths]
    identity = {"hub_model_id": model_id, "sources": source_stamp, "inspector": signature}
    if profile.exists():
        raise ValueError("The target model profile already exists; it will not be overwritten.")
    if pack.exists() and (not marker.is_file() or any(read_json(marker).get(key) != value for key, value in identity.items())):
        raise ValueError("The preparation folder belongs to different or changed model files. Inspect it before preparing again.")
    pack = model_folder(pack.absolute(), root, report, profiles)
    setup = load_setup(root)
    for tool in ("iq_pack.py", "mtp_fetch.py", "mtp_pack.py", "mtp_rt.py"):
        if not (root / "tools" / tool).is_file():
            raise ValueError(f"Required Strata preparation tool is missing: {tool}")
    template_config = read_json(template) if template else None
    engine = Path(template_config["exe"]) if template_config else root / "engine" / setup.EXE
    if not engine.is_file() or engine.resolve().parent.parent != root.resolve():
        raise ValueError("The configured Strata engine is missing or outside this installation.")
    # Official setup provides the version-pinned gguf-py dependency, without downloading a model or starting inference.
    emit("progress", "Checking Strata packing dependencies")
    llama = setup.get_llama_cpp()
    env = dict(os.environ, STRATA_GGUF_PY=str(llama / "gguf-py"), PYTHONUNBUFFERED="1")
    pack.mkdir(parents=True, exist_ok=True)
    write_json(marker, {**identity, "complete": False})
    entry.update(state="preparing", pack=str(pack), profile=str(profile))
    write_json(report / "model-library.json", state)
    refresh(report)
    run_tool(root, "iq_pack.py", "--gguf", paths[0], "--out", pack, "--compat-bf16", env=env)
    require_files(pack, ("index.txt", "dense.bin", "native_experts.txt", "tokenizer/vocab.json", "tokenizer/chat_template.jinja"))
    mtp = models / "mtp"
    rt = mtp / "rt"
    if rt.exists() and rt.resolve() != rt.absolute():
        raise ValueError("The shared MTP directory cannot contain junctions or symlinks.")
    if (rt / "experts.bin").exists():
        if (mtp / "tensors").is_dir():
            run_tool(root, "mtp_fetch.py", "verify", "--out", mtp, env=env)
        require_files(rt, ("experts.bin", "dense.bin", "dense.txt", "draft_vocab.bin"))
    else:
        emit("progress", "Preparing shared MTP draft layer (first setup downloads about 5 GB)")
        run_tool(root, "mtp_fetch.py", "fetch", "--out", mtp, env=env)
        run_tool(root, "mtp_fetch.py", "verify", "--out", mtp, env=env)
        run_tool(root, "mtp_pack.py", "--src", mtp, "--experts", "q2_0", "--out", mtp / "mtp-q2_0.gguf", env=env)
        run_tool(root, "mtp_rt.py", "--gguf", mtp / "mtp-q2_0.gguf", "--out", rt, env=env)
        require_files(root / "data", ("draft_vocab.bin",))
        shutil.copyfile(root / "data/draft_vocab.bin", rt / "draft_vocab.bin")
    validate_mtp(rt)
    # Preserve compatible resource settings and the local endpoint, never model-specific steering or vision assets.
    options = {"--expert-cache": "auto", "--prefill": "auto", "--spec": "4", "--spec-min-p": "0.5", "--max-context": "32768", "--kv": "int8"}
    allowed = set(options) | {"--vram-reserve-mib", "--adapt-async", "--rope-scaling", "--rope-scale"}
    config = {"exe": str(engine), "cwd": str(root), "host": "127.0.0.1", "port": 8080,
              "sampling": {"temperature": 1.0, "top_p": 0.95, "top_k": 20},
              "env": {"STRATA_PIN_BY_NEED": "1"}}
    metadata = read_json(engine.parent / "BUILD.json")
    if template_config:
        config["lib_dirs"] = template_config["lib_dirs"]
    elif metadata.get("backend") == "hip":
        config.update(backend="hip", lib_dirs=[str(path) for path in setup.hip_lib_dirs(engine.parent)])
    else:
        config["lib_dirs"] = setup.engine_lib_dirs(engine.parent, toolkit=int(str(metadata["cuda"]).split(".")[0]))
    if template_config:
        for key in ("port", "api_key", "sampling", "lib_dirs", "backend", "cuda", "parallel", "gpu", "layer_split"):
            if key in template_config:
                config[key] = template_config[key]
        config["env"] = {key: value for key, value in template_config.get("env", {}).items() if key == "STRATA_PIN_BY_NEED"}
        old_args = template_config["args"]
        for index, argument in enumerate(old_args[:-1]):
            if argument in allowed:
                options[argument] = old_args[index + 1]
    expert_profile = root / "data" / ("expert-profile-coder.bin" if coder else "expert-profile.bin")
    require_files(expert_profile.parent, (expert_profile.name,))
    config.update(model_name=entry["name"], tokenizer=str(pack / "tokenizer"), log=str(report / ("model-" + model_id + ".log")),
                  args=["--pack", str(pack), "--native", str(paths[0]), "--ple-gguf", str(ple),
                        "--expert-profile", str(expert_profile), "--mtp", str(rt),
                        *[value for key, item in options.items() for value in (key, item)]])
    # Publish the profile last. A failed preparation remains visible but is never offered as runnable.
    write_json(marker, {**identity, "complete": True})
    staged = profile.with_suffix(".json.hub-preparing")
    write_json(staged, config)
    # Windows rename refuses an existing target, while publishing a complete JSON file atomically.
    staged.rename(profile)
    entry["state"] = "ready"
    write_json(report / "model-library.json", state)
    inventory(report, models, profiles, root)
    emit("complete", "Model ready. Select it in Models and click Start.", profiles_generation=time.time_ns())


def referenced(folder, profiles, report, root=None):
    found = []
    def contains(value):
        if isinstance(value, str):
            path = Path(value)
            return path.is_absolute() and path.resolve().is_relative_to(folder.resolve())
        if isinstance(value, dict):
            return any(contains(v) for v in value.values())
        return isinstance(value, list) and any(contains(v) for v in value)
    candidates = set(profiles.glob("strata-*.json")) | set((report / "profiles-run").glob("*.json"))
    if root is not None:
        candidates |= set(root.glob("strata-*.json"))
    for path in candidates:
        value = read_json(path)
        if 'args' in value and contains(value):
            found.append(str(path))
    return found


def inventory(report, models, profiles, root=None):
    result = library(report)
    for entry in result["models"]:
        folder = Path(entry["path"])
        entry["references"] = referenced(folder, profiles, report)
    result["existing"] = existing_models(models, result["models"], profiles, root)
    for entry in result["existing"]:
        entry["references"] = referenced(Path(entry["path"]), profiles, report)
    write_json(report / "model-inventory.json", result)
    refresh(report)
    emit("complete", "Local model inventory refreshed.")


def existing_models(models, entries, profiles=None, root=None):
    result = []
    managed = [Path(e["path"]).resolve() for e in entries]
    base = models / "models"
    folders = set()
    def failed(error):
        raise error
    if base.is_dir():
        for current, directories, names in os.walk(base, topdown=True, onerror=failed, followlinks=False):
            if any(Path(current).resolve().is_relative_to(path) for path in managed):
                directories[:] = []
                continue
            directories[:] = [name for name in directories if not name.startswith('.') and
                              (Path(current) / name).resolve() == (Path(current) / name).absolute()]
            if any(is_model_file(name) for name in names):
                folders.add(Path(current).absolute())
    candidates = set(profiles.glob('strata-*.json')) if profiles is not None else set()
    if root is not None:
        candidates |= set(root.glob('strata-*.json'))
    for file in candidates:
        arguments = read_json(file).get('args', [])
        if '--native' in arguments:
            source = Path(arguments[arguments.index('--native') + 1])
            if source.is_file():
                folders.add(source.parent.absolute())
    for folder in sorted(folders):
        if folder.resolve() != folder.absolute() or any(folder.resolve().is_relative_to(path) for path in managed):
            continue
        files = [file for file in folder.iterdir() if file.is_file() and is_model_file(file.name)]
        if files:
            result.append({"id": "existing-" + hashlib.sha256(str(folder.resolve()).casefold().encode()).hexdigest()[:20],
                           "name": folder.name if folder.parent == base else f"{folder.parent.name} / {folder.name}",
                           "path": str(folder), "size": sum(p.stat().st_size for p in files),
                           "state": "existing", "files": [{"name": p.name, "size": p.stat().st_size} for p in files]})
    return result


def profile_changes(folder, target, profiles, report, root):
    candidates = set(profiles.glob("strata-*.json")) | set(root.glob("strata-*.json")) | set((report / "profiles-run").glob("*.json"))
    drafts = report / "hub-profiles.json"
    if drafts.exists():
        candidates.add(drafts)
    for file in list(candidates):
        value = read_json(file)
        arguments = value.get("args", [])
        if "--pack" in arguments:
            pack = Path(arguments[arguments.index("--pack") + 1])
            for name in ("manifest.json", "experts.bin.src.json"):
                if (pack / name).is_file():
                    candidates.add(pack / name)
    changes = []
    def update(value):
        if isinstance(value, str):
            path = Path(value)
            if path.is_absolute():
                return str(target / path.resolve().relative_to(folder.resolve())) if path.resolve().is_relative_to(folder.resolve()) else value
            normalized = value.replace("/", "\\").casefold()
            if str(folder).replace("/", "\\").casefold() in normalized:
                raise ValueError("A command string contains the model folder. Update this reference manually before moving the model.")
            return value
        if isinstance(value, dict):
            output = {}
            for key, item in value.items():
                if key == 'extra_args' and isinstance(item, str):
                    arguments = json.loads(item)
                    if not isinstance(arguments, list) or not all(isinstance(argument, str) for argument in arguments):
                        raise ValueError('Saved extra_args must be a JSON string array.')
                    changed = update(arguments)
                    output[key] = json.dumps(changed, indent=2) if changed != arguments else item
                else:
                    output[key] = update(item)
            return output
        if isinstance(value, list):
            return [update(item) for item in value]
        return value
    for file in sorted(candidates):
        before = read_json(file)
        after = update(before)
        if before != after:
            changes.append({"path": str(file), "sha256": hashlib.sha256(file.read_bytes()).hexdigest(), "after": after})
    for file in set(profiles.glob("*.bat")) | set(root.glob("*.bat")):
        if str(folder).replace("/", "\\").casefold() in file.read_text(encoding="utf-8-sig").replace("/", "\\").casefold():
            raise ValueError(f"Launcher directly references this model folder; update it manually first: {file}")
    return changes


def require_idle(root, report, profiles, helper):
    # Reuse the same process ownership and live-port checks as Strata updates.
    namespace = {"__name__": "hub_model_idle_check"}
    if helper is None:
        raise ValueError("The hub idle-check helper is required for model changes.")
    exec(compile(helper.read_text(encoding="utf-8"), str(helper), "exec"), namespace)
    namespace["require_idle"](root, report, profiles)


def folder_contents(folder):
    files, directories = [], []
    def failed(error):
        raise error
    for current, children, names in os.walk(folder, followlinks=False, onerror=failed):
        for name in children:
            path = safe_path(folder, (Path(current) / name).relative_to(folder).as_posix())
            directories.append(str(path))
        for name in names:
            path = safe_path(folder, (Path(current) / name).relative_to(folder).as_posix())
            files.append({"path": str(path), "size": path.stat().st_size, "mtime": path.stat().st_mtime_ns})
        if len(files) > 512 or len(directories) > 2048:
            raise ValueError("This folder is too large for a complete file review. Choose a dedicated model folder.")
    return sorted(files, key=lambda file: file["path"]), sorted(directories)


def profile_deletions(folder, profiles, report, root):
    references = set(map(Path, referenced(folder, profiles, report, root)))
    candidates = set(profiles.glob("strata-*.json")) | set(root.glob("strata-*.json"))
    for file in set(profiles.glob("*.bat")) | set(root.glob("*.bat")):
        content = file.read_text(encoding="utf-8-sig").casefold()
        used = {profile for profile in candidates if profile.name.casefold() in content}
        direct = str(folder).replace('/', '\\').casefold() in content.replace('/', '\\')
        if direct and not used:
            raise ValueError(f"Launcher contains direct model paths; adjust it manually before deleting: {file}")
        if used & references or direct:
            if used - references:
                raise ValueError(f"A launcher also uses other models. Adjust it manually before deleting: {file}")
            references.add(file)
    ownership = report / 'owned-strata.json'
    remaining = [file for file in candidates - references if 'args' in read_json(file)]
    if ownership.is_file():
        config = read_json(ownership).get('config')
        if not remaining or (isinstance(config, str) and Path(config) in references):
            references.add(ownership)
    changes = []
    drafts = report / "hub-profiles.json"
    if drafts.exists():
        value = read_json(drafts)
        names = {file.name for file in references if file.suffix == '.json'}
        value["profiles"] = {name: settings for name, settings in value.get("profiles", {}).items() if name not in names}
        if value.get("selected") in names:
            value.pop("selected")
        if value != read_json(drafts):
            changes.append({"path": str(drafts), "sha256": hash_file(drafts), "after": value})
    return [{"path": str(file), "sha256": hash_file(file)} for file in sorted(references)], changes


def operation_plan(root, report, profiles, model_id, action, destination=None, models=None):
    state = library(report)
    entries = state["models"] + (existing_models(models, state["models"], profiles, root) if models is not None else [])
    entry = next((e for e in entries if e["id"] == model_id), None)
    if entry is None:
        raise ValueError("The selected model is no longer in the local inventory. Scan local files again.")
    folder = model_folder(Path(entry["path"]), root, report, profiles)
    for other in state['models']:
        other_folder = Path(other['path']).resolve()
        if other['id'] != model_id and (other_folder.is_relative_to(folder) or folder.is_relative_to(other_folder)):
            raise ValueError('This folder overlaps another registered model. Manage each dedicated model folder separately.')
    if not folder.is_dir():
        raise ValueError("The registered model folder is missing.")
    files, directories = folder_contents(folder)
    target = None
    if action == "move":
        if destination is None or not destination.is_dir() or destination.resolve() != destination.absolute():
            raise ValueError("Choose an existing destination directory without junctions or symlinks.")
        target = destination / folder.name
        if target.exists() or target.resolve().is_relative_to(folder.resolve()):
            raise ValueError("Destination already exists or is inside the model folder.")
    changes = profile_changes(folder, target, profiles, report, root) if action == "move" else []
    deletions = []
    if action == "delete":
        deletions, changes = profile_deletions(folder, profiles, report, root)
    plan = {"id": model_id, "action": action, "source": str(folder), "target": str(target) if target else None,
            "files": files, "directories": directories, "changes": changes, "deletions": deletions, "entry": entry}
    plan["digest"] = hashlib.sha256(json.dumps(plan, sort_keys=True).encode()).hexdigest()
    return plan


def same_volume(source, target):
    return source.stat().st_dev == target.parent.stat().st_dev


def mutate(root, report, profiles, model_id, action, destination, digest, helper, models=None):
    require_idle(root, report, profiles, helper)
    plan = operation_plan(root, report, profiles, model_id, action, destination, models)
    if plan["digest"] != digest:
        raise ValueError("Model files changed since confirmation. Review the operation again.")
    source = Path(plan["source"])
    state = library(report)
    entry = next((e for e in state["models"] if e["id"] == model_id), None)
    if entry is None:
        entry = plan["entry"]
        entry["state"] = "local"
        state["models"].append(entry)
    staged = []
    if plan["changes"] or plan["deletions"]:
        backup = report / "model-backups" / str(time.time_ns())
        backup.mkdir(parents=True)
        write_json(backup / "operation.json", plan)
        for index, deleted in enumerate(plan["deletions"]):
            file = Path(deleted["path"])
            shutil.copy2(file, backup / f"deleted-{index:03d}-{file.name}")
        for index, change in enumerate(plan["changes"]):
            path = Path(change["path"])
            shutil.copy2(path, backup / f"{index:03d}-{path.name}")
            temporary = path.with_suffix(path.suffix + ".hub-model-move")
            if temporary.exists():
                raise ValueError("A staged profile update already exists; inspect the previous operation before moving.")
            with temporary.open("x", encoding="utf-8") as output:
                json.dump(change["after"], output, indent=2)
            staged.append((temporary, path))
    def commit_profiles(target, renamed):
        for temporary, path in staged:
            if path.is_relative_to(source):
                new_path = target / path.relative_to(source)
                new_temporary = target / temporary.relative_to(source)
                if not renamed:
                    shutil.copyfile(temporary, new_temporary)
                os.replace(new_temporary, new_path)
                if not renamed:
                    temporary.unlink()
            else:
                os.replace(temporary, path)
    if action == "move":
        target = Path(plan["target"])
        if same_volume(source, target):
            source.rename(target)
            commit_profiles(target, True)
        else:
            if shutil.disk_usage(target.parent).free < sum(f["size"] for f in plan["files"]):
                raise ValueError("Insufficient free space at the move destination.")
            target.mkdir()
            for directory in plan["directories"]:
                (target / Path(directory).relative_to(source)).mkdir(parents=True, exist_ok=True)
            total_bytes = sum(f["size"] for f in plan["files"])
            copied_bytes, last_progress = 0, 0.0
            for file in plan["files"]:
                old = Path(file["path"])
                new = target / old.relative_to(source)
                new.parent.mkdir(parents=True, exist_ok=True)
                digest = hashlib.sha256()
                with old.open("rb") as input, new.open("xb") as output:
                    for block in iter(lambda: input.read(CHUNK), b""):
                        digest.update(block)
                        output.write(block)
                        copied_bytes += len(block)
                        if time.monotonic() - last_progress >= 1:
                            emit("progress", f"Moving {100 * copied_bytes / max(total_bytes, 1):.1f}%: {old.name}")
                            last_progress = time.monotonic()
                    output.flush()
                    os.fsync(output.fileno())
                check = hashlib.sha256()
                emit("progress", f"Verifying copied file: {old.name}")
                with new.open("rb") as copied:
                    for block in iter(lambda: copied.read(CHUNK), b""):
                        check.update(block)
                if digest.digest() != check.digest():
                    raise ValueError("Move verification failed. Source files were retained.")
                emit("progress", f"Copied and verified {old.name}")
            commit_profiles(target, False)
            for file in plan["files"]:
                Path(file["path"]).unlink()
            for directory in sorted(map(Path, plan["directories"]), key=lambda path: len(path.parts), reverse=True):
                safe_path(source, directory.relative_to(source).as_posix()).rmdir()
            source.rmdir()
        entry["path"] = str(target)
    elif action == "delete":
        commit_profiles(None, False)
        for deleted in plan["deletions"]:
            Path(deleted["path"]).unlink()
        for file in plan["files"]:
            Path(file["path"]).unlink()
        for directory in sorted(map(Path, plan["directories"]), key=lambda path: len(path.parts), reverse=True):
            safe_path(source, directory.relative_to(source).as_posix()).rmdir()
        source.rmdir()
        state["models"].remove(entry)
    else:
        raise ValueError("Invalid model operation.")
    write_json(report / "model-library.json", state)
    if models is not None:
        inventory(report, models, profiles, root)
    else:
        refresh(report)
    emit("complete", "Model moved." if action == "move" else "Model deleted.", profiles_generation=time.time_ns())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("scan", "download", "prepare", "inventory", "plan-move", "plan-delete", "move", "delete"))
    for key in ("root", "report", "models", "profiles"):
        parser.add_argument("--" + key, type=Path, required=True)
    parser.add_argument("--repo")
    parser.add_argument("--id")
    parser.add_argument("--destination", type=Path)
    parser.add_argument("--confirmation")
    parser.add_argument("--idle-helper", type=Path)
    parser.add_argument("--template", type=Path)
    args = parser.parse_args()
    try:
        if args.mode == "scan":
            scan_repo(args.root, args.report, args.repo)
        elif args.mode == "download":
            require_idle(args.root, args.report, args.profiles, args.idle_helper)
            identity = download(args.root, args.report, args.models, args.id, args.destination)
            prepare(args.root, args.report, args.models, args.profiles, identity, args.idle_helper, args.template)
        elif args.mode == "prepare":
            prepare(args.root, args.report, args.models, args.profiles, args.id, args.idle_helper, args.template)
        elif args.mode == "inventory":
            inventory(args.report, args.models, args.profiles, args.root)
        elif args.mode.startswith("plan-"):
            require_idle(args.root, args.report, args.profiles, args.idle_helper)
            plan = operation_plan(args.root, args.report, args.profiles, args.id, args.mode[5:], args.destination, args.models)
            write_json(args.report / "model-operation.json", plan)
            emit("plan", "Review the model operation before confirming.", plan_generation=time.time_ns())
        else:
            mutate(args.root, args.report, args.profiles, args.id, args.mode, args.destination, args.confirmation, args.idle_helper, args.models)
    except Exception as error:
        emit("error", str(error), **({"profiles_generation": time.time_ns()} if args.mode in ("move", "delete", "download", "prepare") else {}))
        raise


if __name__ == "__main__":
    main()
