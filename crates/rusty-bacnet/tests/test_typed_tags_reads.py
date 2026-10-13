"""Native remote Tags reads against independently encoded UDP replies."""
import asyncio
from contextlib import asynccontextmanager
import copy
import pickle
import socket
import unittest

from rusty_bacnet import (BACnetClient, BipEndpoint, ObjectIdentifier, ObjectType,
                         PropertyIdentifier, PropertyValue, ErrorClass, ErrorCode)

OID = ObjectIdentifier(ObjectType.COLOR, 1)
TAGS = PropertyIdentifier.TAGS
EXHAUST = bytes.fromhex("0d080065786861757374")
FLOOR = bytes.fromhex("0d0600666c6f6f722103")
IDENTITY = bytes.fromhex("0c0fc000011a01e6")
DATE = bytes.fromhex("0a0064a47e0a0804")
TIME = bytes.fromhex("0a0074b40c223807")


@asynccontextmanager
async def reader(endpoint):
    if endpoint:
        owner = BipEndpoint(device_instance=9101, interface="127.0.0.1",
                            broadcast_address="127.0.0.1", port=0)
        await owner.start()
        try:
            yield await owner.client()
        finally:
            await owner.close()
    else:
        async with BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000) as client:
            yield client


class TypedTagsReadTests(unittest.IsolatedAsyncioTestCase):
    async def read(self, value, index=None, *, endpoint=False, rpm=False, batch=False, echo=False):
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            async with reader(endpoint) as client:
                spec = [(OID, [(TAGS, index), (PropertyIdentifier.DESCRIPTION, None)])]
                if batch:
                    await client.add_device(9102, address)
                    call = (client.read_property_multiple_from_devices([(9102, spec)]) if rpm
                            else client.read_property_from_devices([(9102, OID, TAGS, index)]))
                else:
                    call = (client.read_property_multiple(address, spec) if rpm
                            else client.read_property(address, OID, TAGS, index))
                operation = asyncio.ensure_future(call)
                try:
                    packet, sender = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 2)
                    identity = IDENTITY + (bytes((0x29, index)) if index is not None else b"")
                    self.assertEqual(packet[:2], b"\x81\x0a")
                    self.assertEqual(packet[4:6], b"\x01\x04")
                    service = 14 if rpm else 12
                    self.assertEqual(packet[9], service)
                    if rpm:
                        expected = IDENTITY[:5] + b"\x1e\x0a\x01\xe6"
                        expected += bytes((0x19, index)) if index is not None else b""
                        expected += b"\x09\x1c\x1f"
                        self.assertEqual(packet[10:], expected)
                        payload = IDENTITY[:5] + b"\x1e\x2a\x01\xe6"
                        payload += bytes((0x39, index)) if index is not None else b""
                        payload += b"\x4e" + value + b"\x4f"
                        payload += b"\x29\x1c\x5e\x91\x02\x91\x20\x5f\x1f"
                    else:
                        self.assertEqual(packet[10:], identity)
                        payload = identity + b"\x3e" + value + b"\x3f"
                    await self.reply(peer, sender, bytes((0x30, packet[8], service)) + payload)
                    result = await asyncio.wait_for(operation, 2)
                    if batch:
                        self.assertEqual(result[0]["request_index"], 0)
                        self.assertEqual(result[0]["device_instance"], 9102)
                        self.assertIsNone(result[0]["error"])
                        result = result[0]["results" if rpm else "value"]
                    if rpm:
                        self.assertEqual(result[0]["object_id"], OID)
                        rows = result[0]["results"]
                        self.assertEqual(len(rows), 2)
                        self.assertEqual(rows[0]["property_id"], TAGS)
                        self.assertEqual(rows[0]["array_index"], index)
                        self.assertIsNone(rows[0]["error"])
                        self.assertIsNone(rows[1]["value"])
                        self.assertEqual(rows[1]["error"], (ErrorClass.PROPERTY, ErrorCode.UNKNOWN_PROPERTY))
                        result = rows[0]["value"]
                    if echo:
                        operation = asyncio.ensure_future(client.write_property(address, OID, TAGS, result, array_index=index))
                        written, sender = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 2)
                        self.assertEqual(written[9], 15)
                        self.assertEqual(written[10:], identity + b"\x3e" + value + b"\x3f")
                        await self.reply(peer, sender, bytes((0x20, written[8], 15)))
                        await asyncio.wait_for(operation, 2)
                    return result
                finally:
                    operation.cancel()
                    await asyncio.gather(operation, return_exceptions=True)

    async def reply(self, peer, sender, apdu):
        body = b"\x01\x00" + apdu
        await asyncio.get_running_loop().sock_sendto(
            peer, b"\x81\x0a" + (4 + len(body)).to_bytes(2, "big") + body, sender)

    async def test_whole_tags_reads_as_typed_name_value_list(self):
        for endpoint, rpm, batch in [(False, False, False), (False, True, False),
                                     (True, False, False), (True, True, False),
                                     (False, False, True), (False, True, True)]:
            with self.subTest(endpoint=endpoint, rpm=rpm, batch=batch):
                value = await self.read(EXHAUST + FLOOR, endpoint=endpoint, rpm=rpm, batch=batch)
                self.assertEqual(value.tag, "list")
                self.assertEqual(value.value[0], {"name": "exhaust", "value": None})
                self.assertEqual(value.value[1]["name"], "floor")
                self.assertEqual(value.value[1]["value"].tag, "unsigned")
                self.assertEqual(value.value[1]["value"].value, 3)

    async def test_elements_preserve_none_null_primitives_and_exact_write_back(self):
        vectors = [(EXHAUST, "exhaust", None, None), (bytes.fromhex("0a006100"), "a", "null", None),
                   (FLOOR, "floor", "unsigned", 3), (bytes.fromhex("0a0073720078"), "s", "character_string", "x"),
                   (bytes.fromhex("0a0072443fc00000"), "r", "real", 1.5),
                   (DATE, "d", "date", (2026, 10, 8, 4)), (TIME, "t", "time", (12, 34, 56, 7))]
        elements = []
        for wire, name, tag, expected in vectors:
            element = await self.read(wire, 1, echo=True)
            self.assertEqual(element.tag, "name_value")
            self.assertEqual(element.value["name"], name)
            if tag is None:
                self.assertIsNone(element.value["value"])
            else:
                self.assertIsInstance(element.value["value"], PropertyValue)
                self.assertEqual(element.value["value"].tag, tag)
                self.assertEqual(element.value["value"].value, expected)
            self.assertEqual(pickle.loads(pickle.dumps(element)), element)
            self.assertEqual(copy.deepcopy(element).tag, "name_value")
            elements.append(element)
        whole = await self.read(b"".join(v[0] for v in vectors), echo=True)
        self.assertEqual(PropertyValue.list(elements), whole)
        self.assertEqual(pickle.loads(pickle.dumps(whole)).value, whole.value)

    async def test_empty_count_and_indexed_shapes_agree_on_all_read_routes(self):
        for endpoint, rpm, batch in [(False, False, False), (False, True, False),
                                     (True, False, False), (True, True, False),
                                     (False, False, True), (False, True, True)]:
            for raw, index, tag, expected in [(b"", None, "list", []), (b"\x21\x00", 0, "unsigned", 0),
                                             (b"\x21\x02", 0, "unsigned", 2),
                                             (EXHAUST, 1, "name_value", {"name": "exhaust", "value": None})]:
                with self.subTest(endpoint=endpoint, rpm=rpm, batch=batch, index=index, raw=raw):
                    value = await self.read(raw, index, endpoint=endpoint, rpm=rpm, batch=batch)
                    self.assertEqual((value.tag, value.value), (tag, expected))

    async def test_failed_typed_decode_keeps_complete_raw_value(self):
        for raw, index in [(EXHAUST + b"\x0a\x00\xff", None), (b"\x0a\x03a", None),
                           (b"\x0a\x00a\xd0", None), (DATE + TIME[3:], None),
                           (EXHAUST + FLOOR, 1), (EXHAUST + b"\x1a\x00a", None)]:
            for rpm in (False, True):
                with self.subTest(raw=raw, index=index, rpm=rpm):
                    value = await self.read(raw, index, rpm=rpm)
                    self.assertEqual(value.tag, "application_data")
                    self.assertEqual(value.value, raw)
