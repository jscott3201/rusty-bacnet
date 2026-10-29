#!/usr/bin/env python3
"""Run the approved #1979 constituent experiment from frozen artifacts. No builds/retries."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("inputs", type=Path, help="recorded roots, artifacts, and source hashes")
parser.add_argument("output", type=Path, help="a new directory; existing evidence is never overwritten")
args = parser.parse_args()
inputs = json.loads(args.inputs.read_text())
args.output.mkdir(parents=True, exist_ok=False)
shutil.copy2(args.inputs, args.output / "inputs.json")


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def verify():
    for path, expected in inputs["source_hashes"].items():
        if sha(path) != expected:
            raise RuntimeError(f"source changed: {path}")
    for artifact in inputs["artifacts"].values():
        if sha(artifact["path"]) != artifact["sha256"]:
            raise RuntimeError(f"artifact changed: {artifact['path']}")


manifest = {"started": now(), "inputs_sha256": sha(args.inputs), "runs": [],
            "order": {"encoding": list("ACCAAC")}}


def save():
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def execute(label, name, artifact, arguments, kind):
    root = Path(inputs["roots"][label])
    if shutil.disk_usage(root).free < 50 * 1024**3:
        raise RuntimeError("less than 50 GiB free")
    executable = inputs["artifacts"][artifact]
    if sha(executable["path"]) != executable["sha256"]:
        raise RuntimeError(f"artifact changed before {name}")
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(root / "target")
    command = [executable["path"], *arguments]
    record = {"name": name, "label": label, "kind": kind, "cwd": str(root),
              "command": command, "started": now(), "load_before": os.getloadavg(),
              "free_bytes_before": shutil.disk_usage(root).free,
              "runtime_env": {key: env.get(key) for key in
                              ["TOKIO_WORKER_THREADS", "RUST_LOG", "CARGO_TARGET_DIR"]}}
    manifest["runs"].append(record)
    save()
    print("START", name, flush=True)
    with (args.output / (name + ".out")).open("w") as stdout, (args.output / (name + ".err")).open("w") as stderr:
        result = subprocess.run(command, cwd=root, env=env, stdout=stdout, stderr=stderr)
    record.update(ended=now(), exit_code=result.returncode, load_after=os.getloadavg())
    for suffix in ["out", "err"]:
        record[suffix + "_sha256"] = sha(args.output / (name + "." + suffix))
    save()
    print("END", name, result.returncode, flush=True)
    if result.returncode:
        raise RuntimeError(f"{name} failed; evidence retained, matrix stopped")
    if kind != "encoding":
        data = json.loads((args.output / (name + ".out")).read_text())
        if not data["clients_diagnostics"]["valid"]:
            raise RuntimeError(f"{name} is an incomplete measurement")


try:
    verify()
    counts = {"A": 0, "C": 0}
    workloads = ["encode_read_property_request", "encode_apdu_confirmed_request", "encode_npdu_100b_payload"]
    for label in manifest["order"]["encoding"]:
        counts[label] += 1
        name = f"constituent-{label}-{counts[label]:02}"
        baseline = "1979-" + name
        root = Path(inputs["roots"][label])
        for workload in workloads:
            if (root / "target/criterion" / workload / baseline).exists():
                raise RuntimeError(f"refusing to overwrite Criterion baseline {baseline}")
        execute(label, name, label + "-encoding", ["--bench", "^(encode_read_property_request|encode_apdu_confirmed_request|encode_npdu_100b_payload)$",
                "--sample-size", "100", "--warm-up-time", "3", "--measurement-time", "5", "--noplot",
                "--save-baseline", baseline], "encoding")
        for workload in workloads:
            source = root / "target/criterion" / workload / baseline
            destination = args.output / "criterion" / name / workload
            shutil.copytree(source, destination)
            for required in ["estimates.json", "sample.json"]:
                if not (destination / required).is_file():
                    raise RuntimeError(f"missing Criterion {required} for {name}")
    verify()
    manifest["completed"] = now()
except BaseException as error:
    manifest["failure"] = repr(error)
    manifest["stopped"] = now()
    save()
    raise
save()
