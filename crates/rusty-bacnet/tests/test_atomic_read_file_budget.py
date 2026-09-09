"""Installed native AtomicReadFile policy and independent direct/routed UDP vectors."""
import asyncio
import inspect
import socket
import struct
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import BACnetServer


class AtomicReadFileConstructorTests(unittest.TestCase):
    def test_defaults_keyword_only_stub_and_positive_validation(self):
        signature = inspect.signature(BACnetServer)
        for name, default in [
            ("atomic_read_file_max_requested_stream_octets", 16384),
            ("atomic_read_file_max_requested_records", 256),
            ("atomic_read_file_max_service_ack_bytes", 16384),
        ]:
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
                             sc_ca_cert="ca.pem", sc_client_cert="cert.pem", sc_client_key="key.pem")


class AtomicReadFileNativeTests(unittest.IsolatedAsyncioTestCase):
    async def test_complete_ack_or_abort_direct_and_routed(self):
        for record in [False, True]:
            work_name = ("atomic_read_file_max_requested_records" if record else
                         "atomic_read_file_max_requested_stream_octets")
            # Independent service vectors: File:1, CHOICE, INTEGER start0,
            # Unsigned count, closure. Empty-record encoding remains present.
            normal_count = 2 if record else 5
            expected_ack = (b"\x11\x1e\x31\x00\x21\x02\x60\x65\x05abcde\x1f" if record
                            else b"\x11\x0e\x31\x00\x65\x05abcde\x0f")
            for policy, count, empty, abort in [
                ({}, normal_count, False, None),
                ({work_name: normal_count}, normal_count, False, None),
                ({work_name: normal_count - 1}, normal_count, False, 9),
                ({}, 257 if record else 16385, True, 9),
                ({"atomic_read_file_max_service_ack_bytes": len(expected_ack) - 1}, normal_count, False, 1),
                ({"atomic_read_file_max_service_ack_bytes": len(expected_ack)}, normal_count, False, None),
                ({"atomic_read_file_max_service_ack_bytes": 1}, 0, True, 1),
                ({}, 0, True, None),
                ({work_name: 10001 if record else 16385}, 10001 if record else 16385, True, None),
            ]:
                with self.subTest(record=record, policy=policy, count=count, empty=empty):
                    server = BACnetServer(123, interface="127.0.0.1", port=0,
                                          broadcast_address="127.0.0.1", **policy)
                    server.add_file(1, "File")
                    if record:
                        server.set_file_access_method(1, "record")
                        server.set_file_records(1, [] if empty else [b"", b"abcde"])
                    else:
                        server.set_file_data(1, b"" if empty else b"abcde")
                    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                    sock.bind(("127.0.0.1", 0))
                    sock.setblocking(False)
                    loop = asyncio.get_running_loop()
                    try:
                        await server.start()
                        ip, port = (await server.local_address()).rsplit(":", 1)
                        for routed in [False, True]:
                            for segmented in [False, True]:
                                for peer in [0, 3]:  # 50 and 480 byte peer APDUs
                                    invoke = 1 + peer + 4 * segmented + 8 * routed
                                    header = b"\x01\x08\x00\x07\x01\x09" if routed else b"\x01\x00"
                                    choice = 0x1e if record else 0x0e
                                    count_wire = bytes([0x21, count]) if count < 256 else b"\x22" + count.to_bytes(2, "big")
                                    request = b"\xc4\x02\x80\x00\x01" + bytes([choice, 0x31, 0]) + count_wire + bytes([choice + 1])
                                    npdu = header + bytes([2 if segmented else 0, peer, invoke, 6]) + request
                                    wire = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
                                    await loop.sock_sendto(sock, wire, (ip, int(port)))
                                    reply, _ = await asyncio.wait_for(loop.sock_recvfrom(sock, 4096), 2)
                                    self.assertEqual(reply[:2], b"\x81\x0a")
                                    self.assertEqual(int.from_bytes(reply[2:4], "big"), len(reply))
                                    if routed:
                                        self.assertEqual(reply[4:11], b"\x01\x20\x00\x07\x01\x09\xff")
                                    apdu = reply[11:] if routed else reply[6:]
                                    if abort is not None:
                                        self.assertEqual(apdu, bytes([0x71, invoke, abort]))
                                    else:
                                        expected = expected_ack
                                        if empty:
                                            expected = b"\x11\x1e\x31\x00\x21\x00\x1f" if record else b"\x11\x0e\x31\x00\x60\x0f"
                                        self.assertEqual(apdu, bytes([0x30, invoke, 6]) + expected)
                                    async with asyncio.timeout(2):
                                        while (await server.request_admission_counters())["confirmed_active"]:
                                            await asyncio.sleep(0)
                                    with self.assertRaises(BlockingIOError):
                                        sock.recvfrom(4096)
                    finally:
                        await server.stop()
                        sock.close()
