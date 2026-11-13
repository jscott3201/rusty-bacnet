"""COV on the Python endpoints (#2490): the keyword-only ``cov`` turns on
SubscribeCOV, SubscribeCOVProperty and SubscribeCOVPropertyMultiple, and
``set_present_value_local`` and ``set_reliability_local`` are the
application's changes their subscribers hear of.

A raw B/IP socket plays the subscriber and the tests assert what crosses the
wire: the replies, the first notification of each subscription and the ones
a local change sends, and, with COV off, the Reject every one of the three
services got before. The BACnetClient's own ``subscribe_cov`` is checked
against the endpoint too.
"""
import asyncio
import contextlib
import inspect
import socket
import struct
import unittest

from rusty_bacnet import (
    BACnetClient, BipEndpoint, MstpEndpoint, ObjectIdentifier, ObjectType,
    PropertyIdentifier, PropertyValue, Reliability, ScEndpoint,
)

AI1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
SUBSCRIBE_COV, SUBSCRIBE_COV_PROPERTY, SUBSCRIBE_COV_PROPERTY_MULTIPLE = 5, 28, 30
UNCONFIRMED_COV_NOTIFICATION, UNCONFIRMED_COV_NOTIFICATION_MULTIPLE = 2, 11

# Each service's parameters for AI-1's Present_Value, unconfirmed, for 60 s,
# under subscriber process 1, 2 or 3 (the first octets after the header).
BODIES = {
    SUBSCRIBE_COV: "09 01 1c 00 00 00 01 29 00 39 3c",
    SUBSCRIBE_COV_PROPERTY: "09 02 1c 00 00 00 01 29 00 39 3c 4e 09 55 4f",
    # A Max_Notification_Delay of 5 s; the reference is untimestamped.
    SUBSCRIBE_COV_PROPERTY_MULTIPLE:
        "09 03 19 00 29 3c 39 05 4e 0c 00 00 00 01 1e 0e 09 55 0f 29 00 1f 4f",
}
NOTIFIED_AS = {
    SUBSCRIBE_COV: UNCONFIRMED_COV_NOTIFICATION,
    SUBSCRIBE_COV_PROPERTY: UNCONFIRMED_COV_NOTIFICATION,
    SUBSCRIBE_COV_PROPERTY_MULTIPLE: UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
}


def endpoint(**keywords):
    """A B/IP endpoint on loopback with AI-1 at 10.0."""
    owner = BipEndpoint(503_900, interface="127.0.0.1", port=0,
                        broadcast_address="127.0.0.1", **keywords)
    owner.add_analog_input(1, "COV", present_value=10.0)
    return owner


def request(invoke_id, service, body):
    """A confirmed request in an Original-Unicast-NPDU, expecting a reply."""
    apdu = bytes([0x00, 0x05, invoke_id, service]) + bytes.fromhex(body)
    npdu = b"\x01\x04" + apdu
    return struct.pack(">BBH", 0x81, 0x0A, 4 + len(npdu)) + npdu


def real(value):
    """An application-tagged REAL, as a notification carries Present_Value."""
    return b"\x44" + struct.pack(">f", value)


class Subscriber:
    """A raw B/IP socket on loopback that talks to one endpoint."""

    def __init__(self):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.setblocking(False)

    async def send(self, to, invoke_id, service, body):
        ip, port = to.rsplit(":", 1)
        loop = asyncio.get_running_loop()
        await loop.sock_sendto(self.sock, request(invoke_id, service, body), (ip, int(port)))

    async def apdu(self, timeout=2.0):
        """The next APDU the endpoint sends, from a local, unrouted NPDU."""
        loop = asyncio.get_running_loop()
        data, _ = await asyncio.wait_for(loop.sock_recvfrom(self.sock, 2048), timeout)
        assert data[:2] == b"\x81\x0a", data.hex()
        npdu = data[4:]
        assert npdu[0] == 1 and npdu[1] & 0x28 == 0, data.hex()
        return npdu[2:]

    async def quiet(self, timeout=0.2):
        """Nothing more arrives within ``timeout``."""
        with contextlib.suppress(TimeoutError):
            extra = await self.apdu(timeout)
            raise AssertionError(f"unexpected APDU {extra.hex()}")

    def close(self):
        self.sock.close()


def notification(apdu):
    """An unconfirmed COV notification's service and subscriber process."""
    assert apdu[0] == 0x10 and apdu[2] == 0x09, apdu.hex()
    return apdu[1], apdu[3]


class EndpointCovWireTests(unittest.IsolatedAsyncioTestCase):
    async def test_without_cov_the_three_services_are_refused_as_before(self):
        owner = endpoint()
        subscriber = Subscriber()
        try:
            async with owner:
                to = await owner.local_address()
                for invoke_id, (service, body) in enumerate(BODIES.items(), start=1):
                    with self.subTest(service=service):
                        await subscriber.send(to, invoke_id, service, body)
                        # Reject, UNRECOGNIZED_SERVICE.
                        self.assertEqual(await subscriber.apdu(), bytes([0x60, invoke_id, 9]))
                await subscriber.quiet()
                scope = (await owner.server()).service_scope()
                self.assertEqual(scope["executes"], ["read_property"])
        finally:
            subscriber.close()

    async def test_each_subscription_hears_the_applications_changes(self):
        owner = endpoint(cov={})
        subscriber = Subscriber()
        try:
            async with owner:
                to = await owner.local_address()
                for invoke_id, (service, body) in enumerate(BODIES.items(), start=1):
                    with self.subTest(service=service):
                        await subscriber.send(to, invoke_id, service, body)
                        self.assertEqual(await subscriber.apdu(),
                                         bytes([0x20, invoke_id, service]))
                        first = await subscriber.apdu()
                        self.assertEqual(notification(first),
                                         (NOTIFIED_AS[service], invoke_id))
                        self.assertIn(real(10.0), first)
                await subscriber.quiet()

                await owner.set_present_value_local(AI1, PropertyValue.real(25.0))
                heard = [await subscriber.apdu() for _ in BODIES]
                self.assertEqual(
                    sorted(notification(apdu) for apdu in heard),
                    [(UNCONFIRMED_COV_NOTIFICATION, 1), (UNCONFIRMED_COV_NOTIFICATION, 2),
                     (UNCONFIRMED_COV_NOTIFICATION_MULTIPLE, 3)])
                for apdu in heard:
                    self.assertIn(real(25.0), apdu)
                await subscriber.quiet()

                # Status_Flags' FAULT follows a Reliability, and each
                # subscription reports it.
                await owner.set_reliability_local(AI1, Reliability.NO_SENSOR)
                heard = [await subscriber.apdu() for _ in BODIES]
                self.assertEqual(sorted(process for _, process in map(notification, heard)),
                                 [1, 2, 3])
                await subscriber.quiet()
                scope = (await owner.server()).service_scope()
                self.assertEqual(scope["executes"], [
                    "read_property", "subscribe_cov", "subscribe_cov_property",
                    "subscribe_cov_property_multiple"])
        finally:
            subscriber.close()

    async def test_a_timestamped_reference_is_refused_without_a_clock(self):
        owner = endpoint(cov={})
        subscriber = Subscriber()
        try:
            async with owner:
                to = await owner.local_address()
                timestamped = BODIES[SUBSCRIBE_COV_PROPERTY_MULTIPLE].replace(
                    "29 00 1f 4f", "29 01 1f 4f")
                await subscriber.send(to, 1, SUBSCRIBE_COV_PROPERTY_MULTIPLE, timestamped)
                reply = await subscriber.apdu()
                self.assertEqual(reply[:3], bytes([0x50, 1, SUBSCRIBE_COV_PROPERTY_MULTIPLE]))
                # SERVICES (5) / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED (45).
                self.assertIn(bytes.fromhex("91 05 91 2d"), reply)
                await subscriber.quiet()
        finally:
            subscriber.close()

    async def test_the_clients_subscribe_cov_hears_a_local_write(self):
        owner = endpoint(cov={"policy": {"max_subscriptions_per_peer": 4}})
        async with owner:
            address = await owner.local_address()
            async with BACnetClient(interface="127.0.0.1", port=0,
                                    broadcast_address="127.0.0.1",
                                    apdu_timeout_ms=2000) as client:
                notifications = asyncio.Queue()
                iterator = await client.cov_notifications()

                async def collect():
                    async for received in iterator:
                        notifications.put_nowait(received)

                listener = asyncio.create_task(collect())
                try:
                    await client.subscribe_cov(address, 9, AI1, confirmed=False, lifetime=60)
                    for value in (10.0, 30.0):
                        if value != 10.0:
                            await owner.set_present_value_local(AI1, PropertyValue.real(value))
                        received = await asyncio.wait_for(notifications.get(), 2)
                        self.assertEqual(received.subscriber_process_identifier, 9)
                        self.assertEqual(received.monitored_object_identifier, AI1)
                        self.assertIn(PropertyValue.real(value), [
                            item["value"] for item in received.values
                            if item["property_id"] == PropertyIdentifier.PRESENT_VALUE
                        ])
                finally:
                    listener.cancel()
                    with contextlib.suppress(asyncio.CancelledError):
                        await listener


class EndpointCovKeywordTests(unittest.TestCase):
    def test_each_endpoint_takes_a_keyword_only_cov(self):
        for owner in (BipEndpoint, ScEndpoint, MstpEndpoint):
            with self.subTest(owner=owner.__name__):
                parameter = inspect.signature(owner).parameters["cov"]
                self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertIsNone(parameter.default)

    def test_the_bounds_themselves_are_taken(self):
        widest = {"outbox_entries": 65_535, "outbox_octets": 16 * 1024 * 1024,
                  "confirmed_timeout_ms": 3_600_000}
        for owner in (lambda: endpoint(cov=widest),
                      lambda: MstpEndpoint(2490, "/tmp/nonexistent", cov=widest)):
            with self.subTest(owner=owner):
                owner()

    def test_mstp_and_sc_endpoints_take_cov_too(self):
        cov = {"outbox_entries": 4, "confirmed_retries": 0}
        MstpEndpoint(2490, "/tmp/nonexistent", cov=cov)
        ScEndpoint(2490, "wss://localhost:1", bytes([2, 0, 0, 0, 0, 1]), "absent-ca.pem",
                   "absent.pem", "absent.key", sc_device_uuid=bytes([0x11] * 16), cov=cov)

    def test_invalid_cov_dicts_fail_at_construction(self):
        for cov, error, message in [
            ([("outbox_entries", 1)], TypeError, r"is not an instance of 'dict'"),
            ({"outbox": 1}, TypeError, r"^cov got an unexpected key 'outbox'$"),
            ({1: 1}, TypeError, r"^cov keys must be str$"),
            ({"outbox_entries": "16"}, TypeError, r"^cov\['outbox_entries'\]: "),
            ({"policy": [1]}, TypeError, r"^cov\['policy'\]: "),
            ({"policy": {"max_subscriptions": 1}}, TypeError,
             r"^cov\['policy'\] got an unexpected key 'max_subscriptions'$"),
            ({"confirmed_retries": 256}, OverflowError, r"^cov\['confirmed_retries'\]: "),
            ({"confirmed_timeout_ms": -1}, OverflowError, r"^cov\['confirmed_timeout_ms'\]: "),
            ({"outbox_entries": 0}, ValueError, r"^cov\['outbox_entries'\] must be 1 to 65535$"),
            ({"outbox_entries": 65_536}, ValueError,
             r"^cov\['outbox_entries'\] must be 1 to 65535$"),
            ({"outbox_entries": 2**40}, ValueError,
             r"^cov\['outbox_entries'\] must be 1 to 65535$"),
            ({"outbox_octets": 0}, ValueError,
             r"^cov\['outbox_octets'\] must be 1 to 16777216$"),
            ({"outbox_octets": 16 * 1024 * 1024 + 1}, ValueError,
             r"^cov\['outbox_octets'\] must be 1 to 16777216$"),
            ({"confirmed_timeout_ms": 0}, ValueError,
             r"^cov\['confirmed_timeout_ms'\] must be 1 to 3600000$"),
            ({"confirmed_timeout_ms": 3_600_001}, ValueError,
             r"^cov\['confirmed_timeout_ms'\] must be 1 to 3600000$"),
            ({"policy": {"max_subscriptions_global": 0}}, ValueError,
             r"max_subscriptions_global must be positive$"),
        ]:
            for owner in (
                lambda cov: endpoint(cov=cov),
                lambda cov: MstpEndpoint(2490, "/tmp/nonexistent", cov=cov),
            ):
                with self.subTest(cov=cov, owner=owner):
                    with self.assertRaisesRegex(error, message):
                        owner(cov)


class EndpointLocalWriteTests(unittest.IsolatedAsyncioTestCase):
    async def test_local_changes_before_start_raise_runtime_error(self):
        owner = endpoint(cov={})
        with self.assertRaisesRegex(RuntimeError, "endpoint not started"):
            await owner.set_present_value_local(AI1, PropertyValue.real(1.0))
        with self.assertRaisesRegex(RuntimeError, "endpoint not started"):
            await owner.set_reliability_local(AI1, Reliability.NO_SENSOR)

    async def test_local_changes_are_refused_as_on_the_server(self):
        from rusty_bacnet import BacnetProtocolError, ErrorClass, ErrorCode
        owner = endpoint()
        async with owner:
            missing = ObjectIdentifier(ObjectType.ANALOG_INPUT, 9)
            with self.assertRaises(BacnetProtocolError) as caught:
                await owner.set_present_value_local(missing, PropertyValue.real(1.0))
            self.assertEqual((caught.exception.error_class, caught.exception.error_code),
                             (ErrorClass.OBJECT.to_raw(), ErrorCode.UNKNOWN_OBJECT.to_raw()))
            # Without COV a change is still made: a client's read sees it.
            await owner.set_present_value_local(AI1, PropertyValue.real(12.5))
            async with BACnetClient(interface="127.0.0.1", port=0,
                                    broadcast_address="127.0.0.1",
                                    apdu_timeout_ms=2000) as client:
                value = await client.read_property(await owner.local_address(), AI1,
                                                   PropertyIdentifier.PRESENT_VALUE)
            self.assertEqual(value, PropertyValue.real(12.5))


if __name__ == "__main__":
    unittest.main()
