"""COV on an ScEndpoint over a local hub (#2490): with ``cov`` a client's
SubscribeCOV over BACnet/SC is answered and a ``set_present_value_local``
reaches it; without it the request is rejected as an unrecognized service.
"""
import asyncio
import contextlib
import unittest

from rusty_bacnet import (
    BacnetRejectError, ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue,
    RejectReason, ScEndpoint,
)
from sc_client_fixture import ClientScFixture, mtls

VMAC = b"\x02\0\0\0\0\x05"
AI1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)


class ScEndpointCovTests(ClientScFixture):
    @staticmethod
    async def close_endpoint(endpoint):
        # Called in the loop: `close()` makes its awaitable on the running loop.
        await asyncio.wait_for(endpoint.close(), 5)

    async def subscribed(self, cov):
        """Start an ScEndpoint with ``cov`` and a client on one hub, have the
        client subscribe to AI-1, then change it: the two notifications."""
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 5)
        url = await hub.url()
        endpoint = ScEndpoint(device_instance=4105, sc_hub=url, sc_vmac=VMAC,
                              sc_ca_cert=self.path("site.pem"),
                              sc_client_cert=self.path("server.pem"),
                              sc_client_key=self.path("server.key"),
                              sc_device_uuid=mtls.SERVER_UUID, cov=cov)
        endpoint.add_analog_input(1, "COV", present_value=10.0)
        await asyncio.wait_for(endpoint.start(), 10)
        self.addAsyncCleanup(self.close_endpoint, endpoint)
        client = self.client(url, apdu_timeout_ms=2000)
        await asyncio.wait_for(client.__aenter__(), 10)
        address = ":".join(f"{octet:02x}" for octet in VMAC)
        notifications = asyncio.Queue()
        iterator = await client.cov_notifications()

        async def collect():
            async for received in iterator:
                notifications.put_nowait(received)

        listener = asyncio.create_task(collect())
        try:
            await client.subscribe_cov(address, 7, AI1, confirmed=False, lifetime=60)
            first = await asyncio.wait_for(notifications.get(), 5)
            await endpoint.set_present_value_local(AI1, PropertyValue.real(33.0))
            second = await asyncio.wait_for(notifications.get(), 5)
            return first, second
        finally:
            listener.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await listener

    async def test_with_cov_a_client_hears_a_local_change(self):
        first, second = await self.subscribed({})
        for notification in (first, second):
            self.assertEqual(notification.subscriber_process_identifier, 7)
            self.assertEqual(notification.monitored_object_identifier, AI1)
        values = [item["value"] for item in second.values
                  if item["property_id"] == PropertyIdentifier.PRESENT_VALUE]
        self.assertIn(PropertyValue.real(33.0), values)

    async def test_without_cov_the_subscription_is_rejected(self):
        with self.assertRaises(BacnetRejectError) as caught:
            await self.subscribed(None)
        self.assertEqual(caught.exception.reason, RejectReason.UNRECOGNIZED_SERVICE)


if __name__ == "__main__":
    unittest.main()
