"""Installed ordinary COV indefinite lifetime and cancellation contract."""
import asyncio
import contextlib
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier,
    PropertyValue,
)


class OrdinaryCovFieldsTests(unittest.IsolatedAsyncioTestCase):
    async def test_none_and_zero_lifetimes_deliver_initial_change_and_cancel(self):
        server = BACnetServer(503_805, interface="127.0.0.1", port=0,
                             broadcast_address="127.0.0.1")
        server.add_analog_input(1, "Indefinite COV", present_value=10.0)
        target = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        await server.start()
        try:
            async with BACnetClient(interface="127.0.0.1", port=0,
                                    broadcast_address="127.0.0.1", apdu_timeout_ms=2000) as client:
                notifications = asyncio.Queue()
                iterator = await client.cov_notifications()

                async def collect():
                    async for notification in iterator:
                        notifications.put_nowait(notification)

                listener = asyncio.create_task(collect())
                try:
                    address = await server.local_address()
                    value = 10.0
                    for process_id, (confirmed, lifetime) in enumerate(
                        ((False, None), (False, 0), (True, None), (True, 0)), start=1
                    ):
                        with self.subTest(confirmed=confirmed, lifetime=lifetime):
                            await client.subscribe_cov(address, process_id, target,
                                                       confirmed=confirmed, lifetime=lifetime)
                            initial = await asyncio.wait_for(notifications.get(), 2)
                            self.assertEqual(initial.subscriber_process_identifier, process_id)
                            self.assertEqual(initial.monitored_object_identifier, target)
                            self.assertEqual(initial.time_remaining, 0)
                            self.assertIn(PropertyValue.real(value), [
                                item["value"] for item in initial.values
                                if item["property_id"] == PropertyIdentifier.PRESENT_VALUE
                            ])
                            value += 5
                            await server.set_present_value_local(target, PropertyValue.real(value))
                            changed = await asyncio.wait_for(notifications.get(), 2)
                            self.assertEqual(changed.subscriber_process_identifier, process_id)
                            self.assertEqual(changed.monitored_object_identifier, target)
                            self.assertEqual(changed.time_remaining, 0)
                            self.assertIn(PropertyValue.real(value), [
                                item["value"] for item in changed.values
                                if item["property_id"] == PropertyIdentifier.PRESENT_VALUE
                            ])
                            await client.unsubscribe_cov(address, process_id, target)
                            value += 5
                            await server.set_present_value_local(target, PropertyValue.real(value))
                            with self.assertRaises(asyncio.TimeoutError):
                                await asyncio.wait_for(notifications.get(), 0.1)
                finally:
                    listener.cancel()
                    with contextlib.suppress(asyncio.CancelledError):
                        await listener
        finally:
            await server.stop()
