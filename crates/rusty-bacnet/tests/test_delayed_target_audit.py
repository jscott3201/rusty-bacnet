"""Installed paired delay controls, whole-input authoring, and actual B/IP delivery."""
import asyncio
import ast
from pathlib import Path
import tempfile
import unittest

import rusty_bacnet as rb


def config(instance, delay=None):
    return {"instance": instance, "audit_level": "audit_all", "auditable_operations": 2,
            "issue_confirmed_notifications": True, "maximum_send_delay": delay}


class DelayedTargetAuditTests(unittest.IsolatedAsyncioTestCase):
    async def test_pair_presence_and_invalid_late_configuration_are_atomic(self):
        owner = rb.BACnetServer(8491, interface="127.0.0.1", port=0)
        for instance in (1, 2, 3):
            owner.add_audit_reporter(instance, f"reporter-{instance}")
        owner.configure_audit_reporters([config(1, 3600), config(2, 0), config(3)])
        for invalid, exception in [(-1, ValueError), (3601, ValueError), (1 << 64, ValueError),
                                   (True, TypeError), (1.5, TypeError), ("1", TypeError)]:
            with self.subTest(invalid=invalid):
                with self.assertRaises(exception):
                    owner.configure_audit_reporters([config(1, 1), config(2, invalid)])
        owner.configure_audit_recipient({"kind": "device", "object_identifier": rb.ObjectIdentifier(rb.ObjectType.DEVICE,8490)})
        await owner.start()  # Unresolved Device remains an explicit unhealthy configuration.
        try:
            address = await owner.local_address()
            async with rb.BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=1000) as client:
                for instance, delay in ((1, 3600), (2, 0)):
                    target = rb.ObjectIdentifier(rb.ObjectType.AUDIT_REPORTER, instance)
                    self.assertEqual(await client.read_property(address, target, rb.PropertyIdentifier.MAXIMUM_SEND_DELAY), rb.PropertyValue.unsigned(delay))
                    self.assertEqual(await client.read_property(address, target, rb.PropertyIdentifier.SEND_NOW), rb.PropertyValue.boolean(False))
                with self.assertRaises(rb.BacnetProtocolError):
                    await client.read_property(address, rb.ObjectIdentifier(rb.ObjectType.AUDIT_REPORTER,3), rb.PropertyIdentifier.SEND_NOW)
        finally:
            await owner.stop()
        stub = ast.parse(Path(rb.__file__).with_suffix(".pyi").read_text())
        cls = next(node for node in stub.body if isinstance(node, ast.ClassDef) and node.name == "AuditReporterConfiguration")
        field = next(node for node in cls.body if isinstance(node, ast.AnnAssign) and node.target.id == "maximum_send_delay")
        self.assertEqual(ast.unparse(field.annotation), "NotRequired[int | None]")

    async def test_send_now_flushes_delayed_records_and_zero_delay_is_immediate(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = rb.BACnetServer(8490, interface="127.0.0.1", port=0)
            parent.add_audit_log(1, "sink", str(Path(directory) / "audit"), buffer_size=32)
            parent.configure_audit_notification_sink(1, policy="allow_all")
            child = rb.BACnetServer(8491, interface="127.0.0.1", port=0)
            child.add_audit_reporter(1, "reporter")
            child.configure_audit_reporters([config(1, 3600)])
            child.add_binary_value(1, "value")
            await parent.start()
            try:
                destination = await parent.local_address()
                child.add_device_binding(8490, destination)
                child.configure_audit_recipient({"kind": "device", "object_identifier": rb.ObjectIdentifier(rb.ObjectType.DEVICE,8490)})
                await child.start()
                try:
                    address = await child.local_address()
                    async with rb.BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=1000) as client:
                        target = rb.ObjectIdentifier(rb.ObjectType.BINARY_VALUE,1)
                        reporter = rb.ObjectIdentifier(rb.ObjectType.AUDIT_REPORTER,1)
                        query = {"audit_log": rb.ObjectIdentifier(rb.ObjectType.AUDIT_LOG,1), "query_parameters": {"kind": "by_target", "target_device_identifier": rb.ObjectIdentifier(rb.ObjectType.DEVICE,8491), "successful_actions_only": 0}, "requested_count": 32}
                        async def records():
                            result = await client.audit_log_query_typed(destination, query)
                            return [entry["record"]["datum"]["audit_notification"] for entry in result["records"]]
                        self.assertEqual(await records(), [], "startup emits no Audit traffic")
                        for value in (1, 0):
                            await client.write_property(address, target, rb.PropertyIdentifier.PRESENT_VALUE, rb.PropertyValue.enumerated(value))
                        self.assertEqual(await records(), [], "long delay retains the ordinary prefix")
                        await client.write_property(address, reporter, rb.PropertyIdentifier.SEND_NOW, rb.PropertyValue.boolean(True))
                        async with asyncio.timeout(5):
                            while len(found := await records()) < 3:
                                await asyncio.sleep(.01)
                        self.assertEqual(len(found), 3)
                        self.assertEqual(sum(record["target_object"] == target for record in found), 2)
                        self.assertEqual(sum(record["target_property"]["property_identifier"] == rb.PropertyIdentifier.SEND_NOW for record in found), 1)
                        self.assertEqual(await client.read_property(address, reporter, rb.PropertyIdentifier.SEND_NOW), rb.PropertyValue.boolean(False))
                        await client.write_property(address, reporter, rb.PropertyIdentifier.MAXIMUM_SEND_DELAY, rb.PropertyValue.unsigned(0))
                        await client.write_property(address, target, rb.PropertyIdentifier.PRESENT_VALUE, rb.PropertyValue.enumerated(1))
                        async with asyncio.timeout(5):
                            while len(found := await records()) < 5:
                                await asyncio.sleep(.01)
                        self.assertEqual(len(found), 5, "internal command reset emits no WRITE")
                finally:
                    await child.stop()
            finally:
                await parent.stop()
