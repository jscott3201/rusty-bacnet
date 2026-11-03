"""A B/IP endpoint's source Audit Reporter (#2348): the session records the
ReinitializeDevice operations it carries out and report_device_reset_local()
calls through it, filtered by the Reporter's level and operations, and
delivers them to the Device's Audit recipient (here, a server's Audit Log)."""
import asyncio
import tempfile
import unittest
from pathlib import Path

import rusty_bacnet as rb

O = rb.ObjectType
ENDPOINT = 2348
SINK = 2349
REPORTER = rb.ObjectIdentifier(O.AUDIT_REPORTER, 1)
SINK_DEVICE = rb.ObjectIdentifier(O.DEVICE, SINK)
RESET = 1 << rb.AuditOperation.DEVICE_RESET.to_raw()
BACKUP = 1 << rb.AuditOperation.DEVICE_BACKUP.to_raw()


def endpoint(destination, **overrides):
    """A B/IP endpoint reporting to the sink Device at `destination`."""
    keywords = {
        "interface": "127.0.0.1", "port": 0, "broadcast_address": "127.255.255.255",
        "on_reinitialize": lambda request: None, "device_write_policy": lambda context: True,
        "source_audit_reporter": REPORTER,
        "audit_recipient": {"kind": "device", "object_identifier": SINK_DEVICE},
        "audit_device_bindings": [(SINK_DEVICE, destination)],
    }
    keywords.update(overrides)
    return rb.BipEndpoint(ENDPOINT, **keywords)


class EndpointSourceAuditTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.sink = rb.BACnetServer(SINK, interface="127.0.0.1", port=0)
        self.sink.add_audit_log(1, "sink", str(Path(directory.name) / "audit"), buffer_size=32)
        self.sink.configure_audit_notification_sink(1, policy="allow_all")
        await self.sink.start()

        async def stop_sink():
            await self.sink.stop()
        self.addAsyncCleanup(stop_sink)
        self.destination = await self.sink.local_address()
        self.client = rb.BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await self.client.__aenter__()

        async def stop_client():
            await self.client.stop()
        self.addAsyncCleanup(stop_client)

    async def records(self):
        query = {"audit_log": rb.ObjectIdentifier(O.AUDIT_LOG, 1),
                 "query_parameters": {
                     "kind": "by_target", "successful_actions_only": 0,
                     "target_device_identifier": rb.ObjectIdentifier(O.DEVICE, ENDPOINT)},
                 "requested_count": 32}
        result = await self.client.audit_log_query_typed(self.destination, query)
        return [entry["record"]["datum"]["audit_notification"] for entry in result["records"]]

    async def wait_for(self, count):
        async with asyncio.timeout(5):
            while len(found := await self.records()) < count:
                await asyncio.sleep(0.01)
        return found

    async def test_a_peer_reset_and_a_local_reset_are_each_recorded(self):
        owner = endpoint(self.destination)
        owner.add_audit_reporter(1, "Source", audit_level="audit_all",
                                 auditable_operations=RESET, issue_confirmed_notifications=True)
        async with owner:
            address = await owner.local_address()
            await self.client.reinitialize_device(address, rb.ReinitializedState.WARMSTART)
            peer = await self.wait_for(1)
            self.assertIsNone(await owner.report_device_reset_local())
            found = await self.wait_for(2)
        # The log lists the newest first: the local reset, then the peer's.
        found.reverse()
        self.assertEqual(found[0], peer[0])
        this_device = {"kind": "device",
                       "object_identifier": rb.ObjectIdentifier(O.DEVICE, ENDPOINT)}
        for record in found:
            self.assertEqual(record["operation"], rb.AuditOperation.DEVICE_RESET)
            self.assertEqual(record["target_device"], this_device)
            self.assertIsNone(record["target_object"])
        # The peer's reset names the peer and its invoke ID; the local one
        # names this device and has none.
        self.assertNotEqual(found[0]["source_device"], this_device)
        self.assertIsNotNone(found[0]["invoke_id"])
        self.assertEqual(found[1]["source_device"], this_device)
        self.assertIsNone(found[1]["invoke_id"])

    async def test_the_reporters_operations_filter_both_paths(self):
        owner = endpoint(self.destination)
        owner.add_audit_reporter(1, "Source", audit_level="audit_all",
                                 auditable_operations=BACKUP, issue_confirmed_notifications=True)
        async with owner:
            address = await owner.local_address()
            await self.client.reinitialize_device(address, rb.ReinitializedState.WARMSTART)
            await owner.report_device_reset_local()
            # A backup step passes the filter; it is the first record built.
            await self.client.reinitialize_device(address, rb.ReinitializedState.START_BACKUP)
            found = await self.wait_for(1)
            await asyncio.sleep(0.2)
            found = await self.records()
        self.assertEqual([record["operation"] for record in found],
                         [rb.AuditOperation.DEVICE_BACKUP])

    async def test_audit_level_none_records_nothing(self):
        owner = endpoint(self.destination)
        owner.add_audit_reporter(1, "Source", audit_level="none",
                                 auditable_operations=RESET | BACKUP,
                                 issue_confirmed_notifications=True)
        async with owner:
            address = await owner.local_address()
            await self.client.reinitialize_device(address, rb.ReinitializedState.WARMSTART)
            await owner.report_device_reset_local()
            await asyncio.sleep(0.3)
            self.assertEqual(await self.records(), [])

    async def test_bad_keywords_are_refused(self):
        destination = self.destination
        device = rb.ObjectIdentifier(O.DEVICE, 7)
        refused_at_construction = [
            {"source_audit_reporter": rb.ObjectIdentifier(O.ANALOG_VALUE, 1)},
            {"source_audit_reporter": rb.ObjectIdentifier(O.AUDIT_REPORTER, 4194303)},
            {"device_write_policy": None},
            {"audit_recipient": None},
            {"audit_recipient": {"kind": "device",
                                 "object_identifier": rb.ObjectIdentifier(O.DEVICE, ENDPOINT)}},
            {"audit_device_bindings": [(device, "not an address")]},
            {"audit_device_bindings": [(rb.ObjectIdentifier(O.ANALOG_VALUE, 1), destination)]},
            {"audit_device_bindings": [(device, destination), (device, destination)]},
            {"source_audit_reporter": None},
        ]
        for overrides in refused_at_construction:
            with self.subTest(overrides=overrides), self.assertRaises(ValueError):
                endpoint(destination, **overrides)
        # Whether the Reporter is in the database is known at start.
        owner = endpoint(destination)
        owner.add_analog_value(1, "Not a Reporter")
        with self.assertRaisesRegex(rb.BacnetError, "absent from the database"):
            await owner.start()
        await owner.close()
        # Without a Reporter the local report stays a no-op.
        async with rb.BipEndpoint(ENDPOINT, interface="127.0.0.1", port=0,
                                  broadcast_address="127.255.255.255") as plain:
            self.assertIsNone(await plain.report_device_reset_local())

    def test_add_audit_reporter_checks_its_keywords(self):
        owner = endpoint(self.destination)
        with self.assertRaises(ValueError):
            owner.add_audit_reporter(1, "Source", audit_level="sometimes")
        with self.assertRaises(TypeError):
            owner.add_audit_reporter(1, "Source", auditable_operations=True)
        with self.assertRaises(OverflowError):
            owner.add_audit_reporter(1, "Source", auditable_operations=-1)
        self.assertEqual(owner._pending_registration_count(), 0)


if __name__ == "__main__":
    unittest.main()
