"""WriteProperty from an endpoint's client (#2617): the BACnetClient on the
endpoint's socket, its synchronous preflight and independent wire vectors.
The narrow client's commandability= keyword went with it."""
import asyncio
import ast
import inspect
from pathlib import Path
import socket
import unittest

import rusty_bacnet
from rusty_bacnet import (BACnetClient, BACnetServer, BipEndpoint,
                         BacnetProtocolError, ObjectIdentifier, ObjectType,
                         PropertyIdentifier, PropertyValue)


class EndpointWriteTests(unittest.IsolatedAsyncioTestCase):
    async def test_synchronous_preflight(self):
        endpoint = BipEndpoint(device_instance=9180, interface="127.0.0.1", port=0)
        await endpoint.start()
        try:
            role = await endpoint.client()
            self.assertIsInstance(role, BACnetClient)
            args = ("not-an-address", ObjectIdentifier(ObjectType.ANALOG_VALUE, 7),
                    PropertyIdentifier.PRESENT_VALUE, PropertyValue.null())
            # No commandability= any more: it went with the narrow client.
            with self.assertRaises(TypeError):
                role.write_property(*args, commandability="commandable")
            for priority in (0, 17, 255):
                with self.assertRaisesRegex(ValueError, "priority"):
                    role.write_property(*args, priority=priority)
            for priority in (-1, 256):
                with self.assertRaises(OverflowError):
                    role.write_property(*args, priority=priority)
            self.assertEqual((await endpoint.status())["active_leases"], 0)
        finally:
            await endpoint.close()

    async def test_wire_value_priority_result_and_awaitable_contract(self):
        endpoint = BipEndpoint(device_instance=9181, interface="127.0.0.1", port=0)
        await endpoint.start()
        oid = ObjectIdentifier(ObjectType.ANALOG_VALUE, 7)
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0)); peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            try:
                role = await endpoint.client()
                for _ in range(2):
                    for raw, priority, error in ((b"", None, False), (b"\x00", None, False),
                                                 (b"\x00" * 32, 8, False), (b"\x00" * 33, 8, False),
                                                 (b"", None, True)):
                        future = role.write_property(address, oid, PropertyIdentifier.PRESENT_VALUE,
                                                     PropertyValue.application_data(raw), priority=priority,
                                                     array_index=2)
                        self.assertTrue(inspect.isawaitable(future))
                        task = asyncio.ensure_future(future)
                        try:
                            wire, remote = await asyncio.wait_for(loop.sock_recvfrom(peer, 4096), 2)
                            self.assertEqual(wire[9], 15)
                            expected = b"\x0c" + ((2 << 22) | 7).to_bytes(4, "big") + b"\x19\x55\x29\x02\x3e" + raw + b"\x3f"
                            if priority is not None: expected += b"\x49" + bytes([priority])
                            self.assertEqual(wire[10:], expected)
                            payload = b"\x01\x00" + (b"\x50" if error else b"\x20") + bytes([wire[8], 15])
                            if error: payload += b"\x91\x02\x91\x28"
                            await loop.sock_sendto(peer, b"\x81\x0a" + (len(payload)+4).to_bytes(2,"big") + payload, remote)
                            if error:
                                with self.assertRaises(BacnetProtocolError): await asyncio.wait_for(task, 2)
                            else: self.assertIsNone(await asyncio.wait_for(task, 2))
                        finally:
                            if not task.done(): task.cancel()
                            await asyncio.gather(task, return_exceptions=True)
                await endpoint.close()
                with self.assertRaisesRegex(RuntimeError, "ran on an endpoint"):
                    await role.write_property(address, oid, PropertyIdentifier.PRESENT_VALUE, PropertyValue.null())
            finally:
                await endpoint.close()

    async def test_real_empty_recipient_list_and_scalar_error(self):
        server = BACnetServer(9182, interface="127.0.0.1", port=0)
        server.add_notification_class(1, "empty recipients")
        server.add_analog_value(7, "scalar")
        endpoint = BipEndpoint(device_instance=9183, interface="127.0.0.1", port=0)
        await server.start(); await endpoint.start()
        try:
            role = await endpoint.client(); address = await server.local_address()
            self.assertIsNone(await role.write_property(address, ObjectIdentifier(ObjectType.NOTIFICATION_CLASS, 1),
                              PropertyIdentifier.RECIPIENT_LIST, PropertyValue.application_data(b"")))
            with self.assertRaises(BacnetProtocolError):
                await role.write_property(address, ObjectIdentifier(ObjectType.ANALOG_VALUE, 7),
                      PropertyIdentifier.PRESENT_VALUE, PropertyValue.application_data(b""))
        finally:
            await endpoint.close(); await server.stop()

    def test_native_signature_and_installed_stub(self):
        signature = inspect.signature(BACnetClient.write_property)
        self.assertNotIn("commandability", signature.parameters)
        tree = ast.parse(Path(rusty_bacnet.__file__).with_suffix(".pyi").read_text())
        cls = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "BACnetClient")
        method = next(n for n in cls.body if isinstance(n, ast.FunctionDef) and n.name == "write_property")
        self.assertEqual(ast.unparse(method.returns), "Awaitable[None]")
        self.assertEqual([a.arg for a in method.args.kwonlyargs], [])
