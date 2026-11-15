"""Who-Is and Who-Has device ranges take both limits or neither (#1483).

One limit alone used to go out as a request for every device. Each discovery
call now raises ValueError for it, for a low limit above the high one, and
for a limit past the highest instance, 4194303, before the client is even
started, and sends nothing; both limits still go out on the wire. A
confirmed call to a broadcast or group address raises too (#1479).
"""
import asyncio
import socket
import unittest

from rusty_bacnet import BACnetClient, BacnetError, ObjectIdentifier, ObjectType, PropertyIdentifier

OID = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)


def calls(client, address="127.0.0.1:47808"):
    """Each discovery call, taking the limits as keyword arguments."""
    return {
        "who_is": lambda **limits: client.who_is(**limits),
        "discover": lambda **limits: client.discover(wait=0, **limits),
        "who_has_by_id": lambda **limits: client.who_has_by_id(OID, **limits),
        "who_has_by_name": lambda **limits: client.who_has_by_name("AV-1", **limits),
        "who_is directed": lambda **limits: client.who_is(scope=address, **limits),
    }


REFUSED = [
    ({"low": 10}, "low limit 10 was given without a high limit"),
    ({"high": 20}, "high limit 20 was given without a low limit"),
    ({"low": 20, "high": 10}, "low limit 20 is above its high limit 10"),
    ({"low": 0, "high": 4194304}, "high limit 4194304 is above the highest instance, 4194303"),
]


class DeviceRangeTests(unittest.IsolatedAsyncioTestCase):
    async def test_one_limit_or_an_empty_range_raises_before_start(self):
        client = BACnetClient(interface="127.0.0.1", port=0)
        for name, call in calls(client).items():
            for limits, message in REFUSED:
                with self.subTest(call=name, limits=limits):
                    with self.assertRaisesRegex(ValueError, message):
                        call(**limits)

    async def test_only_a_whole_range_reaches_the_wire(self):
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                for limits, _ in REFUSED:
                    with self.assertRaises(ValueError):
                        client.who_is(scope=address, **limits)
                with self.assertRaises(TimeoutError):
                    await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 0.1)

                await client.who_is(scope=address, low=10, high=20)
                frame, _ = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 3)
                # Who-Is with [0] low limit 10 and [1] high limit 20.
                self.assertTrue(
                    frame.endswith(bytes([0x10, 0x08, 0x09, 0x0A, 0x19, 0x14])), frame.hex()
                )

                await client.who_is(scope=address)
                frame, _ = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 3)
                self.assertTrue(frame.endswith(bytes([0x10, 0x08])), frame.hex())

                # The highest instance is a limit like any other.
                await client.who_is(scope=address, low=4194303, high=4194303)
                frame, _ = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 3)
                self.assertTrue(
                    frame.endswith(bytes([0x0B, 0x3F, 0xFF, 0xFF, 0x1B, 0x3F, 0xFF, 0xFF])),
                    frame.hex(),
                )

    async def test_a_confirmed_call_to_a_group_address_raises(self):
        async with BACnetClient(interface="127.0.0.1", port=0) as client:
            for address in ("255.255.255.255:47808", "255.255.255.255:47809", "224.0.0.1:47808"):
                with self.subTest(address=address):
                    with self.assertRaisesRegex(BacnetError, "not to a broadcast or group address"):
                        await asyncio.wait_for(
                            client.read_property(address, OID, PropertyIdentifier.PRESENT_VALUE), 3
                        )


if __name__ == "__main__":
    unittest.main()
