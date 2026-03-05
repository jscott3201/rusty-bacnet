"""Benchmarks: Rust client (subprocess) → Python-hosted BACnetServer.

Measures the performance when a client talks to a Python-hosted server.
The key difference from Py→Rust: here the server is started from Python,
so we measure server-side PyO3 overhead.
"""

import asyncio
import time
import psutil
import pytest
import pytest_asyncio
from rusty_bacnet import (
    BACnetServer,
    BACnetClient,
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
async def py_server_for_rust():
    """Python-hosted BACnet server that clients will connect to."""
    server = BACnetServer(
        2000, "PyServer-ForRust", "127.0.0.1",
        PORT_BIP_SERVER_PY, "127.0.0.255", "bip",
    )
    populate_server(server)
    await server.start()
    yield server
    await server.stop()


@pytest_asyncio.fixture(scope="module")
async def rust_proxy_client(py_server_for_rust):
    """Client talking to Python-hosted server."""
    client = BACnetClient("127.0.0.1", 47826, "127.0.0.255", 3000, "bip")
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
async def test_rust_py_bip_read_property_latency(rust_proxy_client):
    """ReadProperty latency: client → Python server, BIP."""
    tracker = await async_bench(
        lambda: rust_proxy_client.read_property(SERVER_ADDR, AI_0, PV)
    )
    stats = tracker.stats()
    print(f"\n  ReadProperty: {format_stats(stats)}")
    assert stats["mean_us"] < 10000


@pytest.mark.asyncio
async def test_rust_py_bip_read_property_throughput(rust_proxy_client):
    """ReadProperty throughput: 100 sequential reads, → Python server."""
    t_start = time.perf_counter()
    for _ in range(100):
        await rust_proxy_client.read_property(SERVER_ADDR, AI_0, PV)
    elapsed = time.perf_counter() - t_start
    ops_sec = 100 / elapsed
    print(f"\n  ReadProperty throughput: {ops_sec:.0f} ops/s ({elapsed*1000:.0f}ms total)")


# ── WriteProperty ────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_rust_py_bip_write_property_latency(rust_proxy_client):
    """WriteProperty latency: → Python server, BIP."""
    val = PropertyValue.real(75.0)
    tracker = await async_bench(
        lambda: rust_proxy_client.write_property(SERVER_ADDR, AO_0, PV, val, 16)
    )
    stats = tracker.stats()
    print(f"\n  WriteProperty: {format_stats(stats)}")


# ── RPM ──────────────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_rust_py_bip_rpm_latency(rust_proxy_client):
    """ReadPropertyMultiple: 3 objects × 2 properties, → Python server."""
    specs = [
        (AI_0, [(PV, None), (OBJ_NAME, None)]),
        (AI_1, [(PV, None), (OBJ_NAME, None)]),
        (AI_2, [(PV, None), (OBJ_NAME, None)]),
    ]
    tracker = await async_bench(
        lambda: rust_proxy_client.read_property_multiple(SERVER_ADDR, specs)
    )
    stats = tracker.stats()
    print(f"\n  RPM (3×2): {format_stats(stats)}")


# ── COV ──────────────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_rust_py_bip_cov_subscribe_latency(rust_proxy_client):
    """COV Subscribe + Unsubscribe, → Python server."""
    async def sub_unsub():
        await rust_proxy_client.subscribe_cov(SERVER_ADDR, 1, AI_0, True, 60)
        await rust_proxy_client.unsubscribe_cov(SERVER_ADDR, 1, AI_0)

    tracker = await async_bench(sub_unsub, warmup=5, rounds=100)
    stats = tracker.stats()
    print(f"\n  COV Sub/Unsub: {format_stats(stats)}")


# ── WhoIs ────────────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_rust_py_bip_whois_latency(rust_proxy_client):
    """WhoIs discovery, → Python server."""
    tracker = LatencyTracker()
    for _ in range(20):
        rust_proxy_client.clear_devices()
        t0 = time.perf_counter_ns()
        await rust_proxy_client.who_is(2000, 2000)
        await asyncio.sleep(0.3)
        tracker.record(time.perf_counter_ns() - t0)
    stats = tracker.stats()
    print(f"\n  WhoIs+IAm: {format_stats(stats)}")


# ═══════════════════════════════════════════════════════════════════════════════
#  Concurrency Scaling
# ═══════════════════════════════════════════════════════════════════════════════

@pytest.mark.asyncio
@pytest.mark.parametrize("concurrency", [1, 5, 10, 25])
async def test_rust_py_bip_concurrency_scaling(rust_proxy_client, concurrency):
    """ReadProperty throughput at different concurrency, → Python server."""
    total_ops = concurrency * 50
    tracker = LatencyTracker()
    sem = asyncio.Semaphore(concurrency)

    async def single_op():
        async with sem:
            t0 = time.perf_counter_ns()
            await rust_proxy_client.read_property(SERVER_ADDR, AI_0, PV)
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
async def test_rust_py_bip_memory_usage(rust_proxy_client):
    """Track RSS during sustained load → Python server."""
    proc = psutil.Process()
    rss_before = proc.memory_info().rss

    for _ in range(1000):
        await rust_proxy_client.read_property(SERVER_ADDR, AI_0, PV)

    rss_after = proc.memory_info().rss
    delta_mb = (rss_after - rss_before) / (1024 * 1024)
    print(f"\n  RSS: {rss_before / 1024 / 1024:.1f} MB → {rss_after / 1024 / 1024:.1f} MB (delta: {delta_mb:+.1f} MB)")
