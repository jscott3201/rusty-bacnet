"""A local write waiting for its event notifications holds up no other call
on the server (#2537).

AI-1 alarms above 1 to Device 99, which the server never heard from and which
never answers: its notification waits in its lane while Device 99 is looked
for, and the write that made it waits too, up to the APDU timeout (3 s),
before its COV fanout and its return.
"""
import asyncio
import unittest

from rusty_bacnet import (
    BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue,
)
from test_event_notification_counters import UNBOUND_DEVICE, destination, enable_high_limit

TARGET = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
# Far below the 3 s a write waits here.
PROMPTLY = 0.5


class ServerCallsBesideAWriteTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.server = BACnetServer(503_811, interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1")
        self.server.add_analog_input(1, "Alarms to Device 99")
        self.server.add_notification_class(0, "To Device 99")
        await self.server.start()
        await self.server.write_property_local(
            ObjectIdentifier(ObjectType.NOTIFICATION_CLASS, 0),
            PropertyIdentifier.RECIPIENT_LIST,
            PropertyValue.application_data(destination(UNBOUND_DEVICE, False)),
            source_object=None)
        await enable_high_limit(self.server, TARGET)

    async def asyncTearDown(self):
        await self.server.stop()

    def alarm(self):
        """Raise AI-1's alarm: a write that waits for Device 99."""
        return asyncio.ensure_future(
            self.server.set_present_value_local(TARGET, PropertyValue.real(2)))

    async def present_value(self):
        read = self.server.read_property(TARGET, PropertyIdentifier.PRESENT_VALUE)
        return (await asyncio.wait_for(read, PROMPTLY)).value

    async def test_other_calls_go_on_while_a_write_waits(self):
        write = self.alarm()
        await asyncio.sleep(0.1)
        counters = await asyncio.wait_for(
            self.server.event_notification_counters(), PROMPTLY)
        self.assertEqual(counters["device_recipient_unbound"], 0)
        self.assertEqual(await self.present_value(), 2.0)
        self.assertFalse(write.done(), "the write waits for Device 99")
        self.assertIsNone(await write)

    async def test_a_second_write_commits_after_the_first(self):
        first = self.alarm()
        while await self.present_value() != 2.0:
            await asyncio.sleep(0.05)
        # The clear commits behind the alarm and waits in the same lane.
        second = asyncio.ensure_future(
            self.server.set_present_value_local(TARGET, PropertyValue.real(0)))
        self.assertEqual(await asyncio.gather(first, second), [None, None])
        self.assertEqual(await self.present_value(), 0.0)

    async def test_stop_waits_for_a_pending_write_which_succeeds(self):
        loop = asyncio.get_running_loop()
        write = self.alarm()
        await asyncio.sleep(0.1)
        began = loop.time()
        await self.server.stop()
        took = loop.time() - began
        # stop() went on once the write returned, its wait capped at 3 s.
        self.assertTrue(write.done())
        self.assertIsNone(write.result())
        self.assertLess(took, 5)
        with self.assertRaisesRegex(RuntimeError, "^server not started$"):
            await self.server.event_notification_counters()


if __name__ == "__main__":
    unittest.main()
