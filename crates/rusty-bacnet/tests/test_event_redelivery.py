"""A cancelled COV read loses no notification (#1858)."""
import asyncio
import time
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier,
    PropertyValue,
)


def present_value(notification):
    return next(item["value"] for item in notification.values
                if item["property_id"] == PropertyIdentifier.PRESENT_VALUE)


class CovRedeliveryTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_cancelled_wait_for_loses_no_notification(self):
        server = BACnetServer(503_858, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1")
        server.add_analog_input(1, "Redelivered COV", present_value=10.0)
        target = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        await server.start()

        async def stop_server():
            await server.stop()
        self.addAsyncCleanup(stop_server)
        async with BACnetClient(interface="127.0.0.1", port=0,
                                broadcast_address="127.0.0.1", apdu_timeout_ms=2000) as client:
            iterator = await client.cov_notifications()
            address = await server.local_address()
            await client.subscribe_cov(address, 1, target, confirmed=False, lifetime=60)
            initial = await asyncio.wait_for(iterator.__anext__(), 5)
            self.assertEqual(present_value(initial), PropertyValue.real(10.0))

            # Timed-out reads with nothing on the way give up cleanly.
            for _ in range(3):
                with self.assertRaises(asyncio.TimeoutError):
                    await asyncio.wait_for(iterator.__anext__(), 0.05)

            # A read given up while its notification is being delivered: the
            # change is made and the loop blocked until that delivery is
            # queued, then the read is cancelled, as wait_for's timeout does.
            pending = iterator.__anext__()
            change = server.set_present_value_local(target, PropertyValue.real(15.0))
            time.sleep(0.5)
            pending.cancel()
            await change
            again = await asyncio.wait_for(iterator.__anext__(), 5)
            self.assertEqual(present_value(again), PropertyValue.real(15.0))
            self.assertEqual(iterator.missed, 0)

            # And the stream goes on in order.
            await server.set_present_value_local(target, PropertyValue.real(20.0))
            after = await asyncio.wait_for(iterator.__anext__(), 5)
            self.assertEqual(present_value(after), PropertyValue.real(20.0))
        with self.assertRaises(StopAsyncIteration):
            await asyncio.wait_for(iterator.__anext__(), 5)


if __name__ == "__main__":
    unittest.main()
