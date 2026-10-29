#!/usr/bin/env python3
"""Summarize the checked-in #1979 run-level CSVs without pooling requests."""
import csv
import hashlib
import json
from pathlib import Path
import statistics

root = Path(__file__).resolve().parent
provenance = json.loads((root / "provenance.json").read_text())
for name, expected in provenance["derived_csv_sha256"].items():
    assert hashlib.sha256((root / name).read_bytes()).hexdigest() == expected, name

def distribution(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values)}

with (root / "encoding.csv").open() as source:
    encoding = list(csv.DictReader(source))
with (root / "clients.csv").open() as source:
    clients = list(csv.DictReader(source))
assert len(encoding) == 42 and len(clients) == 42
result = {"encoding": {}, "clients": {}, "paired_clients": {}}
for workload in sorted({row["workload"] for row in encoding}):
    values = {
        label: [float(row["median_ns"]) for row in encoding
                if row["workload"] == workload and row["revision"] == label]
        for label in "AC"
    }
    expected = 6 if workload in {"decode_npdu_100b_payload", "full_stack_encode_rp"} else 3
    assert all(len(items) == expected for items in values.values())
    result["encoding"][workload] = {label: distribution(items) for label, items in values.items()}
    result["encoding"][workload]["paired_C_over_A"] = distribution(
        [c / a for a, c in zip(values["A"], values["C"], strict=True)])
for row in clients:
    assert int(row["started_clients"]) == int(row["completed_clients"]) == int(row["clients"])
    assert int(row["errors"]) == int(row["abort_out_of_resources"]) == int(row["abort_admitted"])
    assert int(row["global_overloaded"]) == int(row["errors"]) + int(row["fallback_dropped"])
    assert int(row["terminal_timeout"]) == int(row["other_errors"]) == 0
    expected = 100 * int(row["errors"]) / (int(row["successful"]) + int(row["errors"]))
    assert abs(float(row["error_rate_pct"]) - expected) < 1e-12
metrics = ("error_rate_pct", "throughput", "elapsed_secs", "p50_us", "p99_us", "errors")
repeat_values = {}
for label in "ABC":
    rows = [row for row in clients if row["kind"] == "repeat" and row["revision"] == label]
    assert len(rows) == 10 and all(int(row["clients"]) == 100 for row in rows)
    repeat_values[label] = {metric: [float(row[metric]) for row in rows] for metric in metrics}
    result["clients"][label] = {metric: distribution(values) for metric, values in repeat_values[label].items()}
for newer, older in [("B", "A"), ("C", "A"), ("C", "B")]:
    a, b = repeat_values[older], repeat_values[newer]
    result["paired_clients"][newer + "_minus_" + older + "_percentage_points"] = distribution(
        [bv - av for av, bv in zip(a["error_rate_pct"], b["error_rate_pct"], strict=True)])
    result["paired_clients"][newer + "_over_" + older + "_throughput"] = distribution(
        [bv / av for av, bv in zip(a["throughput"], b["throughput"], strict=True)])
print(json.dumps(result, indent=2))
