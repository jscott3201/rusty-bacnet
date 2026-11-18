"""COV on the Python endpoints' full server (#2617): it answers SubscribeCOV,
SubscribeCOVProperty and SubscribeCOVPropertyMultiple as a standalone server
does, and the changes the application makes through
``await endpoint.server()`` reach the subscribers, as a peer's writes do when
the endpoint's ``mutation_policy`` lets them. The read-only server, the
default, takes the subscriptions; ``mutation_policy="deny_all"`` denies them.
``cov_policy`` is read as ``BACnetServer``'s is.

A raw B/IP socket plays the subscriber and the tests assert what crosses the
wire: the replies, the first notification of each subscription and the ones
a local change sends. The BACnetClient's own ``subscribe_cov`` is checked
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
AV1 = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
SUBSCRIBE_COV, SUBSCRIBE_COV_PROPERTY, SUBSCRIBE_COV_PROPERTY_MULTIPLE = 5, 28, 30
UNCONFIRMED_COV_NOTIFICATION, UNCONFIRMED_COV_NOTIFICATION_MULTIPLE = 2, 11
# SERVICES / SERVICE_REQUEST_DENIED, as an Error PDU carries them.
DENIED = bytes.fromhex("91 05 91 1d")

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
    async def test_with_deny_all_the_three_services_are_denied(self):
        owner = endpoint(mutation_policy="deny_all")
        subscriber = Subscriber()
        try:
            async with owner:
                to = await owner.local_address()
                for invoke_id, (service, body) in enumerate(BODIES.items(), start=1):
                    with self.subTest(service=service):
                        await subscriber.send(to, invoke_id, service, body)
                        reply = await subscriber.apdu()
                        # An Error PDU, SERVICES / SERVICE_REQUEST_DENIED.
                        self.assertEqual(reply[:3], bytes([0x50, invoke_id, service]))
                        self.assertIn(DENIED, reply)
                await subscriber.quiet()
        finally:
            subscriber.close()

    async def test_each_subscription_hears_the_applications_changes(self):
        # The read-only default takes the subscriptions (#2617).
        owner = endpoint()
        subscriber = Subscriber()
        try:
            async with owner:
                to = await owner.local_address()
                server = await owner.server()
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

                await server.set_present_value_local(AI1, PropertyValue.real(25.0))
                heard = [await subscriber.apdu(timeout=6.0) for _ in BODIES]
                self.assertEqual(
                    sorted(notification(apdu) for apdu in heard),
                    [(UNCONFIRMED_COV_NOTIFICATION, 1), (UNCONFIRMED_COV_NOTIFICATION, 2),
                     (UNCONFIRMED_COV_NOTIFICATION_MULTIPLE, 3)])
                for apdu in heard:
                    self.assertIn(real(25.0), apdu)
                await subscriber.quiet()

                # Status_Flags' FAULT follows a Reliability, and each
                # subscription reports it.
                await server.set_reliability_local(AI1, Reliability.NO_SENSOR)
                heard = [await subscriber.apdu(timeout=6.0) for _ in BODIES]
                self.assertEqual(sorted(process for _, process in map(notification, heard)),
                                 [1, 2, 3])
                await subscriber.quiet()
        finally:
            subscriber.close()

    async def test_the_cov_policy_caps_a_peers_subscriptions(self):
        # The endpoint's cov_policy reaches its server: past the peer's
        # quota of one, a subscription is refused for want of room.
        owner = endpoint(cov_policy={"max_subscriptions_per_peer": 1})
        subscriber = Subscriber()
        try:
            async with owner:
                to = await owner.local_address()
                await subscriber.send(to, 1, SUBSCRIBE_COV, BODIES[SUBSCRIBE_COV])
                self.assertEqual(await subscriber.apdu(), bytes([0x20, 1, SUBSCRIBE_COV]))
                await subscriber.apdu()  # its first notification
                second = BODIES[SUBSCRIBE_COV].replace("09 01", "09 02", 1)
                await subscriber.send(to, 2, SUBSCRIBE_COV, second)
                # An Error PDU, RESOURCES (3) / NO_SPACE_TO_ADD_LIST_ELEMENT (19).
                self.assertEqual(await subscriber.apdu(),
                                 bytes.fromhex("50 02 05 91 03 91 13"))
                counters = await (await owner.server()).cov_counters()
                self.assertEqual(counters["subscriptions_rejected_quota"], 1)
        finally:
            subscriber.close()

    async def test_a_timestamped_reference_is_taken_with_the_servers_clock(self):
        # The narrow endpoint had no clock and refused it; the full server
        # has one (#2617).
        owner = endpoint()
        subscriber = Subscriber()
        try:
            async with owner:
                to = await owner.local_address()
                timestamped = BODIES[SUBSCRIBE_COV_PROPERTY_MULTIPLE].replace(
                    "29 00 1f 4f", "29 01 1f 4f")
                await subscriber.send(to, 1, SUBSCRIBE_COV_PROPERTY_MULTIPLE, timestamped)
                self.assertEqual(await subscriber.apdu(),
                                 bytes([0x20, 1, SUBSCRIBE_COV_PROPERTY_MULTIPLE]))
        finally:
            subscriber.close()

    async def test_the_clients_subscribe_cov_hears_a_local_write(self):
        owner = endpoint(cov_policy={"max_subscriptions_per_peer": 4})
        async with owner:
            address = await owner.local_address()
            server = await owner.server()
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
                            await server.set_present_value_local(AI1, PropertyValue.real(value))
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

    async def test_a_peers_write_the_policy_allows_notifies_the_subscribers(self):
        # Network writes notify too, as on a standalone server: the narrow
        # endpoint took no write to an object (#2617).
        owner = endpoint(mutation_policy="permissive")
        owner.add_analog_value(1, "Setpoint")
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
                    await client.subscribe_cov(address, 4, AV1, confirmed=False, lifetime=60)
                    await asyncio.wait_for(notifications.get(), 2)
                    await client.write_property(address, AV1, PropertyIdentifier.PRESENT_VALUE,
                                                PropertyValue.real(21.5), priority=8)
                    received = await asyncio.wait_for(notifications.get(), 2)
                    self.assertIn(PropertyValue.real(21.5), [
                        item["value"] for item in received.values
                        if item["property_id"] == PropertyIdentifier.PRESENT_VALUE
                    ])
                finally:
                    listener.cancel()
                    with contextlib.suppress(asyncio.CancelledError):
                        await listener


class EndpointCovKeywordTests(unittest.TestCase):
    def test_each_endpoint_takes_a_keyword_only_cov_policy(self):
        for owner in (BipEndpoint, ScEndpoint, MstpEndpoint):
            with self.subTest(owner=owner.__name__):
                parameters = inspect.signature(owner).parameters
                parameter = parameters["cov_policy"]
                self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertIsNone(parameter.default)
                self.assertNotIn("cov", parameters)

    def test_invalid_cov_policies_fail_at_construction_as_on_the_server(self):
        sc = (2490, "wss://localhost:1", bytes([2, 0, 0, 0, 0, 1]), "absent-ca.pem",
              "absent.pem", "absent.key")
        for policy, error, message in [
            ([("max_subscriptions_global", 1)], TypeError, r"is not an instance of 'dict'"),
            ({"max_subscriptions": 1}, TypeError,
             r"^cov_policy got an unexpected key 'max_subscriptions'$"),
            ({"max_subscriptions_global": 0}, ValueError,
             r"max_subscriptions_global must be positive$"),
        ]:
            for owner in (
                lambda policy: endpoint(cov_policy=policy),
                lambda policy: MstpEndpoint(2490, "/tmp/nonexistent", cov_policy=policy),
                lambda policy: ScEndpoint(*sc, sc_device_uuid=bytes([0x11] * 16),
                                          cov_policy=policy),
            ):
                with self.subTest(policy=policy, owner=owner):
                    with self.assertRaisesRegex(error, message):
                        owner(policy)


class EndpointLocalWriteTests(unittest.IsolatedAsyncioTestCase):
    async def test_the_server_is_there_only_while_the_endpoint_runs(self):
        owner = endpoint()
        with self.assertRaisesRegex(RuntimeError, "endpoint not started"):
            await owner.server()
        async with owner:
            server = await owner.server()
            await server.set_present_value_local(AI1, PropertyValue.real(1.0))
        with self.assertRaisesRegex(RuntimeError, "came from is closed"):
            await server.set_reliability_local(AI1, Reliability.NO_SENSOR)
        with self.assertRaisesRegex(RuntimeError, "endpoint not started"):
            await owner.server()

    async def test_local_changes_are_refused_as_on_the_server(self):
        from rusty_bacnet import BacnetProtocolError, ErrorClass, ErrorCode
        owner = endpoint()
        async with owner:
            server = await owner.server()
            missing = ObjectIdentifier(ObjectType.ANALOG_INPUT, 9)
            with self.assertRaises(BacnetProtocolError) as caught:
                await server.set_present_value_local(missing, PropertyValue.real(1.0))
            self.assertEqual((caught.exception.error_class, caught.exception.error_code),
                             (ErrorClass.OBJECT.to_raw(), ErrorCode.UNKNOWN_OBJECT.to_raw()))
            # A local change is made under the read-only stance too: a
            # client's read sees it.
            await server.set_present_value_local(AI1, PropertyValue.real(12.5))
            async with BACnetClient(interface="127.0.0.1", port=0,
                                    broadcast_address="127.0.0.1",
                                    apdu_timeout_ms=2000) as client:
                value = await client.read_property(await owner.local_address(), AI1,
                                                   PropertyIdentifier.PRESENT_VALUE)
            self.assertEqual(value, PropertyValue.real(12.5))


if __name__ == "__main__":
    unittest.main()
