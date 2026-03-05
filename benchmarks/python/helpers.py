"""Shared helpers for BACnet Python benchmarks."""

import os
import ssl
import subprocess
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path

# BACnet engineering units constants
UNITS_DEGREES_F = 64
UNITS_PERCENT = 23

# Base ports for different transports and roles
# BIP: 47808-47829, SC: 47900-47929
PORT_BIP_SERVER_RUST = 47808
PORT_BIP_SERVER_PY = 47809
PORT_BIP_CLIENT_BASE = 47811  # clients use 47811-47830

PORT_SC_HUB = 47900
PORT_SC_SERVER_RUST = 47901
PORT_SC_SERVER_PY = 47902
PORT_SC_CLIENT_BASE = 47910

# Object setup: same for all servers
OBJECTS = {
    "analog_inputs": [(0, "AI-0", UNITS_DEGREES_F, 72.5),
                      (1, "AI-1", UNITS_DEGREES_F, 68.0),
                      (2, "AI-2", UNITS_PERCENT, 55.0)],
    "analog_outputs": [(0, "AO-0", UNITS_DEGREES_F)],
    "binary_values": [(0, "BV-0")],
}


def populate_server(server):
    """Add standard objects to a BACnetServer instance."""
    for inst, name, units, pv in OBJECTS["analog_inputs"]:
        server.add_analog_input(inst, name, units, pv)
    for inst, name, units in OBJECTS["analog_outputs"]:
        server.add_analog_output(inst, name, units)
    for inst, name in OBJECTS["binary_values"]:
        server.add_binary_value(inst, name)


@dataclass
class TlsCerts:
    """Paths to generated TLS certificates."""
    ca_cert: str
    server_cert: str
    server_key: str
    client_cert: str
    client_key: str
    _tmpdir: tempfile.TemporaryDirectory = field(repr=False, default=None)

    def cleanup(self):
        if self._tmpdir:
            self._tmpdir.cleanup()


def generate_tls_certs() -> TlsCerts:
    """Generate CA + server + client certs using openssl CLI."""
    tmpdir = tempfile.TemporaryDirectory(prefix="bacnet_bench_certs_")
    d = tmpdir.name

    def run(cmd):
        subprocess.run(cmd, shell=True, check=True, capture_output=True, cwd=d,
                       executable="/bin/bash")

    # CA
    run("openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 "
        "-keyout ca.key -out ca.pem -days 1 -nodes "
        "-subj '/CN=BACnet Bench CA'")

    # Server cert with SAN
    san_conf = os.path.join(d, "san.cnf")
    with open(san_conf, "w") as f:
        f.write("[req]\n"
                "distinguished_name = req_dn\n"
                "req_extensions = v3_req\n"
                "[req_dn]\n"
                "[v3_req]\n"
                "subjectAltName = DNS:localhost,IP:127.0.0.1\n"
                "[v3_ca]\n"
                "subjectAltName = DNS:localhost,IP:127.0.0.1\n")

    run("openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 "
        "-keyout server.key -out server.csr -nodes "
        "-subj '/CN=localhost' -config san.cnf")
    run("openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key "
        "-CAcreateserial -out server.pem -days 1 "
        "-extensions v3_ca -extfile san.cnf")

    # Client cert (for mTLS)
    run("openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 "
        "-keyout client.key -out client.csr -nodes "
        "-subj '/CN=BACnet Bench Client'")
    run("openssl x509 -req -in client.csr -CA ca.pem -CAkey ca.key "
        "-CAcreateserial -out client.pem -days 1")

    return TlsCerts(
        ca_cert=os.path.join(d, "ca.pem"),
        server_cert=os.path.join(d, "server.pem"),
        server_key=os.path.join(d, "server.key"),
        client_cert=os.path.join(d, "client.pem"),
        client_key=os.path.join(d, "client.key"),
        _tmpdir=tmpdir,
    )


class LatencyTracker:
    """Lightweight latency tracker using perf_counter_ns."""

    def __init__(self):
        self.samples = []

    def record(self, ns: int):
        self.samples.append(ns)

    def stats(self) -> dict:
        if not self.samples:
            return {}
        s = sorted(self.samples)
        n = len(s)
        return {
            "count": n,
            "min_us": s[0] / 1000,
            "mean_us": sum(s) / n / 1000,
            "median_us": s[n // 2] / 1000,
            "p95_us": s[int(n * 0.95)] / 1000,
            "p99_us": s[int(n * 0.99)] / 1000,
            "max_us": s[-1] / 1000,
        }


def format_stats(stats: dict) -> str:
    """Format latency stats as a readable string."""
    if not stats:
        return "no samples"
    return (f"n={stats['count']}, mean={stats['mean_us']:.0f}µs, "
            f"median={stats['median_us']:.0f}µs, "
            f"p95={stats['p95_us']:.0f}µs, p99={stats['p99_us']:.0f}µs, "
            f"max={stats['max_us']:.0f}µs")
