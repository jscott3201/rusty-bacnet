"""Installed native GetEventInformation limits with independent UDP framing."""
import asyncio
import inspect
import socket
import struct
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue


class EventInformationConstructorTests(unittest.TestCase):
    def test_positive_keyword_only_defaults_and_installed_stub(self):
        signature = inspect.signature(BACnetServer)
        for name, default in [("event_information_max_objects", 4096),
                              ("event_information_max_returned_summaries", 256),
                              ("event_information_max_service_ack_bytes", 16384)]:
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


def summary_oids(ack):
    """Read seven tagged summary fields, including constructed timestamp choices."""
    assert ack[0] == 0x0e and ack[-3:-1] == b"\x0f\x19"
    offset = 1
    result = []
    while ack[offset] != 0x0f:
        assert ack[offset] == 0x0c
        result.append(int.from_bytes(ack[offset + 1:offset + 5], "big"))
        for field in range(7):
            tag = ack[offset]
            assert tag >> 4 == field and tag & 8
            offset += 1
            if tag & 7 == 6:
                depth = 1
                while depth:
                    tag = ack[offset]
                    offset += 1
                    if tag & 8 and tag & 7 == 6:
                        depth += 1
                    elif tag & 8 and tag & 7 == 7:
                        depth -= 1
                    else:
                        assert tag & 7 <= 4
                        offset += tag & 7
            else:
                assert tag & 7 <= 4
                offset += tag & 7
    assert offset == len(ack) - 3 and ack[-1] in (0, 1)
    return result, bool(ack[-1])


class EventInformationNativeTests(unittest.IsolatedAsyncioTestCase):
    async def exchange(self, server, sock, service=b"", routed=False, segmented=False, invoke=77):
        ip, port = (await server.local_address()).rsplit(":", 1)
        header = b"\x01\x08\x00\x07\x01\x09" if routed else b"\x01\x00"
        npdu = header + bytes([2 if segmented else 0, 5, invoke, 29]) + service
        wire = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
        loop = asyncio.get_running_loop()
        await loop.sock_sendto(sock, wire, (ip, int(port)))
        reply, _ = await asyncio.wait_for(loop.sock_recvfrom(sock, 4096), 2)
        self.assertEqual(reply[:2], b"\x81\x0a")
        self.assertEqual(int.from_bytes(reply[2:4], "big"), len(reply))
        if routed:
            self.assertEqual(reply[4:11], b"\x01\x20\x00\x07\x01\x09\xff")
        async with asyncio.timeout(2):
            while (await server.request_admission_counters())["confirmed_active"]:
                await asyncio.sleep(0)
        return reply[11:] if routed else reply[6:]

    async def test_total_object_abort_tiny_empty_bytes_and_decode_precedence(self):
        for policy, expected in [({"event_information_max_objects": 1}, 9),
                                 ({"event_information_max_service_ack_bytes": 3}, 1),
                                 ({"event_information_max_service_ack_bytes": 4}, None),
                                 ({}, None)]:
            server = BACnetServer(123, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", **policy)
            server.add_notification_class(0, "NC")
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.bind(("127.0.0.1", 0))
            sock.setblocking(False)
            try:
                await server.start()
                for routed in [False, True]:
                    for segmented in [False, True]:
                        for service in [b"", b"\x0c\xff\xff\xff\xff"]:
                            apdu = await self.exchange(server, sock, service, routed, segmented)
                            self.assertEqual(apdu, bytes([0x71, 77, expected]) if expected else
                                             b"\x30\x4d\x1d\x0e\x0f\x19\x00")
                        invalid = await self.exchange(server, sock, b"\xff", routed, segmented, 78)
                        self.assertNotEqual(invalid[0], 0x71)
                        self.assertIn(invalid[0] >> 4, [5, 6])
            finally:
                await server.stop()
                sock.close()

    async def test_native_active_event_pages_without_gaps(self):
        server = BACnetServer(123, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1",
                              event_information_max_returned_summaries=2)
        server.add_notification_class(0, "NC")
        for instance in range(1, 6):
            server.add_analog_input(instance, f"AI-{instance}")
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        sock.setblocking(False)
        try:
            await server.start()
            for instance in range(1, 6):
                oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, instance)
                await server.write_property_local(oid, PropertyIdentifier.HIGH_LIMIT, PropertyValue.real(1))
                await server.write_property_local(oid, PropertyIdentifier.LIMIT_ENABLE, PropertyValue.bit_string(6, b"\x40"))
                await server.set_present_value_local(oid, PropertyValue.real(2))
            for routed in [False, True]:
                for segmented in [False, True]:
                    cursor = b""
                    seen = []
                    for page in range(3):
                        apdu = await self.exchange(server, sock, cursor, routed, segmented, 80 + page)
                        self.assertEqual(apdu[:3], bytes([0x30, 80 + page, 29]))
                        oids, more = summary_oids(apdu[3:])
                        self.assertEqual(len(oids), 2 if page < 2 else 1)
                        self.assertEqual(more, page < 2)
                        seen.extend(oids)
                        cursor = b"\x0c" + oids[-1].to_bytes(4, "big")
                    self.assertEqual(seen, [1, 2, 3, 4, 5])
        finally:
            await server.stop()
            sock.close()
