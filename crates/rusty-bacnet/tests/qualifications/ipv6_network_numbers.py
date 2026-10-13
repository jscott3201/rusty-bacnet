"""Installed full-server Number controls on an explicit isolated IPv6 link.

Run directly outside the checkout with RB_IPV6_TEST_ADDRESS/INDEX, like the
shared #887 packet-info oracle. Missing fixture/native inputs fail explicitly;
normal pytest does not collect this external-network qualification.
"""
import asyncio
import hashlib
import json
import os
from pathlib import Path
import socket

import rusty_bacnet
from ipv6_selected_link import ADDRESS, INDEX, GROUP, frame, receive, udp

PEER = b"\x40\x87\x91"
# The server's VMAC: its device instance, 321 (H.7.2).
NODE = b"\x00\x01\x41"
QUERY = b"\x01\x80\x12"


def nni(number, flag):
    return b"\x01\x80\x13" + number.to_bytes(2, "big") + bytes([flag])


def native_identity():
    module = Path(rusty_bacnet.__file__).resolve()
    assert "/workspace/" not in str(module), module
    native = {str(p): hashlib.sha256(p.read_bytes()).hexdigest()
              for p in module.parent.glob("*.so")}
    assert len(native) == 1, native
    return native


async def run():
    before = native_identity()
    print(json.dumps({"native_before": before}), flush=True)
    # Separate observer namespace avoids sender-local multicast reflection.
    remote = os.environ["RB_IPV6_OBSERVER_ADDRESS"]
    assert remote.startswith("fd") and remote != ADDRESS
    observer = socket.create_connection((remote, 47809), timeout=2)
    observed = observer.makefile("rb")
    # local_address() names the server's VMAC, not its port: pick the port.
    with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as probe:
        probe.bind((ADDRESS, 0))
        port = probe.getsockname()[1]
    node = NODE
    local = (ADDRESS, port)
    group = (GROUP, port, 0, INDEX)
    peer = udp(ADDRESS)
    server = rusty_bacnet.BACnetServer(
        321, transport="ipv6", port=port, ipv6_interface=ADDRESS)

    def send(npdu, broadcast):
        peer.sendto(frame(2 if broadcast else 1, PEER,
                          npdu if broadcast else node + npdu),
                    group if broadcast else local)

    async def fence():
        peer.sendto(frame(6, PEER, b""), local)
        data, source, destination = await asyncio.to_thread(
            receive, peer, lambda b: b[:2] == b"\x82\x07")
        assert len(data) == 10 and data[:4] == b"\x82\x07\x00\x0a"
        assert data[7:] == PEER
        assert data[4:7] == node
        assert source[1] == port and destination == ADDRESS

    async def expect(number):
        packet = json.loads(await asyncio.to_thread(observed.readline))
        data = bytes(packet["bytes"])
        # Do not filter by expected payload: an unsolicited/invalid-query reply
        # with the prior number must fail before the later positive marker.
        assert data == frame(2, node, nni(number, 0)), data.hex()
        assert packet["source"] == f"[{ADDRESS}]:{port}"
        assert packet["destination"] == GROUP and packet["index"] == observer_index
        print(json.dumps({"independent_observer": packet}), flush=True)

    try:
        await server.start()
        assert await server.local_address() == "bip6:0x000141"
        observer.sendall(json.dumps({"port": port}).encode() + b"\n")
        observer_index = json.loads(await asyncio.to_thread(observed.readline))["index"]
        await fence()
        send(nni(999, 1), False)  # Unicast announcements cannot teach UNKNOWN.
        send(QUERY, False)
        await fence()  # Same unicast path admitted before switching to group.
        send(QUERY, True)
        send(nni(77, 0), True)
        send(QUERY, True)
        await expect(77)
        send(QUERY, False)  # Valid local unicast query must reply by multicast.
        await expect(77)
        for value, flag, expected in [(78, 1, 78), (79, 0, 78), (80, 1, 80)]:
            send(nni(value, flag), True)
            send(QUERY, True)
            await expect(expected)
        for invalid in [b"\x01\x80\x13\x00\xc8",
                        b"\x01\x88\x00\x04\x01\x09\x13\x00\xc8\x01"]:
            send(invalid, True)
            send(QUERY, True)
            await expect(80)
        for i, invalid in enumerate([QUERY + b"\x00",
                                    b"\x01\x88\x00\x04\x01\x09\x12"]):
            send(invalid, False)
            await fence()
            send(nni(81 + i, 1), True)
            send(QUERY, True)
            await expect(81 + i)
    finally:
        await server.stop()
        observed.close()
        observer.close()
        peer.close()
    # No SO_REUSEADDR: completed server teardown releases the selected port.
    with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as rebound:
        rebound.bind(local)
    after = native_identity()
    assert before == after
    print(json.dumps({"native_after": after}), flush=True)
    print("installed Linux IPv6 full-server Number wire qualification passed")


if __name__ == "__main__":
    asyncio.run(run())
