"""The ScEndpoint's full server over a local hub (#2617): a client's
SubscribeCOV over BACnet/SC is answered and a ``set_present_value_local``
on ``await endpoint.server()`` reaches it, its reads see the change, and
its WriteProperty is denied by the read-only default; with
``mutation_policy="deny_all"`` the subscription is denied too.
"""
import asyncio
import contextlib
import unittest

from rusty_bacnet import (
    BacnetProtocolError, BACnetServer, ErrorClass, ErrorCode, ObjectIdentifier,
    ObjectType, PropertyIdentifier, PropertyValue, ScEndpoint,
)
from sc_client_fixture import ClientScFixture, mtls

VMAC = b"\x02\0\0\0\0\x05"
AI1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
PV = PropertyIdentifier.PRESENT_VALUE


class ScEndpointServerTests(ClientScFixture):
    @staticmethod
    async def close_endpoint(endpoint):
        # Called in the loop: `close()` makes its awaitable on the running loop.
        await asyncio.wait_for(endpoint.close(), 5)

    async def started(self, **keywords):
        """An ScEndpoint with AI-1 and a client on one hub: the endpoint, the
        client and the endpoint's address."""
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 5)
        url = await hub.url()
        endpoint = ScEndpoint(device_instance=4105, sc_hub=url, sc_vmac=VMAC,
                              sc_ca_cert=self.path("site.pem"),
                              sc_client_cert=self.path("server.pem"),
                              sc_client_key=self.path("server.key"),
                              sc_device_uuid=mtls.SERVER_UUID, **keywords)
        endpoint.add_analog_input(1, "COV", present_value=10.0)
        await asyncio.wait_for(endpoint.start(), 10)
        self.addAsyncCleanup(self.close_endpoint, endpoint)
        client = self.client(url, apdu_timeout_ms=2000)
        await asyncio.wait_for(client.__aenter__(), 10)
        return endpoint, client, ":".join(f"{octet:02x}" for octet in VMAC)

    async def subscribed(self, endpoint, client, address):
        """Have the client subscribe to AI-1, then change it through the
        endpoint's server: the two notifications."""
        notifications = asyncio.Queue()
        iterator = await client.cov_notifications()

        async def collect():
            async for received in iterator:
                notifications.put_nowait(received)

        listener = asyncio.create_task(collect())
        try:
            await client.subscribe_cov(address, 7, AI1, confirmed=False, lifetime=60)
            first = await asyncio.wait_for(notifications.get(), 5)
            server = await endpoint.server()
            self.assertIsInstance(server, BACnetServer)
            await server.set_present_value_local(AI1, PropertyValue.real(33.0))
            second = await asyncio.wait_for(notifications.get(), 5)
            return first, second
        finally:
            listener.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await listener

    async def test_a_client_hears_a_local_change_and_reads_it(self):
        endpoint, client, address = await self.started()
        first, second = await self.subscribed(endpoint, client, address)
        for notification in (first, second):
            self.assertEqual(notification.subscriber_process_identifier, 7)
            self.assertEqual(notification.monitored_object_identifier, AI1)
        values = [item["value"] for item in second.values
                  if item["property_id"] == PV]
        self.assertIn(PropertyValue.real(33.0), values)
        self.assertEqual(await client.read_property(address, AI1, PV),
                         PropertyValue.real(33.0))
        # The read-only default denies the client's write.
        with self.assertRaises(BacnetProtocolError) as caught:
            await client.write_property(address, AI1, PropertyIdentifier.OUT_OF_SERVICE,
                                        PropertyValue.boolean(True))
        self.assertEqual((caught.exception.error_class, caught.exception.error_code),
                         (ErrorClass.SERVICES, ErrorCode.SERVICE_REQUEST_DENIED))

    async def test_with_deny_all_the_subscription_is_denied(self):
        endpoint, client, address = await self.started(mutation_policy="deny_all")
        with self.assertRaises(BacnetProtocolError) as caught:
            await self.subscribed(endpoint, client, address)
        self.assertEqual((caught.exception.error_class, caught.exception.error_code),
                         (ErrorClass.SERVICES, ErrorCode.SERVICE_REQUEST_DENIED))


if __name__ == "__main__":
    unittest.main()
