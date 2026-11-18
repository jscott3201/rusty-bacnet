"""COV-multiple notifications reach `cov_notifications()` (#2654).

A SubscribeCOVPropertyMultiple subscription to a real server notifies the
client with ConfirmedCOVNotificationMultiple or
UnconfirmedCOVNotificationMultiple. Each arrives on the same iterator as the
single-object notifications, as a `CovNotificationMultiple` whose `kind` is
"multiple", in the order they came; a confirmed one is acknowledged, so the
server sends it once. The routed case, through a real `BACnetRouter`, is in
test_routed_cov.py.
"""
import asyncio
import contextlib
import unittest

import rusty_bacnet as rb
from bvll_peer import mac
from test_endpoint_routed import finish

O = rb.ObjectType
AI_1 = rb.ObjectIdentifier(O.ANALOG_INPUT, 1)
AI_2 = rb.ObjectIdentifier(O.ANALOG_INPUT, 2)
PV = rb.PropertyIdentifier.PRESENT_VALUE
SF = rb.PropertyIdentifier.STATUS_FLAGS
DEVICE = 2654


async def collect(client):
    """A queue each COV notification the client hears goes to, and the
    task that fills it."""
    queue = asyncio.Queue()
    iterator = await client.cov_notifications()

    async def fill():
        async for received in iterator:
            queue.put_nowait(received)

    return queue, asyncio.create_task(fill())


async def stop(task):
    task.cancel()
    with contextlib.suppress(asyncio.CancelledError):
        await task


def values_of(notification, object_id, property_id):
    """The values `notification` reports for one property of one object."""
    return [value for item in notification.objects
            if item["monitored_object_identifier"] == object_id
            for value in item["values"] if value["property_id"] == property_id]


class CovMultipleNotificationTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.server = rb.BACnetServer(DEVICE, interface="127.0.0.1", port=0,
                                      broadcast_address="127.0.0.1")
        self.server.add_analog_input(1, "Supply air", present_value=10.0)
        self.server.add_analog_input(2, "Return air", present_value=20.0)
        await asyncio.wait_for(self.server.start(), 10)
        self.addAsyncCleanup(finish, self.server.stop)
        self.address = await self.server.local_address()

    async def notified(self, queue, process, confirmed):
        """The next notification: a COV-multiple one from the server for
        `process`, by the service `confirmed` names."""
        heard = await asyncio.wait_for(queue.get(), 5)
        self.assertIsInstance(heard, rb.CovNotificationMultiple)
        self.assertEqual(heard.kind, "multiple")
        self.assertEqual(heard.subscriber_process_identifier, process)
        self.assertEqual(heard.initiating_device_identifier,
                         rb.ObjectIdentifier(O.DEVICE, DEVICE))
        self.assertEqual(heard.delivery, "confirmed" if confirmed else "unconfirmed")
        self.assertEqual(heard.source_mac, mac(self.address))
        self.assertEqual((heard.source_network, heard.source_address), (None, None))
        self.assertGreater(heard.time_remaining, 0)
        self.assertIn("CovNotificationMultiple(device=2654", repr(heard))
        return heard

    async def subscription(self, client, queue, process, confirmed, timestamped):
        """Subscribe `process` to both inputs' Present_Value and AI 1's
        Status_Flags, see the first report and a change, then cancel the
        whole context and see nothing more."""
        specs = [(AI_1, [(PV, None, None, timestamped), (SF, None, None, timestamped)]),
                 (AI_2, [(PV, None, None, timestamped)])]
        await asyncio.wait_for(client.subscribe_cov_property_multiple(
            self.address, process, specs, issue_confirmed_notifications=confirmed,
            max_notification_delay=1, lifetime=60), 5)

        # The first report holds every value subscribed to, as it stands.
        first = await self.notified(queue, process, confirmed)
        self.assertEqual([item["monitored_object_identifier"] for item in first.objects],
                         [AI_1, AI_2])
        self.assertIn(rb.PropertyValue.real(10.0),
                      [value["value"] for value in values_of(first, AI_1, PV)])
        self.assertIn(rb.PropertyValue.real(20.0),
                      [value["value"] for value in values_of(first, AI_2, PV)])
        flags = values_of(first, AI_1, SF)
        self.assertTrue(flags)
        self.assertIsInstance(flags[0]["value"].value, rb.BitString)
        for item in first.objects:
            for value in item["values"]:
                self.assertIsNone(value["array_index"])
                if timestamped:
                    hour, minute, second, hundredths = value["time_of_change"]
                    self.assertLess((hour, minute, second, hundredths), (24, 60, 60, 100))
                else:
                    self.assertIsNone(value["time_of_change"])
        if timestamped:
            (year, month, day, weekday), (hour, _, _, _) = first.timestamp
            self.assertGreaterEqual(year, 2026)
            self.assertTrue(1 <= month <= 12 and 1 <= day <= 31 and 1 <= weekday <= 7)
            self.assertLess(hour, 24)
        else:
            self.assertIsNone(first.timestamp)

        # A change of AI 1 is reported.
        await self.server.set_present_value_local(AI_1, rb.PropertyValue.real(30.0))
        changed = await self.notified(queue, process, confirmed)
        self.assertIn(rb.PropertyValue.real(30.0),
                      [value["value"] for value in values_of(changed, AI_1, PV)])

        # A confirmed report was acknowledged: nothing comes again, past the
        # server's 3 s wait for an answer before it sends one again.
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(queue.get(), 3.5 if confirmed else 0.5)

        # Cancel the whole context: no references and no timing.
        await asyncio.wait_for(client.subscribe_cov_property_multiple(
            self.address, process, [], issue_confirmed_notifications=confirmed), 5)
        await self.server.set_present_value_local(AI_1, rb.PropertyValue.real(10.0))
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(queue.get(), 2)

    async def test_confirmed_and_unconfirmed_with_and_without_timestamps(self):
        async with rb.BACnetClient(interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            queue, task = await collect(client)
            try:
                for process, (confirmed, timestamped) in enumerate(
                        ((True, True), (False, False), (True, False), (False, True)),
                        start=31):
                    with self.subTest(confirmed=confirmed, timestamped=timestamped):
                        await self.subscription(client, queue, process, confirmed, timestamped)
            finally:
                await stop(task)

    async def test_both_kinds_arrive_on_one_iterator(self):
        async with rb.BACnetClient(interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            queue, task = await collect(client)
            try:
                await asyncio.wait_for(client.subscribe_cov(
                    self.address, 41, AI_1, confirmed=False, lifetime=60), 5)
                single = await asyncio.wait_for(queue.get(), 5)
                self.assertIsInstance(single, rb.CovNotification)
                self.assertEqual((single.kind, single.subscriber_process_identifier),
                                 ("object", 41))

                await asyncio.wait_for(client.subscribe_cov_property_multiple(
                    self.address, 42, [(AI_2, [(PV, None, None, False)])],
                    issue_confirmed_notifications=False, max_notification_delay=1,
                    lifetime=60), 5)
                multiple = await self.notified(queue, 42, False)
                self.assertEqual([item["monitored_object_identifier"]
                                  for item in multiple.objects], [AI_2])

                # A change of AI 1 reaches the object subscription alone.
                await self.server.set_present_value_local(AI_1, rb.PropertyValue.real(15.0))
                again = await asyncio.wait_for(queue.get(), 5)
                self.assertEqual((again.kind, again.subscriber_process_identifier),
                                 ("object", 41))
                await asyncio.wait_for(client.unsubscribe_cov(self.address, 41, AI_1), 5)
                await asyncio.wait_for(client.subscribe_cov_property_multiple(
                    self.address, 42, [], issue_confirmed_notifications=False), 5)
            finally:
                await stop(task)

    async def test_a_multiple_refuses_copy_and_pickle(self):
        import copy
        import pickle
        async with rb.BACnetClient(interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1",
                                   apdu_timeout_ms=2000) as client:
            queue, task = await collect(client)
            try:
                await asyncio.wait_for(client.subscribe_cov_property_multiple(
                    self.address, 43, [(AI_1, [(PV, None, None, False)])],
                    issue_confirmed_notifications=False, max_notification_delay=1,
                    lifetime=60), 5)
                heard = await self.notified(queue, 43, False)
                with self.assertRaisesRegex(TypeError, "cannot pickle"):
                    pickle.dumps(heard)
                for operation in (copy.copy, copy.deepcopy):
                    with self.assertRaises(TypeError):
                        operation(heard)
                await asyncio.wait_for(client.subscribe_cov_property_multiple(
                    self.address, 43, [], issue_confirmed_notifications=False), 5)
            finally:
                await stop(task)


if __name__ == "__main__":
    unittest.main()
