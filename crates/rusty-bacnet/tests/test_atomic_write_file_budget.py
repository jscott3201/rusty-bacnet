"""Installed native write admission: independent direct/routed wire and state proof."""
import asyncio
import inspect
import socket
import struct
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import BACnetServer


class AtomicWriteFileConstructorTests(unittest.TestCase):
    def test_defaults_keyword_only_stub_and_positive_validation(self):
        signature = inspect.signature(BACnetServer)
        for name, default in [
            ("atomic_write_file_max_stream_payload_octets", 16384),
            ("atomic_write_file_max_records", 256),
            ("atomic_write_file_max_record_payload_bytes", 16384),
        ]:
            parameter = signature.parameters[name]
            self.assertEqual(parameter.default, default)
            self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertIn(f"{name}: int = {default}",
                          Path(rusty_bacnet.__file__).with_suffix(".pyi").read_text())
            for transport in ["bip", "ipv6", "sc", "mstp"]:
                for value, error in [(0, ValueError), (-1, OverflowError),
                                     (1 << 200, OverflowError), (1.5, TypeError), ("2", TypeError)]:
                    with self.subTest(name=name, transport=transport, value=value):
                        with self.assertRaises(error):
                            BACnetServer(123, transport=transport, **{name: value})
                positive: dict[str, Any] = {name: (1 << (8 * struct.calcsize("P"))) - 1}
                BACnetServer(123, transport=transport, **positive)


def unsigned(value):
    data = value.to_bytes(max(1, (value.bit_length() + 7) // 8), "big")
    return bytes([0x20 + len(data)]) + data


def octets(value):
    if len(value) < 5:
        return bytes([0x60 + len(value)]) + value
    if len(value) < 254:
        return b"\x65" + bytes([len(value)]) + value
    return b"\x65\xfe" + len(value).to_bytes(2, "big") + value


class AtomicWriteFileNativeTests(unittest.IsolatedAsyncioTestCase):
    async def test_whole_ack_or_abort_and_readback_direct_routed(self):
        for record in [False, True]:
            work = "atomic_write_file_max_records" if record else "atomic_write_file_max_stream_payload_octets"
            normal = [b"", b"abcde"] if record else [b"abcde"]
            for policy, payload, refused in [
                ({}, normal, False),
                ({work: 2 if record else 5}, normal, False),
                ({work: 1 if record else 4}, normal, True),
                ({"atomic_write_file_max_record_payload_bytes": 5}, normal, False),
                ({"atomic_write_file_max_record_payload_bytes": 4}, normal, record),
                ({}, [b""] * (257 if record else 1), record),
                ({}, [], False),
                ({}, [b""], False),
                ({"atomic_write_file_max_record_payload_bytes": 5}, [b"abc", b"def"], record),
            ]:
                for routed in [False, True]:
                    for segmented in [False, True]:
                        with self.subTest(record=record, policy=policy, count=len(payload), routed=routed, segmented=segmented):
                            server = BACnetServer(123, interface="127.0.0.1", port=0,
                                                  broadcast_address="127.0.0.1", **policy)
                            server.add_file(1, "file")
                            if record:
                                server.set_file_access_method(1, "record")
                                server.set_file_records(1, [b"sentinel"])
                            else:
                                server.set_file_data(1, b"sentinel")
                            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                            sock.bind(("127.0.0.1", 0))
                            sock.setblocking(False)
                            loop = asyncio.get_running_loop()
                            try:
                                await server.start()
                                ip, port = (await server.local_address()).rsplit(":", 1)

                                async def exchange(service, request, invoke):
                                    header = b"\x01\x08\x00\x07\x01\x09" if routed else b"\x01\x00"
                                    npdu = header + bytes([2 if segmented else 0, 3, invoke, service]) + request
                                    wire = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
                                    await loop.sock_sendto(sock, wire, (ip, int(port)))
                                    reply, _ = await asyncio.wait_for(loop.sock_recvfrom(sock, 4096), 2)
                                    self.assertEqual(reply[:2], b"\x81\x0a")
                                    self.assertEqual(int.from_bytes(reply[2:4], "big"), len(reply))
                                    if routed:
                                        self.assertEqual(reply[4:11], b"\x01\x20\x00\x07\x01\x09\xff")
                                    async with asyncio.timeout(2):
                                        while (await server.request_admission_counters())["confirmed_active"]:
                                            await asyncio.sleep(0)
                                    with self.assertRaises(BlockingIOError):
                                        sock.recvfrom(4096)
                                    return reply[11:] if routed else reply[6:]

                                choice = b"\x1e" if record else b"\x0e"
                                close = b"\x1f" if record else b"\x0f"
                                # File:1 and signed -1 append. Payload caps exclude these fields.
                                data = (unsigned(len(payload)) + b"".join(map(octets, payload))
                                        if record else octets(b"".join(payload)))
                                request = b"\xc4\x02\x80\x00\x01" + choice + b"\x31\xff" + data + close
                                actual = await exchange(7, request, 42)
                                self.assertEqual(actual, b"\x71\x2a\x09" if refused else
                                                 b"\x30\x2a\x07" + (b"\x19\x01" if record else b"\x09\x08"))
                                expected = ([b"sentinel"] if refused else [b"sentinel"] + payload)
                                count = 1 if refused else len(expected)
                                read_request = b"\xc4\x02\x80\x00\x01" + choice + b"\x31\x00" + unsigned(count if record else 100) + close
                                read_ack = (unsigned(count) + b"".join(map(octets, expected)) if record
                                            else octets(b"".join(expected)))
                                self.assertEqual(await exchange(6, read_request, 43),
                                                 b"\x30\x2b\x06\x11" + choice + b"\x31\x00" + read_ack + close)
                            finally:
                                await server.stop()
                                sock.close()
