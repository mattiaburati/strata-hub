"""List devices for the selected engine without starting a model."""
import csv
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys


def detect(config):
    backend = config.get("backend") or "cuda"
    env = dict(os.environ)
    for key in ("CUDA_VISIBLE_DEVICES", "HIP_VISIBLE_DEVICES", "ROCR_VISIBLE_DEVICES", "GPU_DEVICE_ORDINAL"):
        env.pop(key, None)
    if backend == "cuda":
        done = subprocess.run(
            ["nvidia-smi", "--query-gpu=index,name,memory.total", "--format=csv,noheader,nounits"],
            capture_output=True, text=True, timeout=20, check=True, env=env,
        )
        cards = [{"index": int(index), "name": name.strip(), "vram_gb": float(memory) / 1024}
                 for index, name, memory in csv.reader(io.StringIO(done.stdout), skipinitialspace=True)]
    elif backend == "hip":
        probe = Path(config["exe"]).with_name("strata-device.exe")
        if not probe.is_file():
            raise ValueError(f"GPU detection requires the engine's device probe: {probe}")
        env["PATH"] = os.pathsep.join([*(config.get("lib_dirs") or []), env.get("PATH", "")])
        done = subprocess.run([str(probe), "--list-devices"], cwd=probe.parent,
                              capture_output=True, text=True, timeout=20, check=True, env=env)
        cards = []
        for line in done.stdout.splitlines():
            match = re.match(r"\s*device\s+(\d+):\s*(.+)", line)
            memory = re.match(r"\s*arch\s+gfx[0-9a-f]+\s*,\s*([\d.]+)\s*GiB", line)
            if match:
                cards.append({"index": int(match[1]), "name": match[2].strip(), "vram_gb": None})
            elif memory and cards:
                cards[-1]["vram_gb"] = float(memory[1])
            elif line.strip().startswith("cannot run:"):
                if not cards:
                    raise ValueError(line.strip())
                cards[-1]["problem"] = line.strip()
        if any(card["vram_gb"] is None for card in cards):
            raise ValueError("The device probe did not report GPU memory")
    else:
        raise ValueError(f"GPU detection is not available for backend {backend!r}")
    if not cards:
        raise ValueError("No GPUs were reported for this engine backend")
    return cards


def main():
    try:
        cards = detect(json.loads(sys.argv[1]))
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        detail = error.stderr.strip() if isinstance(error, subprocess.CalledProcessError) and error.stderr else str(error)
        print("HUB_EVENT " + json.dumps({"kind": "error", "message": f"GPU detection failed: {detail}"}))
        raise SystemExit(1) from error
    print("HUB_EVENT " + json.dumps({"kind": "complete", "cards": cards,
          "message": f"Detected {len(cards)} GPU(s). Selection applies on the next model launch."}))


if __name__ == "__main__":
    main()
