"""Installed-wheel event telemetry: BACnetServer.event_notification_counters() (#1142, #1160, #1196, #1225, #1259, #1346, #2537)."""
import ast
import asyncio
import importlib.util
from pathlib import Path
import sys
import unittest

from rusty_bacnet import (
    BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue,
)

FIELDS = frozenset({
    "notification_class_missing",
    "recipient_list_unavailable",
    "recipient_list_invalid",
    "recipient_list_too_long",
    "device_recipient_unbound",
    "recipient_unroutable",
    "confirmed_broadcast_recipient",
    "confirmed_waited_for_invoke_id",
    "confirmed_rejected",
    "confirmed_unanswered",
    "unconfirmed_send_failed",
    "apdu_too_large",
    "received_not_forwarded",
    "forwarding_cap_dropped",
    "received_not_logged",
    "lane_full",
    "lanes_entries_full",
    "lanes_full",
    "cov_wait_expired",
})


def destination(recipient: bytes, confirmed: bool) -> bytes:
    """One framed Recipient_List entry, active all week and all day for every transition."""
    return (b"\x82\x01\xfe"            # valid-days: every day
            + b"\xb4\x00\x00\x00\x00"  # from-time 00:00:00.00
            + b"\xb4\x17\x3b\x3b\x63"  # to-time 23:59:59.99
            + recipient
            + b"\x21\x01"              # process-identifier 1
            + (b"\x11" if confirmed else b"\x10")
            + b"\x82\x05\xe0")         # transitions: all three


# device [0]: Device 99, which the server never heard from.
UNBOUND_DEVICE = b"\x0c\x02\x00\x00\x63"
# address [1]: a MAC on the global broadcast network 65535.
MAC_ON_NETWORK_65535 = b"\x1e\x22\xff\xff\x65\x06\x7f\x00\x00\x01\xba\xc1\x1f"
# address [1]: network 0 with an empty MAC, the local broadcast.
LOCAL_BROADCAST = b"\x1e\x21\x00\x60\x1f"
# address [1]: network 0, 127.0.0.1 port 0, which macOS and Linux refuse to send
# to. Windows accepts the send, so the test that relies on it skips there.
UNSENDABLE_UNICAST = b"\x1e\x21\x00\x65\x06\x7f\x00\x00\x01\x00\x00\x1f"
# address [1]: network 0, 127.0.0.1 port 1, which the OS accepts the send to.
SENDABLE_UNICAST = b"\x1e\x21\x00\x65\x06\x7f\x00\x00\x01\x00\x01\x1f"


async def enable_high_limit(server, target):
    for prop, value in [
        (PropertyIdentifier.HIGH_LIMIT, PropertyValue.real(1)),
        # high-limit-enable only
        (PropertyIdentifier.LIMIT_ENABLE, PropertyValue.bit_string(6, b"\x40")),
        # every transition enabled
        (PropertyIdentifier.EVENT_ENABLE, PropertyValue.bit_string(5, b"\xe0")),
    ]:
        await server.write_property_local(target, prop, value, source_object=None)


class EventNotificationCountersTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_transition_without_its_notification_class_is_counted(self):
        server = BACnetServer(503_807, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1")
        # Notification_Class defaults to 0, and no class 0 is added.
        server.add_analog_input(1, "Event counters")
        target = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        with self.assertRaisesRegex(RuntimeError, "^server not started$"):
            await server.event_notification_counters()
        await server.start()
        try:
            expected = dict.fromkeys(FIELDS, 0)
            self.assertEqual(await server.event_notification_counters(), expected)
            await enable_high_limit(server, target)
            self.assertEqual(await server.event_notification_counters(), expected)

            # NORMAL -> HIGH_LIMIT finds no Notification Class 0.
            await server.set_present_value_local(target, PropertyValue.real(2))
            expected["notification_class_missing"] += 1
            self.assertEqual(await server.event_notification_counters(), expected)
        finally:
            await server.stop()
        with self.assertRaisesRegex(RuntimeError, "^server not started$"):
            await server.event_notification_counters()

    async def test_each_route_skip_is_counted_once(self):
        server = BACnetServer(503_808, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1")
        server.add_analog_input(1, "Route skips")
        server.add_notification_class(0, "Route skip class")
        target = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        await server.start()
        try:
            await server.write_property_local(
                ObjectIdentifier(ObjectType.NOTIFICATION_CLASS, 0),
                PropertyIdentifier.RECIPIENT_LIST,
                PropertyValue.application_data(
                    destination(UNBOUND_DEVICE, False)
                    + destination(MAC_ON_NETWORK_65535, False)
                    + destination(LOCAL_BROADCAST, True)),
                source_object=None)
            await enable_high_limit(server, target)
            expected = dict.fromkeys(FIELDS, 0)
            self.assertEqual(await server.event_notification_counters(), expected)

            # NORMAL -> HIGH_LIMIT matches all three destinations and skips each.
            # Device 99 is looked for first with a Who-Is (#1368) and never
            # answers, and the write waits for it, up to the APDU timeout (3 s),
            # before its COV fanout and its return (#2537). The two that can
            # never be routed count at once, while the write still waits.
            loop = asyncio.get_running_loop()
            began = loop.time()
            write = asyncio.ensure_future(
                server.set_present_value_local(target, PropertyValue.real(2)))
            expected.update(recipient_unroutable=1, confirmed_broadcast_recipient=1)
            deadline = began + 2
            while (await server.event_notification_counters() != expected
                   and loop.time() < deadline):
                await asyncio.sleep(0.05)
            self.assertFalse(write.done(), "the write waits for Device 99")
            self.assertEqual(await server.event_notification_counters(), expected)
            await write
            self.assertGreaterEqual(loop.time() - began, 2.5)

            # Device 99 counts once the APDU timeout has passed. Whether the
            # write's wait ran out first (cov_wait_expired) isn't asserted here;
            # the Rust lane tests pin both orders.
            expected.update(device_recipient_unbound=1)
            del expected["cov_wait_expired"]

            async def counted():
                counters = await server.event_notification_counters()
                del counters["cov_wait_expired"]
                return counters

            deadline = loop.time() + 10
            while await counted() != expected and loop.time() < deadline:
                await asyncio.sleep(0.05)
            self.assertEqual(await counted(), expected)
        finally:
            await server.stop()

    @unittest.skipIf(sys.platform == "win32",
                     "Windows accepts a UDP send to port 0; the Rust event routing"
                     " tests cover the counter with a failing transport")
    async def test_a_failed_unconfirmed_send_is_counted_once_per_destination(self):
        server = BACnetServer(503_809, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1")
        server.add_analog_input(1, "Send failures")
        server.add_notification_class(0, "Send failure class")
        target = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        await server.start()
        try:
            await server.write_property_local(
                ObjectIdentifier(ObjectType.NOTIFICATION_CLASS, 0),
                PropertyIdentifier.RECIPIENT_LIST,
                PropertyValue.application_data(
                    destination(UNSENDABLE_UNICAST, False)
                    + destination(SENDABLE_UNICAST, False)
                    + destination(UNSENDABLE_UNICAST, False)),
                source_object=None)
            await enable_high_limit(server, target)
            expected = dict.fromkeys(FIELDS, 0)
            self.assertEqual(await server.event_notification_counters(), expected)

            # Two of the three destinations fail to send; the middle one is served.
            await server.set_present_value_local(target, PropertyValue.real(2))
            expected["unconfirmed_send_failed"] = 2
            self.assertEqual(await server.event_notification_counters(), expected)
        finally:
            await server.stop()

    def test_installed_stub_shape(self):
        spec = importlib.util.find_spec("rusty_bacnet")
        assert spec is not None and spec.origin is not None
        origin = Path(spec.origin)
        candidates = [origin.with_suffix(".pyi"), origin.parent / "rusty_bacnet.pyi"]
        stub = next(path for path in candidates if path.exists())
        classes = {node.name: node for node in ast.parse(stub.read_text()).body
                   if isinstance(node, ast.ClassDef)}
        fields = {node.target.id: ast.unparse(node.annotation)
                  for node in classes["EventNotificationCounters"].body
                  if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name)}
        self.assertEqual(fields, dict.fromkeys(FIELDS, "int"))
        method = next(node for node in classes["BACnetServer"].body
                      if isinstance(node, ast.FunctionDef)
                      and node.name == "event_notification_counters")
        assert method.returns is not None
        self.assertEqual(ast.unparse(method.returns), "Awaitable[EventNotificationCounters]")


if __name__ == "__main__":
    unittest.main()
