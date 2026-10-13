"""Explicit installed-extension qualification on a task-owned internal IPv6 link.

Run this file directly outside the checkout with RB_IPV6_TEST_ADDRESS/INDEX.
It fails if the fixture is absent; normal pytest does not collect this target.
"""
import asyncio
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import socket
import struct
import time
import rusty_bacnet

ADDRESS = os.environ["RB_IPV6_TEST_ADDRESS"]
INDEX = int(os.environ["RB_IPV6_TEST_INDEX"])
assert ipaddress.IPv6Address(ADDRESS).is_private and ADDRESS.startswith("fd")
assert INDEX > 0
GROUP = "ff05::bac0"
DEADLINE = 2


def udp(address="::", port=0, join=False):
    sock = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)
    sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_RECVPKTINFO, 1)
    sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_MULTICAST_IF, INDEX)
    sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_MULTICAST_LOOP, 1)
    sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_MULTICAST_HOPS, 0)
    sock.bind((address, port))
    if join:
        sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_JOIN_GROUP,
                        socket.inet_pton(socket.AF_INET6, GROUP) + struct.pack("@I", INDEX))
    return sock


def frame(function, vmac, payload):
    return bytes([0x82, function]) + (7 + len(payload)).to_bytes(2, "big") + vmac + payload


def receive(sock, predicate):
    end = time.monotonic() + DEADLINE
    while True:
        remaining = end - time.monotonic()
        assert remaining > 0, "expected independent wire frame absent"
        sock.settimeout(remaining)
        data, control, flags, source = sock.recvmsg(2048, 256)
        assert not flags & (socket.MSG_TRUNC | socket.MSG_CTRUNC)
        infos = [v for level, kind, v in control
                 if level == socket.IPPROTO_IPV6 and kind == socket.IPV6_PKTINFO]
        assert len(infos) == 1 and len(infos[0]) == 20
        destination = socket.inet_ntop(socket.AF_INET6, infos[0][:16])
        index = struct.unpack("@I", infos[0][16:])[0]
        assert int.from_bytes(data[2:4], "big") == len(data)
        print(json.dumps({"observed": data.hex(), "source": source, "destination": destination, "index": index}), flush=True)
        if predicate(data):
            assert source[0] == ADDRESS and index == INDEX
            return data, source, destination


async def server_case(explicit):
    observer = udp(join=True)
    port = observer.getsockname()[1]
    peer = udp(ADDRESS)
    kwargs = {"ipv6_interface": ADDRESS} if explicit else {}
    server = rusty_bacnet.BACnetServer(321, transport="ipv6", port=port, **kwargs)
    try:
        await server.start()
        local = await server.local_address()
        # The server's VMAC is its device instance, 321 (H.7.2).
        assert local == "bip6:0x000141", local
        # The independently encoded group Who-Is asks only for Device 321.
        npdu = b"\x01\x00\x10\x08\x0a\x01\x41\x1a\x01\x41"
        peer.sendto(frame(2, b"\x40\x88\x31", npdu), (GROUP, port, 0, INDEX))
        data, source, destination = await asyncio.to_thread(
            receive, peer,
            lambda b: b[:2] == b"\x82\x01" and b[10:14] == b"\x01\x00\x10\x00")
        assert source[1] == port and destination == ADDRESS
        assert data[4:7] == b"\x00\x01\x41" and data[7:10] == b"\x40\x88\x31"
        assert data[14] == 0xc4 and int.from_bytes(data[15:19], "big") == (8 << 22) | 321
        print(json.dumps({"server_explicit": explicit, "published": local,
                          "iam_source": source, "destination": destination, "frame": data.hex()}))
    finally:
        await server.stop()
        observer.close()
        peer.close()


async def client_case(explicit):
    observer = udp(join=True)
    port = observer.getsockname()[1]
    peer = udp(ADDRESS)
    kwargs = {"ipv6_interface": ADDRESS} if explicit else {}
    try:
        async with rusty_bacnet.BACnetClient(transport="ipv6", port=port, **kwargs) as client:
            await client.who_is()
            data, source, destination = await asyncio.to_thread(
                receive, observer, lambda b: b[:2] == b"\x82\x02" and b[7:] == b"\x01\x20\xff\xff\x00\xff\x10\x08")
            assert source[1] == port and destination == GROUP
            # Drop the shared-port observer before unicast so only the client
            # can consume the independently encoded I-Am reply.
            observer.close()
            iam = b"\x01\x00\x10\x00\xc4" + ((8 << 22) | 654).to_bytes(4, "big")
            iam += b"\x22\x04\x00\x91\x03\x22\x03\xe7"
            peer.sendto(frame(1, b"\x40\x88\x32", data[4:7] + iam), source)
            async with asyncio.timeout(DEADLINE):
                while True:
                    devices = await client.discovered_devices()
                    if devices:
                        break
                    await asyncio.sleep(0)
            assert len(devices) == 1
            device = devices[0]
            assert device.object_identifier.instance == 654
            # A B/IPv6 device's MAC is the VMAC it sent from (U.1.3).
            assert device.mac_address == b"\x40\x88\x32"
            assert device.max_apdu_length == 1024 and device.vendor_id == 999
            print(json.dumps({"client_explicit": explicit, "who_is_source": source,
                              "destination": destination, "frame": data.hex(), "discovered": 654}))
    finally:
        observer.close()
        peer.close()


async def main():
    path = Path(rusty_bacnet.__file__).resolve()
    assert "/workspace/" not in str(path), path
    print(json.dumps({"installed_module": str(path), "native": {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in path.parent.glob("*.so")}}))
    # The public server default sends a directed I-Am to the group requester.
    for explicit in [False, True]:
        await server_case(explicit)
        await client_case(explicit)
    print(json.dumps({"native_after": {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in path.parent.glob("*.so")}}))
    print("4 installed Linux IPv6 server/client cases passed")


if __name__ == "__main__":
    asyncio.run(main())
