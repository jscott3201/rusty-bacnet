"""Benchmarks: Python ↔ Python (both sides using rusty_bacnet PyO3 bindings).

Measures the full PyO3 round-trip overhead when both client and server
are hosted in the same Python process via the rusty_bacnet module.
"""

import asyncio
import time
import psutil
import pytest
import pytest_asyncio
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)
from helpers import (
    populate_server,
    LatencyTracker,
    format_stats,
    PORT_BIP_SERVER_PY,
)


SERVER_ADDR = f"127.0.0.1:{PORT_BIP_SERVER_PY}"
AI_0 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 0)
AI_1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
AI_2 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 2)
AO_0 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 0)
PV = PropertyIdentifier.PRESENT_VALUE
OBJ_NAME = PropertyIdentifier.OBJECT_NAME

WARMUP = 20
ROUNDS = 200


@pytest_asyncio.fixture(scope="module")
async def py_server():
    """Python-hosted BACnet server on BIP (port 47809)."""
    server = BACnetServer(
        2000, "PyBenchServer", "127.0.0.1",
        PORT_BIP_SERVER_PY, "127.0.0.255", "bip",
    )
    populate_server(server)
    await server.start()
    yield server
    await server.stop()


@pytest_asyncio.fixture(scope="module")
async def py_client(py_server):
    """Python client talking to the Python server."""
    client = BACnetClient("127.0.0.1", 47825, "127.0.0.255", 3000, "bip")
    await client.__aenter__()
    await client.read_property(SERVER_ADDR, AI_0, PV)
    yield client
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


# ── ReadProperty ─────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_pypy_bip_read_property_latency(py_client):
    """ReadProperty latency: Python client → Python server, BIP."""
    tracker = await async_bench(
        lambda: py_client.read_property(SERVER_ADDR, AI_0, PV)
    )
    stats = tracker.stats()
    print(f"\n  ReadProperty: {format_stats(stats)}")
    assert stats["mean_us"] < 10000


@pytest.mark.asyncio
async def test_pypy_bip_read_property_throughput(py_client):
    """ReadProperty throughput: 100 sequential reads, Py→Py."""
    t_start = time.perf_counter()
    for _ in range(100):
        await py_client.read_property(SERVER_ADDR, AI_0, PV)
    elapsed = time.perf_counter() - t_start
    ops_sec = 100 / elapsed
    print(f"\n  ReadProperty throughput: {ops_sec:.0f} ops/s ({elapsed*1000:.0f}ms total)")


# ── WriteProperty ────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_pypy_bip_write_property_latency(py_client):
    """WriteProperty latency: Python → Python, BIP."""
    val = PropertyValue.real(75.0)
    tracker = await async_bench(
        lambda: py_client.write_property(SERVER_ADDR, AO_0, PV, val, 16)
    )
    stats = tracker.stats()
    print(f"\n  WriteProperty: {format_stats(stats)}")


# ── ReadPropertyMultiple ─────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_pypy_bip_rpm_latency(py_client):
    """ReadPropertyMultiple: 3 objects × 2 properties, Py→Py."""
    specs = [
        (AI_0, [(PV, None), (OBJ_NAME, None)]),
        (AI_1, [(PV, None), (OBJ_NAME, None)]),
        (AI_2, [(PV, None), (OBJ_NAME, None)]),
    ]
    tracker = await async_bench(
        lambda: py_client.read_property_multiple(SERVER_ADDR, specs)
    )
    stats = tracker.stats()
    print(f"\n  RPM (3×2): {format_stats(stats)}")


# ── COV Subscribe ────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_pypy_bip_cov_subscribe_latency(py_client):
    """COV Subscribe + Unsubscribe, Py→Py BIP."""
    async def sub_unsub():
        await py_client.subscribe_cov(SERVER_ADDR, 1, AI_0, True, 60)
        await py_client.unsubscribe_cov(SERVER_ADDR, 1, AI_0)

    tracker = await async_bench(sub_unsub, warmup=5, rounds=100)
    stats = tracker.stats()
    print(f"\n  COV Sub/Unsub: {format_stats(stats)}")


# ── WhoIs Discovery ─────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_pypy_bip_whois_latency(py_client):
    """WhoIs broadcast + collection, Py→Py BIP."""
    tracker = LatencyTracker()
    for _ in range(20):
        py_client.clear_devices()
        t0 = time.perf_counter_ns()
        await py_client.who_is(2000, 2000)
        await asyncio.sleep(0.3)
        tracker.record(time.perf_counter_ns() - t0)
    stats = tracker.stats()
    print(f"\n  WhoIs+IAm: {format_stats(stats)}")


# ═══════════════════════════════════════════════════════════════════════════════
#  Concurrency Scaling
# ═══════════════════════════════════════════════════════════════════════════════

@pytest.mark.asyncio
@pytest.mark.parametrize("concurrency", [1, 5, 10, 25])
async def test_pypy_bip_concurrency_scaling(py_client, concurrency):
    """ReadProperty throughput at different concurrency, Py→Py."""
    total_ops = concurrency * 50
    tracker = LatencyTracker()
    sem = asyncio.Semaphore(concurrency)

    async def single_op():
        async with sem:
            t0 = time.perf_counter_ns()
            await py_client.read_property(SERVER_ADDR, AI_0, PV)
            tracker.record(time.perf_counter_ns() - t0)

    t_start = time.perf_counter()
    await asyncio.gather(*[single_op() for _ in range(total_ops)])
    elapsed = time.perf_counter() - t_start
    ops_sec = total_ops / elapsed

    stats = tracker.stats()
    print(f"\n  Concurrency={concurrency}: {ops_sec:.0f} ops/s, {format_stats(stats)}")


# ═══════════════════════════════════════════════════════════════════════════════
#  Memory
# ═══════════════════════════════════════════════════════════════════════════════

@pytest.mark.asyncio
async def test_pypy_bip_memory_usage(py_client):
    """Track RSS during sustained Py→Py load."""
    proc = psutil.Process()
    rss_before = proc.memory_info().rss

    for _ in range(1000):
        await py_client.read_property(SERVER_ADDR, AI_0, PV)

    rss_after = proc.memory_info().rss
    delta_mb = (rss_after - rss_before) / (1024 * 1024)
    print(f"\n  RSS: {rss_before / 1024 / 1024:.1f} MB → {rss_after / 1024 / 1024:.1f} MB (delta: {delta_mb:+.1f} MB)")
