"""RPM from an endpoint's client (#2617), the standalone profile on the
endpoint's socket: ordered results and independent ACK vectors."""
import asyncio
import socket
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, BipEndpoint, BacnetError,
    ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue,
)


class EndpointRpmTests(unittest.IsolatedAsyncioTestCase):
    async def test_server_results_shared_with_standalone_and_profile_preflight(self):
        server = BACnetServer(9123, interface="127.0.0.1", port=0)
        endpoint = BipEndpoint(device_instance=9124, interface="127.0.0.1", port=0)
        await server.start()
        await endpoint.start()
        oid = ObjectIdentifier(ObjectType.DEVICE, 9123)
        specs = [(oid, [(PropertyIdentifier.OBJECT_NAME, None),
                        (PropertyIdentifier.OBJECT_LIST, 0),
                        (PropertyIdentifier.OBJECT_NAME, 2),
                        (PropertyIdentifier.OBJECT_NAME, None)])]
        try:
            role = await endpoint.client()
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                expected = await client.read_property_multiple(await server.local_address(), specs)
                actual = await role.read_property_multiple(await server.local_address(), specs)
                self.assertEqual(actual, expected)
                self.assertEqual(actual[0]["object_id"], oid)
                results = actual[0]["results"]
                self.assertEqual(len(results), 4)
                self.assertIsNotNone(results[2]["error"])
                self.assertIsNone(results[2]["value"])
                self.assertEqual(results[0], results[3])
            # The standalone profile: an empty list is refused before
            # anything is sent; selectors and wildcard instances are taken.
            for specs in ([], [(oid, [])]):
                with self.subTest(specs=specs), self.assertRaisesRegex(BacnetError, "nonempty"):
                    await role.read_property_multiple(await server.local_address(), specs)
            all_of = await role.read_property_multiple(
                await server.local_address(), [(oid, [(PropertyIdentifier.ALL, None)])])
            self.assertGreater(len(all_of[0]["results"]), 4)
            self.assertEqual((await endpoint.status())["active_leases"], 0)
            self.assertIsInstance(role, BACnetClient)
        finally:
            await endpoint.close()
            await server.stop()
        with self.assertRaisesRegex(RuntimeError, "ran on an endpoint"):
            await role.read_property_multiple("127.0.0.1:47808", [(oid, [(PropertyIdentifier.OBJECT_NAME, None)])])

    async def test_independent_ordered_ack_errors_and_corruption(self):
        endpoint = BipEndpoint(device_instance=9125, interface="127.0.0.1", port=0)
        await endpoint.start()
        oid = ObjectIdentifier(ObjectType.DEVICE, 10)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            loop = asyncio.get_running_loop()
            try:
                role = await endpoint.client()
                for fault in (None, "wrong_index", "missing", "extra", "reorder", "wrong_object", "malformed"):
                    with self.subTest(fault=fault):
                        task = asyncio.ensure_future(role.read_property_multiple(address, [
                            (oid, [(PropertyIdentifier.OBJECT_NAME, None), (PropertyIdentifier.OBJECT_LIST, 0)])]))
                        try:
                            wire, remote = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 2)
                            self.assertEqual(wire[9], 14)
                            # Object[0], list[1]; result[2], optional index[3], value[4]/error[5].
                            first = b"\x29\x4d\x4e\x21\x2a\x4f"
                            # Omitted error index is legal even though request index is zero.
                            second = b"\x29\x4c" + (b"\x39\x01" if fault == "wrong_index" else b"") + b"\x5e\x91\x02\x91\x20\x5f"
                            results = first + second
                            if fault == "missing": results = first
                            if fault == "extra": results += first
                            if fault == "reorder": results = second + first
                            object_bytes = ((8 << 22) | (11 if fault == "wrong_object" else 10)).to_bytes(4, "big")
                            body = b"\x0c" + object_bytes + b"\x1e" + results + b"\x1f"
                            if fault == "malformed": body = b"\x00"
                            payload = b"\x01\x00\x30" + bytes([wire[8], 14]) + body
                            await loop.sock_sendto(peer, b"\x81\x0a" + (len(payload) + 4).to_bytes(2, "big") + payload, remote)
                            if fault == "malformed":
                                with self.assertRaises(BacnetError): await asyncio.wait_for(task, 2)
                            elif fault:
                                # The standalone profile (#2617): the answer as the
                                # device gave it, each result naming what it is, for
                                # the caller to compare with what it asked.
                                answered = await asyncio.wait_for(task, 2)
                                if fault == "wrong_object":
                                    self.assertEqual(answered[0]["object_id"],
                                                     ObjectIdentifier(ObjectType.DEVICE, 11))
                                if fault == "extra":
                                    self.assertEqual(len(answered[0]["results"]), 3)
                                if fault == "missing":
                                    self.assertEqual(len(answered[0]["results"]), 1)
                                if fault == "reorder":
                                    self.assertEqual(answered[0]["results"][0]["property_id"],
                                                     PropertyIdentifier.OBJECT_LIST)
                                if fault == "wrong_index":
                                    self.assertEqual(answered[0]["results"][1]["array_index"], 1)
                            else:
                                result = (await asyncio.wait_for(task, 2))[0]["results"]
                                self.assertEqual(result[0]["value"], PropertyValue.unsigned(42))
                                self.assertIsNone(result[1]["array_index"])
                                self.assertIsNotNone(result[1]["error"])
                            self.assertEqual((await endpoint.status())["active_leases"], 0)
                        finally:
                            if not task.done(): task.cancel()
                            await asyncio.gather(task, return_exceptions=True)
            finally:
                await endpoint.close()
