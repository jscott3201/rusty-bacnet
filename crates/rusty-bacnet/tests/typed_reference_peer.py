"""Independent BACnet/IP replies for typed-reference consumer tests."""
import asyncio
from contextlib import asynccontextmanager
from dataclasses import dataclass
import socket

from rusty_bacnet import (BACnetClient, BipEndpoint, ObjectIdentifier,
                         PropertyIdentifier, ErrorClass, ErrorCode)


@dataclass(frozen=True)
class Target:
    oid: ObjectIdentifier
    prop: PropertyIdentifier
    object_word: int
    property_number: int


def unsigned_context(tag, number):
    size = max(1, (number.bit_length() + 7) // 8)
    return bytes(((tag << 4) | 8 | size,)) + number.to_bytes(size, "big")


@asynccontextmanager
async def reader(endpoint):
    if endpoint:
        owner = BipEndpoint(device_instance=9497, interface="127.0.0.1",
                            broadcast_address="127.0.0.1", port=0)
        await owner.start()
        try:
            yield await owner.client()
        finally:
            await owner.close()
    else:
        async with BACnetClient(interface="127.0.0.1", port=0,
                                apdu_timeout_ms=2000) as client:
            yield client


class ReferencePeer:
    def __init__(self, test, target):
        self.test = test
        self.target = target
        test.assertEqual(target.prop.to_raw(), target.property_number)

    async def read(self, octets, index=None, *, route="rp", echo=False, extra=()):
        """RPM returns successful rows plus an embedded DESCRIPTION error.

        `extra` adds repeated-property RPM rows for mixed fallback observations.
        Echo uses the ordinary client WP and proves octet preservation only.
        """
        test, target = self.test, self.target
        endpoint, rpm = route.startswith("endpoint_"), route.endswith("rpm")
        batch, singular = route.startswith("batch_"), route.startswith("device_")
        assert not echo or not endpoint
        assert not extra or rpm
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            async with reader(endpoint) as client:
                if batch or singular:
                    await client.add_device(9498, address)
                references = [(target.prop, index)] * (1 + len(extra))
                references.append((PropertyIdentifier.DESCRIPTION, None))
                specs = [(target.oid, references)]
                if batch:
                    call = (client.read_property_multiple_from_devices([(9498, specs)]) if rpm
                            else client.read_property_from_devices([(9498, target.oid, target.prop, index)]))
                elif singular:
                    call = (client.read_property_multiple_from_device(9498, specs) if rpm
                            else client.read_property_from_device(9498, target.oid, target.prop, index))
                else:
                    call = (client.read_property_multiple(address, specs) if rpm
                            else client.read_property(address, target.oid, target.prop, index))
                operation = asyncio.ensure_future(call)
                try:
                    packet, sender = await asyncio.wait_for(loop.sock_recvfrom(peer, 4096), 2)
                    test.assertEqual(packet[:2], b"\x81\x0a")
                    test.assertEqual(int.from_bytes(packet[2:4], "big"), len(packet))
                    test.assertEqual(packet[4:6], b"\x01\x04")
                    service = 14 if rpm else 12
                    test.assertEqual(packet[9], service)
                    oid = b"\x0c" + target.object_word.to_bytes(4, "big")
                    identity = oid + unsigned_context(1, target.property_number)
                    if index is not None:
                        identity += unsigned_context(2, index)
                    if rpm:
                        requested = unsigned_context(0, target.property_number)
                        if index is not None:
                            requested += unsigned_context(1, index)
                        test.assertEqual(packet[10:], oid + b"\x1e" + requested * (1 + len(extra)) + b"\x09\x1c\x1f")
                        payload = oid + b"\x1e"
                        for raw in (octets, *extra):
                            payload += unsigned_context(2, target.property_number)
                            if index is not None:
                                payload += unsigned_context(3, index)
                            payload += b"\x4e" + raw + b"\x4f"
                        payload += b"\x29\x1c\x5e\x91\x02\x91\x20\x5f\x1f"
                    else:
                        test.assertEqual(packet[10:], identity)
                        payload = identity + b"\x3e" + octets + b"\x3f"
                    await self.reply(peer, sender, bytes((0x30, packet[8], service)) + payload)
                    result = await asyncio.wait_for(operation, 2)
                    if batch:
                        # Completion order is deliberately not an API ordering promise.
                        by_index = {row["request_index"]: row for row in result}
                        test.assertEqual(set(by_index), {0})
                        result = by_index[0]
                        test.assertEqual(result["device_instance"], 9498)
                        test.assertIsNone(result["error"])
                        result = result["results" if rpm else "value"]
                    if rpm:
                        test.assertEqual(len(result), 1)
                        test.assertEqual(result[0]["object_id"], target.oid)
                        rows = result[0]["results"]
                        test.assertEqual(len(rows), 2 + len(extra))
                        for row in rows[:-1]:
                            test.assertEqual(row["property_id"], target.prop)
                            test.assertEqual(row["array_index"], index)
                            test.assertIsNone(row["error"])
                        test.assertIsNone(rows[-1]["value"])
                        test.assertEqual(rows[-1]["property_id"], PropertyIdentifier.DESCRIPTION)
                        test.assertEqual(rows[-1]["error"], (ErrorClass.PROPERTY, ErrorCode.UNKNOWN_PROPERTY))
                        result = [row["value"] for row in rows[:-1]] if extra else rows[0]["value"]
                    if echo:
                        operation = asyncio.ensure_future(client.write_property(
                            address, target.oid, target.prop, result, array_index=index))
                        written, sender = await asyncio.wait_for(loop.sock_recvfrom(peer, 4096), 2)
                        test.assertEqual(written[9], 15)
                        test.assertEqual(written[10:], identity + b"\x3e" + octets + b"\x3f")
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
