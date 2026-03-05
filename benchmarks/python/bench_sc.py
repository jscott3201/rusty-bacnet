"""Benchmarks: BACnet/SC (TLS) and SC-mTLS transport benchmarks.

Tests SC transport with Python client → Rust server over TLS WebSocket.
Measures TLS handshake + WS framing overhead vs BIP baseline.

Architecture: ScHub (relay) ← BACnetServer (SC node) ← BACnetClient (SC node)
"""

import asyncio
import time
import psutil
import pytest
import pytest_asyncio
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    ScHub,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)
from helpers import (
    populate_server,
    generate_tls_certs,
    LatencyTracker,
    format_stats,
)

AI_0 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 0)
AI_1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
AI_2 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 2)
AO_0 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 0)
PV = PropertyIdentifier.PRESENT_VALUE
OBJ_NAME = PropertyIdentifier.OBJECT_NAME

WARMUP = 10
ROUNDS = 100


@pytest.fixture(scope="module")
def certs():
    c = generate_tls_certs()
    yield c
    c.cleanup()


@pytest_asyncio.fixture(scope="module")
async def sc_hub(certs):
    """SC Hub (TLS relay) on a random port."""
    hub = ScHub(
        listen="127.0.0.1:0",
        cert=certs.server_cert,
        key=certs.server_key,
        vmac=b"\x00\x00\x00\x00\x00\x01",
    )
    await hub.start()
    yield hub
    await hub.stop()


@pytest_asyncio.fixture(scope="module")
async def sc_server(sc_hub, certs):
    """SC BACnet server connecting to the hub."""
    hub_url = await sc_hub.url()
    server = BACnetServer(
        3000, "SC-BenchServer", "127.0.0.1", 47900, "127.0.0.255", "sc",
        sc_hub=hub_url,
        sc_vmac=b"\x00\x01\x02\x03\x04\x05",
        sc_ca_cert=certs.ca_cert,
        sc_client_cert=certs.server_cert,
        sc_client_key=certs.server_key,
    )
    populate_server(server)
    await server.start()
    yield server
    await server.stop()


@pytest_asyncio.fixture(scope="module")
async def sc_client(sc_server, sc_hub, certs):
    """SC client connecting to the hub."""
    hub_url = await sc_hub.url()
    client = BACnetClient(
        "127.0.0.1", 47910, "127.0.0.255", 5000, "sc",
        sc_hub=hub_url,
        sc_vmac=b"\x00\x01\x02\x03\x04\x06",
        sc_ca_cert=certs.ca_cert,
        sc_client_cert=certs.server_cert,
        sc_client_key=certs.server_key,
    )
    await client.__aenter__()
    # Wait for hub handshake to complete
    await asyncio.sleep(1)
    # In SC, address the server by its VMAC (hex-colon notation)
    server_vmac = "00:01:02:03:04:05"
    for attempt in range(5):
        try:
            await client.read_property(server_vmac, AI_0, PV)
            break
        except Exception:
            if attempt == 4:
                raise
            await asyncio.sleep(1)
    yield client, server_vmac
    await client.__aexit__(None, None, None)


async def async_bench(coro_fn, warmup=WARMUP, rounds=ROUNDS):
    """Run an async benchmark, return LatencyTracker."""
    for _ in range(warmup):
        await coro_fn()
    tracker = LatencyTracker()
    for _ in range(rounds):
        t0 = time.perf_counter_ns()
        await coro_fn()
        tracker.record(time.perf_counter_ns() - t0)
    return tracker


# ── SC ReadProperty ──────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_sc_read_property_latency(sc_client):
    """ReadProperty latency over BACnet/SC (TLS)."""
    client, addr = sc_client
    tracker = await async_bench(
        lambda: client.read_property(addr, AI_0, PV)
    )
    stats = tracker.stats()
    print(f"\n  SC ReadProperty: {format_stats(stats)}")
    assert stats["mean_us"] < 50000  # SC is slower due to TLS


@pytest.mark.asyncio
async def test_sc_read_property_throughput(sc_client):
    """ReadProperty throughput: 100 sequential reads, SC."""
    client, addr = sc_client
    t_start = time.perf_counter()
    for _ in range(100):
        await client.read_property(addr, AI_0, PV)
    elapsed = time.perf_counter() - t_start
    ops_sec = 100 / elapsed
    print(f"\n  SC ReadProperty throughput: {ops_sec:.0f} ops/s ({elapsed*1000:.0f}ms total)")


# ── SC WriteProperty ─────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_sc_write_property_latency(sc_client):
    """WriteProperty latency over SC."""
    client, addr = sc_client
    val = PropertyValue.real(75.0)
    tracker = await async_bench(
        lambda: client.write_property(addr, AO_0, PV, val, 16)
    )
    stats = tracker.stats()
    print(f"\n  SC WriteProperty: {format_stats(stats)}")


# ── SC RPM ───────────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_sc_rpm_latency(sc_client):
    """ReadPropertyMultiple over SC."""
    client, addr = sc_client
    specs = [
        (AI_0, [(PV, None), (OBJ_NAME, None)]),
        (AI_1, [(PV, None), (OBJ_NAME, None)]),
        (AI_2, [(PV, None), (OBJ_NAME, None)]),
    ]
    tracker = await async_bench(
        lambda: client.read_property_multiple(addr, specs)
    )
    stats = tracker.stats()
    print(f"\n  SC RPM (3×2): {format_stats(stats)}")


# ── SC COV ───────────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_sc_cov_subscribe_latency(sc_client):
    """COV Subscribe + Unsubscribe over SC."""
    client, addr = sc_client

    async def sub_unsub():
        await client.subscribe_cov(addr, 1, AI_0, True, 60)
        await client.unsubscribe_cov(addr, 1, AI_0)

    tracker = await async_bench(sub_unsub, warmup=3, rounds=50)
    stats = tracker.stats()
    print(f"\n  SC COV Sub/Unsub: {format_stats(stats)}")


# ═══════════════════════════════════════════════════════════════════════════════
#  SC Concurrency Scaling
# ═══════════════════════════════════════════════════════════════════════════════

@pytest.mark.asyncio
@pytest.mark.parametrize("concurrency", [1, 5, 10, 25])
async def test_sc_concurrency_scaling(sc_client, concurrency):
    """ReadProperty throughput over SC at different concurrency."""
    client, addr = sc_client
    total_ops = concurrency * 25
    tracker = LatencyTracker()
    sem = asyncio.Semaphore(concurrency)

    async def single_op():
        async with sem:
            t0 = time.perf_counter_ns()
            await client.read_property(addr, AI_0, PV)
            tracker.record(time.perf_counter_ns() - t0)

    t_start = time.perf_counter()
    await asyncio.gather(*[single_op() for _ in range(total_ops)])
    elapsed = time.perf_counter() - t_start
    ops_sec = total_ops / elapsed

    stats = tracker.stats()
    print(f"\n  SC Concurrency={concurrency}: {ops_sec:.0f} ops/s, {format_stats(stats)}")


# ═══════════════════════════════════════════════════════════════════════════════
#  Memory
# ═══════════════════════════════════════════════════════════════════════════════

@pytest.mark.asyncio
async def test_sc_memory_usage(sc_client):
    """Track RSS during sustained SC load."""
    client, addr = sc_client
    proc = psutil.Process()
    rss_before = proc.memory_info().rss

    for _ in range(500):
        await client.read_property(addr, AI_0, PV)

    rss_after = proc.memory_info().rss
    delta_mb = (rss_after - rss_before) / (1024 * 1024)
    print(f"\n  SC RSS: {rss_before / 1024 / 1024:.1f} MB → {rss_after / 1024 / 1024:.1f} MB (delta: {delta_mb:+.1f} MB)")
