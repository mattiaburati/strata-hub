"""Run installed Strata's calibration on an effective hub profile, without starting an API server."""
import argparse
import datetime
import importlib
import json
import math
from pathlib import Path
import sys
import time


def emit(kind, message, **fields):
    print("HUB_EVENT " + json.dumps({"kind": kind, "message": message, **fields}), flush=True)


def require_idle(root, report, profiles, helper):
    # Share the ownership, live-port and Windows executable checks used by updates/model operations.
    namespace = {"__name__": "hub_calibration_idle_check"}
    exec(compile(helper.read_text(encoding="utf-8"), str(helper), "exec"), namespace)
    namespace["require_idle"](root, report, profiles)


def run(root, report, profiles, helper, request_path):
    request = json.loads(request_path.read_text(encoding="utf-8"))
    config = request["config"]
    if Path(config["cwd"]).resolve() != root.resolve():
        raise ValueError("The profile does not belong to the configured Strata installation.")
    started = time.monotonic()
    emit("progress", "Preparing the selected profile", stage="Preparing", profile=request["profile"])
    require_idle(root, report, profiles, helper)
    sys.path.insert(0, str(root))
    sys.path.insert(0, str(root / "tools"))
    calibration = importlib.import_module("calibrate")
    server = importlib.import_module("serve.server")
    if not all(callable(getattr(calibration, name, None)) for name in ("run", "apply")):
        raise RuntimeError("This Strata installation has no supported calibration API. Update Strata first.")
    defaults = calibration.DEFAULTS
    expected = {"--pcie-frac", "--spec-min-p", "--pool-workers", "--adapt-every", "--adapt-swaps", "--adapt-decay"}
    optional = {"--prefill-pipe"}
    if not expected <= set(defaults) or set(defaults) - expected - optional:
        raise RuntimeError("Strata's calibration options changed. Update the hub before calibrating.")
    config["log"] = str(request_path.parent / "engine.log")
    engines = []
    vision = None
    stage = "Loading engine"

    def progress(message=""):
        nonlocal stage
        message = str(message).strip()
        if not message:
            return
        if "PCIe share" in message:
            stage = "PCIe sharing"
        elif "draft floor" in message:
            stage = "Speculative decoding"
        elif "CPU workers" in message:
            stage = "CPU workers"
        elif "expert tier" in message:
            stage = "Expert cache"
        emit("progress", message[:1600], stage=stage)

    def start_engine(arguments):
        emit("progress", f"Loading engine for measurements (start {len(engines) + 1})", stage=stage)
        engine = server.StrataEngine(config["exe"], arguments, cwd=config["cwd"],
                                     log=config["log"], env=server.child_env(config))
        engines.append(engine.proc)
        return engine

    with open(config["log"], "a", encoding="utf-8") as engine_log:
        try:
            if config.get("vision"):
                emit("progress", "Loading the image encoder before sizing the expert cache", stage="Vision encoder")
                vision = server.Vision(config["vision"], log=engine_log,
                                       env=server.vision_env(config, server.child_env(config)))
            result = calibration.run(config, say=progress, start_engine=start_engine)
        finally:
            # Upstream closes each measured engine; wait for any surviving child before releasing the hub lock.
            for process in engines:
                if process.poll() is None:
                    process.kill()
                process.wait()
            if vision is not None:
                vision.shutdown()
    if result["report"].get("failed_starts"):
        raise RuntimeError("Some calibration candidates failed to start. Results were not applied; see the calibration log.")
    speed = result["report"].get("tok_s")
    if isinstance(speed, bool) or not isinstance(speed, (int, float)) or not math.isfinite(speed) or speed <= 0:
        raise RuntimeError("Calibration returned no valid measured speed. Results were not applied.")
    if set(result["settings"]) - set(defaults):
        raise ValueError("Calibration returned unknown engine settings.")
    require_idle(root, report, profiles, helper)
    applied = {flag: result["settings"].get(flag, default) for flag, default in defaults.items()}
    output = {"profile": request["profile"], "original_settings": request["original_settings"], "saved_state": request["saved_state"],
              "applied": applied, "report": result["report"],
              "date": datetime.datetime.now().astimezone().isoformat(timespec="seconds"),
              "context": request["original_settings"]["values"][0],
              "vision": request["original_settings"]["vision"],
              "parallel": request["original_settings"]["parallel"]}
    path = request_path.parent / "result.json"
    temporary = path.with_suffix(".json.tmp")
    temporary.write_text(json.dumps(output, indent=2, allow_nan=False), encoding="utf-8")
    temporary.replace(path)
    emit("complete", f"Measurements complete: {speed:.1f} token/s. Saving profile settings.",
         stage="Measurements complete", seconds=round(time.monotonic() - started), tok_s=speed)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("root", "report", "profiles", "idle-helper", "request"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    try:
        run(args.root, args.report, args.profiles, args.idle_helper, args.request)
    except Exception as error:
        emit("error", str(error), stage="Failed")
        raise


if __name__ == "__main__":
    main()
