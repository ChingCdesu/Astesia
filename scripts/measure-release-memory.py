#!/usr/bin/env python3
"""Measure isolated macOS release-test workloads using kernel physical footprint."""

import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import selectors
import statistics
import subprocess
import time


class RUsageInfoV4(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16), ("values", ctypes.c_uint64 * 35)]


def usage(libproc, pid):
    result = RUsageInfoV4()
    if libproc.proc_pid_rusage(pid, 4, ctypes.byref(result)) != 0:
        return None
    if result.values[9] != 0 or result.values[7] == 0:
        return None
    return {
        "physical_bytes": result.values[7],
        "peak_physical_bytes": result.values[28],
        "resident_bytes": result.values[6],
    }


def measure(binary, fixture, scenario, output, libproc):
    test = (
        "ui::query_item::tests::release_memory_query_chart_workload"
        if scenario == "chart"
        else "application::memory_workloads::release_memory_workload"
    )
    env = dict(os.environ, ASTESIA_MEMORY_FIXTURE_PATH=str(fixture),
               ASTESIA_MEMORY_SCENARIO=scenario,
               ASTESIA_MEMORY_OUTPUT_DIR=str(output.parent / (output.name.split("-")[0] + "-files")))
    started = time.monotonic()
    child = subprocess.Popen([str(binary), "--exact", test, "--ignored", "--nocapture"],
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=env)
    selector = selectors.DefaultSelector()
    selector.register(child.stdout, selectors.EVENT_READ)
    samples = []
    stages = {}
    stage = "starting"
    pending = b""
    with output.open("wb") as log:
        while True:
            reading = usage(libproc, child.pid)
            if reading:
                reading["seconds"] = round(time.monotonic() - started, 3)
                reading["stage"] = stage
                samples.append(reading)
                stages[stage] = reading
            events = selector.select(0.05)
            for key, _ in events:
                chunk = os.read(key.fd, 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                log.write(chunk)
                log.flush()
                pending += chunk
                while b"\n" in pending:
                    line, pending = pending.split(b"\n", 1)
                    if line.startswith(b"MEMORY_STAGE "):
                        stage = line.decode().split()[1]
            if child.poll() is not None and not selector.get_map():
                break
    selector.close()
    if child.returncode:
        raise RuntimeError(f"{scenario} failed ({child.returncode}); see {output}")
    return {
        "scenario": scenario, "pid": child.pid, "elapsed_seconds": time.monotonic() - started,
        "peak_physical_bytes": max(s["peak_physical_bytes"] for s in samples),
        "stage_medians": {
            name: statistics.median(s["physical_bytes"] for s in samples if s["stage"] == name)
            for name in stages
        },
        "stages": stages, "samples": samples,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--label", required=True)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--scenarios", nargs="+", default=["query", "csv", "json", "xlsx", "backup", "restore", "table_chart", "chart"])
    args = parser.parse_args()
    libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    libproc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
    libproc.proc_pid_rusage.restype = ctypes.c_int
    args.output_dir.mkdir(parents=True, exist_ok=True)
    result = {"label": args.label, "binary": str(args.test_binary.resolve()),
              "binary_sha256": hashlib.file_digest(args.test_binary.open("rb"), "sha256").hexdigest(),
              "fixture_sha256": hashlib.file_digest(args.fixture.open("rb"), "sha256").hexdigest(),
              "runs": []}
    for scenario in args.scenarios:
        for repetition in range(args.repetitions):
            log = args.output_dir / f"{args.label}-{scenario}-{repetition + 1}.log"
            run = measure(args.test_binary.resolve(), args.fixture.resolve(), scenario, log, libproc)
            run["repetition"] = repetition + 1
            result["runs"].append(run)
            print(f"{scenario} #{repetition + 1}: peak {run['peak_physical_bytes'] / 1048576:.2f} MiB", flush=True)
            (args.output_dir / f"{args.label}-workloads.json").write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
