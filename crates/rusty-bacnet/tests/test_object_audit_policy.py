"""Installed AV/BV policy authoring, validation and actual B/IP target records."""
import asyncio
import ast
import inspect
import socket
from pathlib import Path
import tempfile
import unittest

import rusty_bacnet as rb


class ObjectAuditPolicyTests(unittest.IsolatedAsyncioTestCase):
    def test_all_builder_parsers_validate_before_registration_and_stub_parity(self):
        owners = [
            rb.BACnetServer(8101), rb.BipEndpoint(device_instance=8102),
            rb.MstpEndpoint(device_instance=8103, serial_port="/nonexistent"),
            rb.ScEndpoint(device_instance=8104, sc_hub="wss://localhost:1",
                sc_vmac=b"\x02\0\0\0\0\1", sc_device_uuid=bytes.fromhex("8e62ac46d7084226913776a32b619315"),
                sc_ca_cert="absent", sc_client_cert="absent", sc_client_key="absent"),
        ]
        tree = ast.parse(Path(rb.__file__).with_suffix(".pyi").read_text())
        for owner in owners:
            for name in ("add_analog_value", "add_binary_value"):
                method = getattr(owner, name)
                for option in ({"audit_level": "invalid"}, {"auditable_operations": 1 << 16},
                               {"auditable_operations": True}, {"auditable_operations": -1},
                               {"audit_priority_filter": 65536}, {"audit_priority_filter": True}):
                    with self.subTest(owner=type(owner).__name__, method=name, option=option):
                        with self.assertRaises((ValueError, TypeError)):
                            method(1, name, **option)
                # Same identifier/name succeeds: invalid calls stored nothing.
                method(1, name, audit_level="default", auditable_operations=0,
                       audit_priority_filter="inherit")
                cls = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == type(owner).__name__)
                stub = next(n for n in cls.body if isinstance(n, ast.FunctionDef) and n.name == name)
                expected = [arg.arg for arg in stub.args.args + stub.args.kwonlyargs if arg.arg != "self"]
                self.assertEqual(list(inspect.signature(method).parameters), expected)

    async def test_server_and_endpoint_policy_rows_survive_start_and_are_readable(self):
        for endpoint in (False, True):
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
                sock.bind(("127.0.0.1", 0))
                port = sock.getsockname()[1]
            owner = (rb.BipEndpoint(device_instance=8111, interface="127.0.0.1", port=port)
                     if endpoint else rb.BACnetServer(8110, interface="127.0.0.1", port=0))
            with self.assertRaises(ValueError):
                owner.add_analog_value(1, "av", auditable_operations=1 << 16)
            owner.add_analog_value(1, "av", audit_level="default", auditable_operations=3,
                                   audit_priority_filter="inherit")
            owner.add_binary_value(1, "bv", audit_priority_filter=129)
            await owner.start()
            try:
                address = await owner.local_address()
                async with rb.BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=1000) as client:
                    av = rb.ObjectIdentifier(rb.ObjectType.ANALOG_VALUE, 1)
                    bv = rb.ObjectIdentifier(rb.ObjectType.BINARY_VALUE, 1)
                    self.assertEqual(await client.read_property(address, av, rb.PropertyIdentifier.AUDIT_PRIORITY_FILTER), rb.PropertyValue.null())
                    self.assertEqual(await client.read_property(address, av, rb.PropertyIdentifier.AUDITABLE_OPERATIONS), rb.PropertyValue.bit_string(6, b"\xc0"))
                    self.assertEqual(await client.read_property(address, bv, rb.PropertyIdentifier.AUDIT_PRIORITY_FILTER), rb.PropertyValue.bit_string(0, b"\x81\0"))
                    with self.assertRaises(rb.BacnetProtocolError):
                        await client.read_property(address, bv, rb.PropertyIdentifier.AUDIT_LEVEL)
            finally:
                if endpoint: await owner.close()
                else: await owner.stop()

    async def test_wire_object_override_and_mandatory_changes_reach_installed_sink(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = rb.BACnetServer(8120, interface="127.0.0.1", port=0)
            parent.add_audit_log(1, "sink", str(Path(directory) / "audit"), buffer_size=16)
            parent.configure_audit_notification_sink(1, policy="allow_all")
            child = rb.BACnetServer(8121, interface="127.0.0.1", port=0)
            child.add_audit_reporter(1, "reporter")
            child.configure_audit_reporters([{"instance": 1, 'audit_level': "audit_all", 'auditable_operations': 0, 'issue_confirmed_notifications': True}])
            child.add_analog_value(1, "av", audit_level="none", auditable_operations=2)
            child.add_binary_value(1, "bv", audit_level="audit_all", auditable_operations=2,
                                   audit_priority_filter=128)
            await parent.start()
            try:
                parent_address = await parent.local_address()
                child.add_device_binding(8120, parent_address)
                child.configure_audit_recipient({"kind": "device", "object_identifier": rb.ObjectIdentifier(rb.ObjectType.DEVICE,8120)})
                await child.start()
                try:
                    address = await child.local_address()
                    async with rb.BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=1000) as client:
                        av=rb.ObjectIdentifier(rb.ObjectType.ANALOG_VALUE,1)
                        bv=rb.ObjectIdentifier(rb.ObjectType.BINARY_VALUE,1)
                        await client.write_property(address,av,rb.PropertyIdentifier.PRESENT_VALUE,rb.PropertyValue.real(1.0)) # suppressed
                        await client.write_property(address,bv,rb.PropertyIdentifier.PRESENT_VALUE,rb.PropertyValue.enumerated(1),priority=16) # suppressed
                        await client.write_property(address,bv,rb.PropertyIdentifier.PRESENT_VALUE,rb.PropertyValue.enumerated(1),priority=8)
                        await client.write_property(address,av,rb.PropertyIdentifier.AUDIT_LEVEL,rb.PropertyValue.enumerated(1)) # AUDIT_ALL
                        await client.write_property(address,av,rb.PropertyIdentifier.PRESENT_VALUE,rb.PropertyValue.real(2.0))
                        query={"audit_log":rb.ObjectIdentifier(rb.ObjectType.AUDIT_LOG,1),"query_parameters":{"kind":"by_target","target_device_identifier":rb.ObjectIdentifier(rb.ObjectType.DEVICE,8121),"successful_actions_only":0},"requested_count":16}
                        async with asyncio.timeout(5):
                            while True:
                                result=await client.audit_log_query_typed(parent_address,query)
                                if len(result["records"])>=3: break
                                await asyncio.sleep(.01)
                        records=[entry["record"]["datum"]["audit_notification"] for entry in result["records"]]
                        self.assertEqual(len(records),3)
                        self.assertCountEqual([(r["target_object"], r["target_property"]["property_identifier"]) for r in records],
                                              [(bv, rb.PropertyIdentifier.PRESENT_VALUE), (av, rb.PropertyIdentifier.AUDIT_LEVEL), (av, rb.PropertyIdentifier.PRESENT_VALUE)])
                        self.assertEqual(next(r for r in records if r["target_object"] == bv)["target_priority"], 8)
                        self.assertTrue(all(r["operation"] == rb.AuditOperation.WRITE for r in records))
                finally: await child.stop()
            finally: await parent.stop()
