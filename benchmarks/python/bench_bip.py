"""Benchmarks: the Python bindings over B/IP on loopback.

The client and the server both run through the rusty_bacnet package, in
this one process, so each request crosses between Python and Rust on both
ends. This replaces three suites that measured the same path under names
from an older layout.
"""

import asyncio
import math
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
    PORT_BIP_SERVER_RUST,
)


SERVER_ADDR = f"127.0.0.1:{PORT_BIP_SERVER_RUST}"
AI_0 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 0)
AI_1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
AI_2 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 2)
AO_0 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 0)
PV = PropertyIdentifier.PRESENT_VALUE
OBJ_NAME = PropertyIdentifier.OBJECT_NAME

WARMUP = 20
ROUNDS = 200

# A server admits at most 16 confirmed requests in flight from one peer by
# default; the concurrency test keeps each client at or under it.
PER_CLIENT_LIMIT = 16
# The concurrency test's clients use these ports, one each.
PORT_CONCURRENCY_CLIENTS = 47840
MAX_CONCURRENCY_CLIENTS = 4


@pytest_asyncio.fixture(scope="module")
async def bip_server():
    server = BACnetServer(
        1000, "BenchServer", "127.0.0.1",
        PORT_BIP_SERVER_RUST, "127.0.0.255", "bip",
    )
    populate_server(server)
    await server.start()
    yield server
    await server.stop()


@pytest_asyncio.fixture(scope="module")
async def bip_client(bip_server):
    client = BACnetClient("127.0.0.1", 47820, "127.0.0.255", 3000, "bip")
    await client.__aenter__()
    await client.read_property(SERVER_ADDR, AI_0, PV)
    yield client
    await client.__aexit__(None, None, None)


@pytest_asyncio.fixture(scope="module")
async def bip_clients(bip_server):
    """Several clients, each its own peer to the server."""
    clients = []
    for i in range(MAX_CONCURRENCY_CLIENTS):
        client = BACnetClient(
            "127.0.0.1", PORT_CONCURRENCY_CLIENTS + i, "127.0.0.255", 3000, "bip",
        )
        await client.__aenter__()
        await client.read_property(SERVER_ADDR, AI_0, PV)
        clients.append(client)
    yield clients
    for client in clients:
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
async def test_bip_read_property_latency(bip_client):
    """ReadProperty latency over B/IP."""
    tracker = await async_bench(
        lambda: bip_client.read_property(SERVER_ADDR, AI_0, PV)
    )
    stats = tracker.stats()
    print(f"\n  ReadProperty: {format_stats(stats)}")
    assert stats["mean_us"] < 10000  # sanity: <10ms


@pytest.mark.asyncio
async def test_bip_read_property_throughput(bip_client):
    """ReadProperty throughput: 100 sequential reads."""
    t_start = time.perf_counter()
    for _ in range(100):
        await bip_client.read_property(SERVER_ADDR, AI_0, PV)
    elapsed = time.perf_counter() - t_start
    ops_sec = 100 / elapsed
    print(f"\n  ReadProperty throughput: {ops_sec:.0f} ops/s ({elapsed*1000:.0f}ms total)")
    assert ops_sec > 100


# ── WriteProperty ────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_bip_write_property_latency(bip_client):
    """WriteProperty latency over B/IP."""
    val = PropertyValue.real(75.0)
    tracker = await async_bench(
        lambda: bip_client.write_property(SERVER_ADDR, AO_0, PV, val, 16)
    )
    stats = tracker.stats()
    print(f"\n  WriteProperty: {format_stats(stats)}")
    assert stats["mean_us"] < 10000


# ── ReadPropertyMultiple ─────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_bip_rpm_latency(bip_client):
    """ReadPropertyMultiple: 3 objects × 2 properties, B/IP."""
    specs = [
        (AI_0, [(PV, None), (OBJ_NAME, None)]),
        (AI_1, [(PV, None), (OBJ_NAME, None)]),
        (AI_2, [(PV, None), (OBJ_NAME, None)]),
    ]
    tracker = await async_bench(
        lambda: bip_client.read_property_multiple(SERVER_ADDR, specs)
    )
    stats = tracker.stats()
    print(f"\n  RPM (3×2): {format_stats(stats)}")
    assert stats["mean_us"] < 20000


# ── COV Subscribe ────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_bip_cov_subscribe_latency(bip_client):
    """COV Subscribe + Unsubscribe round-trip, B/IP."""
    async def sub_unsub():
        await bip_client.subscribe_cov(SERVER_ADDR, 1, AI_0, True, 60)
        await bip_client.unsubscribe_cov(SERVER_ADDR, 1, AI_0)

    tracker = await async_bench(sub_unsub, warmup=5, rounds=100)
    stats = tracker.stats()
    print(f"\n  COV Sub/Unsub: {format_stats(stats)}")
    assert stats["mean_us"] < 20000


# ── WhoIs Discovery ─────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_bip_whois_fixed_wait(bip_client):
    """Who-Is, then a fixed 300 ms wait for I-Am: times the wait, not the stack."""
    tracker = LatencyTracker()
    for _ in range(20):
        bip_client.clear_devices()
        t0 = time.perf_counter_ns()
        await bip_client.who_is(1000, 1000)
        await asyncio.sleep(0.3)
        tracker.record(time.perf_counter_ns() - t0)
    stats = tracker.stats()
    print(f"\n  WhoIs+IAm: {format_stats(stats)}")


# ═══════════════════════════════════════════════════════════════════════════════
#  Concurrency Scaling (B/IP)
# ═══════════════════════════════════════════════════════════════════════════════

@pytest.mark.asyncio
@pytest.mark.parametrize("in_flight", [1, 5, 10, 16, 32, 64])
async def test_bip_concurrency_scaling(bip_clients, in_flight):
    """ReadProperty throughput with `in_flight` requests outstanding, spread
    over as few clients as keep each at or under the server's per-peer limit."""
    n_clients = math.ceil(in_flight / PER_CLIENT_LIMIT)
    assert n_clients <= len(bip_clients)
    per_client = math.ceil(in_flight / n_clients)
    assert per_client <= PER_CLIENT_LIMIT
    total_ops = in_flight * 50
    tracker = LatencyTracker()
    sems = [asyncio.Semaphore(per_client) for _ in range(n_clients)]

    async def single_op(i):
        client, sem = bip_clients[i % n_clients], sems[i % n_clients]
        async with sem:
            t0 = time.perf_counter_ns()
            await client.read_property(SERVER_ADDR, AI_0, PV)
            tracker.record(time.perf_counter_ns() - t0)

    t_start = time.perf_counter()
    await asyncio.gather(*[single_op(i) for i in range(total_ops)])
    elapsed = time.perf_counter() - t_start
    ops_sec = total_ops / elapsed

    stats = tracker.stats()
    print(f"\n  In flight={in_flight} over {n_clients} client(s): "
          f"{ops_sec:.0f} ops/s, {format_stats(stats)}")


# ═══════════════════════════════════════════════════════════════════════════════
#  Memory Tracking (B/IP)
# ═══════════════════════════════════════════════════════════════════════════════

@pytest.mark.asyncio
async def test_bip_memory_usage(bip_client):
    """Track RSS during sustained load."""
    proc = psutil.Process()
    rss_before = proc.memory_info().rss

    for _ in range(1000):
        await bip_client.read_property(SERVER_ADDR, AI_0, PV)

    rss_after = proc.memory_info().rss
    delta_mb = (rss_after - rss_before) / (1024 * 1024)
    print(f"\n  RSS: {rss_before / 1024 / 1024:.1f} MB → {rss_after / 1024 / 1024:.1f} MB (delta: {delta_mb:+.1f} MB)")
