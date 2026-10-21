"""Generated Python endpoint Devices use the link-resolved shared identity."""
import asyncio
import unittest

from rusty_bacnet import BipEndpoint, ObjectIdentifier, ObjectType, PropertyIdentifier, ScEndpoint
from test_sc_hub_mtls import MtlsFixture, SERVER_UUID, CLIENT_UUID

CASES = ((999, 999), (1474, 1474), (65535, 1476))
DEVICE = ObjectIdentifier(ObjectType.DEVICE, 1720)
CAPACITY = PropertyIdentifier.MAX_APDU_LENGTH_ACCEPTED


class BipIdentityCapacityTests(unittest.IsolatedAsyncioTestCase):
    async def test_generated_device_matches_resolved_capacity(self):
        reader = BipEndpoint(device_instance=1721, interface="127.0.0.1", port=0,
                             broadcast_address="127.0.0.1")
        async with reader:
            client = await reader.client()
            for requested, effective in CASES:
                with self.subTest(requested=requested):
                    target = BipEndpoint(device_instance=1720, interface="127.0.0.1", port=0,
                                         broadcast_address="127.0.0.1", max_apdu=requested)
                    async with target:
                        self.assertEqual((await target.status())["max_apdu"], effective)
                        value = await asyncio.wait_for(client.read_property(
                            await target.local_address(), DEVICE, CAPACITY), 3)
                        self.assertEqual(value.value, effective)
                        await target.broadcast_i_am()


class ScIdentityCapacityTests(MtlsFixture):
    def endpoint(self, hub_url, instance, raw=1476):
        server = instance == 1720
        return ScEndpoint(
            device_instance=instance, sc_hub=hub_url,
            sc_vmac=b"\x02\0\0\0\0" + bytes([2 if server else 3]),
            sc_device_uuid=SERVER_UUID if server else CLIENT_UUID,
            sc_ca_cert=self.path("site.pem"),
            sc_client_cert=self.path("server.pem" if server else "client.pem"),
            sc_client_key=self.path("server.key" if server else "client.key"),
            max_apdu=raw,
        )

    async def test_generated_device_matches_resolved_capacity(self):
        for requested, effective in CASES:
            with self.subTest(requested=requested):
                hub = self.hub()
                await asyncio.wait_for(hub.start(), 5)
                try:
                    url = await hub.url()
                    target = self.endpoint(url, 1720, requested)
                    reader = self.endpoint(url, 1721)
                    async with target, reader:
                        self.assertEqual((await target.status())["max_apdu"], effective)
                        client = await reader.client()
                        value = await asyncio.wait_for(client.read_property(
                            "02:00:00:00:00:02", DEVICE, CAPACITY), 3)
                        self.assertEqual(value.value, effective)
                        await target.broadcast_i_am()
                finally:
                    await asyncio.wait_for(hub.stop(), 5)

    async def test_invalid_capacity_precedes_sc_dial(self):
        accepted = []
        observed = asyncio.Event()

        def connected(_reader, writer):
            accepted.append(writer)
            observed.set()
            writer.close()

        listener = await asyncio.start_server(connected, "127.0.0.1", 0)
        address = listener.sockets[0].getsockname()
        try:
            for invalid in (0, 49):
                with self.subTest(max_apdu=invalid):
                    with self.assertRaisesRegex(ValueError, "max-APDU"):
                        self.endpoint(f"wss://127.0.0.1:{address[1]}", 1720, invalid)
            self.assertEqual(accepted, [])
            # Qualify the same accept queue after synchronous validation returns.
            _reader, writer = await asyncio.open_connection(*address)
            writer.close()
            await writer.wait_closed()
            await asyncio.wait_for(observed.wait(), 3)
            self.assertEqual(len(accepted), 1)
        finally:
            listener.close()
            await listener.wait_closed()
            for writer in accepted:
                await writer.wait_closed()
