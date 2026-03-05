"""Shared pytest fixtures for BACnet Python benchmarks."""

import asyncio
import pytest
import pytest_asyncio
from rusty_bacnet import BACnetServer, BACnetClient
from helpers import (
    populate_server,
    generate_tls_certs,
    PORT_BIP_SERVER_RUST,
    PORT_BIP_SERVER_PY,
    PORT_BIP_CLIENT_BASE,
)


# ── BIP Servers ──────────────────────────────────────────────────────────────

@pytest_asyncio.fixture(scope="module")
async def bip_server_a():
    """Rust-backed BACnet server on BIP (port 47808)."""
    server = BACnetServer(
        1000, "BenchServer-A", "127.0.0.1",
        PORT_BIP_SERVER_RUST, "127.0.0.255", "bip",
    )
    populate_server(server)
    await server.start()
    yield server
    await server.stop()


@pytest_asyncio.fixture(scope="module")
async def bip_server_b():
    """Second Rust-backed BACnet server on BIP (port 47809)."""
    server = BACnetServer(
        2000, "BenchServer-B", "127.0.0.1",
        PORT_BIP_SERVER_PY, "127.0.0.255", "bip",
    )
    populate_server(server)
    await server.start()
    yield server
    await server.stop()


# ── TLS Certificates ────────────────────────────────────────────────────────

@pytest.fixture(scope="session")
def tls_certs():
    """Generate ephemeral CA + server + client certs for SC benchmarks."""
    certs = generate_tls_certs()
    yield certs
    certs.cleanup()


# ── Utility ──────────────────────────────────────────────────────────────────

@pytest.fixture
def client_port(request):
    """Allocate a unique client port based on test index."""
    # Use a simple counter based on test node id hash
    port = PORT_BIP_CLIENT_BASE + (hash(request.node.nodeid) % 20)
    return port
