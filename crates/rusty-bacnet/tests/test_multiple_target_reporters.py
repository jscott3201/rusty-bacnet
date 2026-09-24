"""Installed native plural target configuration, arbitration and per-object status."""
import asyncio
import tempfile
import unittest
from pathlib import Path

import rusty_bacnet as rb


def configuration(instance, selectors=None, operations=2, confirmed=False):
    return dict(instance=instance, audit_level="audit_all", auditable_operations=operations,
                issue_confirmed_notifications=confirmed, monitored_objects=selectors)


class MultipleTargetReporters(unittest.IsolatedAsyncioTestCase):
    async def test_complete_list_validation_is_atomic_and_owned(self):
        server = rb.BACnetServer(8, interface="127.0.0.1", port=0)
        self.assertFalse(hasattr(server, "configure_audit_reporter"))
        for instance in (1, 2):
            server.add_audit_reporter(instance, f"Reporter {instance}")
        server.configure_audit_recipient(dict(kind="device", object_identifier=rb.ObjectIdentifier(rb.ObjectType.DEVICE, 9)))
        selectors = [rb.ObjectType.BINARY_VALUE]
        values = [configuration(2, []), configuration(1, selectors)]
        server.configure_audit_reporters(values)
        values[1]["audit_level"] = "none"
        selectors.clear()
        for invalid in ([], [configuration(1), configuration(1)], [configuration(1)] * 65,
                        [configuration(1), configuration(99)],
                        [configuration(1), {**configuration(2), "audit_level": "default"}],
                        [configuration(1), {**configuration(2), "audit_priority_filter": 65536}],
                        [configuration(1), {**configuration(2), "unexpected": True}]):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                server.configure_audit_reporters(invalid)
        for invalid in (None, (), {}, [None], [configuration(1), {**configuration(2), "issue_confirmed_notifications": 1}]):
            with self.subTest(invalid=invalid), self.assertRaises(TypeError):
                server.configure_audit_reporters(invalid)
        await server.start()
        try:
            for instance in (1, 2):
                oid = rb.ObjectIdentifier(rb.ObjectType.AUDIT_REPORTER, instance)
                self.assertEqual((await server.read_property(oid, rb.PropertyIdentifier.AUDIT_LEVEL)).value, 1)
            oid = rb.ObjectIdentifier(rb.ObjectType.AUDIT_REPORTER, 1)
            # The caller's selector clear did not turn the selected object into
            # an empty collection. The property is a list with one encoded item.
            self.assertEqual(len((await server.read_property(oid, rb.PropertyIdentifier.MONITORED_OBJECTS)).value), 1)
            with self.assertRaises(RuntimeError):
                server.configure_audit_reporters([configuration(1)])
        finally:
            await server.stop()

    async def test_real_bip_plural_election_filters_health_and_exact_records(self):
        for overlap, confirmed in ((True, False), (True, True), (False, False), (False, True)):
            with self.subTest(overlap=overlap, confirmed=confirmed), tempfile.TemporaryDirectory() as directory:
                parent = rb.BACnetServer(9, interface="127.0.0.1", port=0)
                parent.add_audit_log(7, "Receiver", str(Path(directory) / "audit"), buffer_size=20)
                parent.configure_audit_notification_sink(7, policy="allow_all")
                child = rb.BACnetServer(8, interface="127.0.0.1", port=0)
                first = rb.ObjectIdentifier(rb.ObjectType.BINARY_VALUE, 1)
                second = rb.ObjectIdentifier(rb.ObjectType.BINARY_VALUE, 2)
                for instance in (1, 2):
                    child.add_audit_reporter(instance, f"Reporter {instance}")
                    child.add_binary_value(instance, f"Value {instance}")
                child.configure_audit_recipient(dict(kind="device", object_identifier=rb.ObjectIdentifier(rb.ObjectType.DEVICE, 9)))
                # In an overlap, the lower instance filters WRITE and prevents
                # the higher matching Reporter from substituting its own policy.
                child.configure_audit_reporters([
                    configuration(2, [rb.ObjectType.BINARY_VALUE] if overlap else [second], 2, confirmed),
                    configuration(1, [first], 0 if overlap else 2, confirmed),
                ])
                await parent.start()
                try:
                    parent_address = await parent.local_address()
                    child.add_device_binding(9, parent_address)
                    await child.start()
                    try:
                        for instance in (1, 2):
                            oid = rb.ObjectIdentifier(rb.ObjectType.AUDIT_REPORTER, instance)
                            self.assertEqual((await child.read_property(oid, rb.PropertyIdentifier.RELIABILITY)).value, 10 if overlap else 0)
                        async with rb.BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000) as client:
                            address = await child.local_address()
                            for target in (first, second):
                                await client.write_property(address, target, rb.PropertyIdentifier.PRESENT_VALUE, rb.PropertyValue.enumerated(1))
                            query = dict(audit_log=rb.ObjectIdentifier(rb.ObjectType.AUDIT_LOG, 7),
                                         query_parameters=dict(kind="by_target", target_device_identifier=rb.ObjectIdentifier(rb.ObjectType.DEVICE, 8), successful_actions_only=0), requested_count=20)
                            expected = 1 if overlap else 2
                            async with asyncio.timeout(5):
                                while True:
                                    records = (await client.audit_log_query_typed(parent_address, query))["records"]
                                    if len(records) >= expected:
                                        break
                                    await asyncio.sleep(0.01)
                            self.assertEqual(len(records), expected)
                            notifications = [record["record"]["datum"]["audit_notification"] for record in records]
                            self.assertEqual({record["target_object"] for record in notifications}, {second} if overlap else {first, second})
                            self.assertTrue(all(record["operation"] == rb.AuditOperation.WRITE and record["result"] is None for record in notifications))
                    finally:
                        await child.stop()
                finally:
                    await parent.stop()
