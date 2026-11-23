"""Installed native ReadRange pages using independent direct/routed UDP vectors."""
import asyncio
import inspect
import socket
import struct
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import (BACnetClient, BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier,
                          Segmentation)

CALENDAR = ObjectIdentifier(ObjectType.CALENDAR, 1)
# ReadRange of Calendar:1 Date_List, a BACnetLIST (ReadRange refuses arrays
# such as Object_List): context object identifier [0], property identifier [1].
REQUEST = b"\x0c\x01\x80\x00\x01\x19\x17"


def date_entries(count):
    """Distinct date [0] calendar entries, five octets each like an object identifier."""
    return [bytes([0x0C, 126, 1 + i // 28, 1 + i % 28, 0xFF]) for i in range(count)]


async def fill_date_list(server, entries):
    address = await server.local_address()
    async with BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000) as client:
        for start in range(0, len(entries), 64):
            await client.add_list_element(address, CALENDAR, PropertyIdentifier.DATE_LIST,
                                          b"".join(entries[start:start + 64]))


class ReadRangeConstructorTests(unittest.TestCase):
    def test_defaults_keyword_only_stub_and_positive_validation(self):
        signature = inspect.signature(BACnetServer)
        for name, default in [("read_range_max_returned_items", 256),
                              ("read_range_max_service_ack_bytes", 16384)]:
            parameter = signature.parameters[name]
            self.assertEqual(parameter.default, default)
            self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertIn(f"{name}: int = {default}",
                          Path(rusty_bacnet.__file__).with_suffix(".pyi").read_text())
            for transport in ["bip", "ipv6", "sc", "mstp"]:
                for value, error in [(0, ValueError), (-1, OverflowError),
                                     (1 << 200, OverflowError), (1.5, TypeError)]:
                    with self.subTest(name=name, transport=transport, value=value):
                        with self.assertRaises(error):
                            BACnetServer(123, transport=transport, **{name: value})
                positive: dict[str, Any] = {name: (1 << (8 * struct.calcsize("P"))) - 1}
                BACnetServer(123, transport=transport, **positive,
                             sc_device_uuid=bytes.fromhex("8e62ac46d7084226913776a32b619315"),
                             sc_ca_cert="ca.pem", sc_client_cert="cert.pem", sc_client_key="key.pem")


class ReadRangeNativeTests(unittest.IsolatedAsyncioTestCase):
    async def exchange(self, server, sock, service, routed, segmented, peer, invoke):
        ip, port = (await server.local_address()).rsplit(":", 1)
        header = b"\x01\x08\x00\x07\x01\x09" if routed else b"\x01\x00"
        npdu = header + bytes([2 if segmented else 0, peer, invoke, 26]) + service
        wire = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
        loop = asyncio.get_running_loop()
        await loop.sock_sendto(sock, wire, (ip, int(port)))
        reply, _ = await asyncio.wait_for(loop.sock_recvfrom(sock, 4096), 2)
        self.assertEqual(reply[:2], b"\x81\x0a")
        self.assertEqual(int.from_bytes(reply[2:4], "big"), len(reply))
        if routed:
            self.assertEqual(reply[4:11], b"\x01\x20\x00\x07\x01\x09\xff")
        apdu = reply[11:] if routed else reply[6:]
        self.assertLessEqual(len(apdu), {0: 50, 3: 480, 5: 1476}[peer])
        async with asyncio.timeout(2):
            while (await server.request_admission_counters())["confirmed_active"]:
                await asyncio.sleep(0)
        return apdu

    async def test_directional_pages_bytes_peer_limits_and_abort_wire(self):
        request = REQUEST
        entries = date_entries(7)
        for policy, capacity in [({}, 7), ({"read_range_max_returned_items": 1}, 1),
                                 ({"read_range_max_returned_items": 2}, 2),
                                 ({"read_range_max_service_ack_bytes": 24}, 2),
                                 ({"read_range_max_service_ack_bytes": 23}, 1),
                                 ({"read_range_max_service_ack_bytes": 18}, 0),
                                 ({"read_range_max_service_ack_bytes": 13}, 0)]:
            # One APDU per answer, so the budget trims to the peer's limit
            # rather than segmenting past it (#2617).
            server = BACnetServer(123, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", **policy,
                                  segmentation_supported=Segmentation.NO_SEGMENTATION)
            server.add_calendar(1, "CAL-1")
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.bind(("127.0.0.1", 0))
            sock.setblocking(False)
            try:
                await server.start()
                await fill_date_list(server, entries)
                for routed in [False, True]:
                    for segmented in [False, True]:
                        for peer in [0, 3]:
                            for direction in [0, 1, -1]:
                                with self.subTest(policy=policy, routed=routed, segmented=segmented,
                                                  peer=peer, direction=direction):
                                    invoke = 11 + direction
                                    range_wire = b"" if direction == 0 else bytes([
                                        0x3e, 0x21, 7 if direction < 0 else 1, 0x31,
                                        249 if direction < 0 else 7, 0x3f])
                                    apdu = await self.exchange(server, sock, request + range_wire,
                                                               routed, segmented, peer, invoke)
                                    n = min(capacity, 6 if peer == 0 else 7)
                                    if n == 0:
                                        self.assertEqual(apdu, bytes([0x71, invoke, 1]))
                                    else:
                                        chosen = entries[-n:] if direction < 0 else entries[:n]
                                        flags = (0x40 if direction < 0 else 0x80) | 0x20 if n < 7 else 0xc0
                                        ack = request + bytes([0x3a, 5, flags, 0x49, n, 0x5e]) + b"".join(chosen) + b"\x5f"
                                        self.assertEqual(apdu, bytes([0x30, invoke, 26]) + ack)
                # Missing reference is a genuine empty match, never MORE_ITEMS.
                empty = await self.exchange(server, sock, request + b"\x3e\x21\x08\x31\x01\x3f", False, False, 3, 42)
                if policy.get("read_range_max_service_ack_bytes", 16384) < 14:
                    self.assertEqual(empty, b"\x71\x2a\x01")
                else:
                    self.assertEqual(empty, b"\x30\x2a\x1a" + request + b"\x3a\x05\x00\x49\x00\x5e\x5f")
            finally:
                await server.stop()
                sock.close()

    async def test_default_257_and_forward_backward_continuation(self):
        server = BACnetServer(123, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
        server.add_calendar(1, "CAL-1")
        entries = date_entries(257)
        request = REQUEST
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        sock.setblocking(False)
        try:
            await server.start()
            await fill_date_list(server, entries)
            for backwards in [False, True]:
                reference = 257 if backwards else 1
                count = -257 if backwards else 257
                service = request + b"\x3e\x22" + reference.to_bytes(2, "big") + b"\x32" + count.to_bytes(2, "big", signed=True) + b"\x3f"
                apdu = await self.exchange(server, sock, service, True, True, 5, 77)
                flags = 0x60 if backwards else 0xa0
                chosen = entries[1:] if backwards else entries[:256]
                self.assertEqual(apdu, b"\x30\x4d\x1a" + request + bytes([0x3a, 5, flags, 0x4a, 1, 0, 0x5e]) + b"".join(chosen) + b"\x5f")
                reference = 1 if backwards else 257
                service = request + b"\x3e\x22" + reference.to_bytes(2, "big") + bytes([0x31, 255 if backwards else 1, 0x3f])
                final = await self.exchange(server, sock, service, True, True, 5, 78)
                flags = 0x80 if backwards else 0x40
                last = entries[0] if backwards else entries[-1]
                self.assertEqual(final, b"\x30\x4e\x1a" + request + bytes([0x3a, 5, flags, 0x49, 1, 0x5e]) + last + b"\x5f")
                self.assertEqual((last + b"".join(chosen)) if backwards else (b"".join(chosen) + last), b"".join(entries))
        finally:
            await server.stop()
            sock.close()
