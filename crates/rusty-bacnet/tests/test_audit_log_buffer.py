"""Installed-artifact tests for an Audit Log's Buffer_Size writes and purge (#1238)."""

from __future__ import annotations

import asyncio
import inspect
import tempfile
import unittest
from pathlib import Path
from typing import TYPE_CHECKING, Any, cast

from rusty_bacnet import (
    AuditOperation,
    BACnetClient,
    BACnetServer,
    BacnetProtocolError,
    ErrorClass,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

if TYPE_CHECKING:
    from rusty_bacnet import AuditLogQueryRequestInput, AuditNotificationInput

AUDIT_LOG = ObjectIdentifier(ObjectType.AUDIT_LOG, 1)


def device(instance: int) -> dict[str, Any]:
    return {"kind": "device", "object_identifier": ObjectIdentifier(ObjectType.DEVICE, instance)}


def report(comment: str) -> AuditNotificationInput:
    return cast("AuditNotificationInput", {
        "source_device": device(1),
        "operation": AuditOperation.WRITE,
        "target_device": device(2),
        "source_comment": comment,
    })


def query() -> AuditLogQueryRequestInput:
    return cast("AuditLogQueryRequestInput", {
        "audit_log": AUDIT_LOG,
        "query_parameters": {
            "kind": "by_target",
            "target_device_identifier": ObjectIdentifier(ObjectType.DEVICE, 2),
            "successful_actions_only": 0,
        },
        "requested_count": 10,
    })


def logger(storage: str) -> BACnetServer:
    server = BACnetServer(
        device_instance=1238, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
    )
    server.add_audit_log(1, "Audit", storage, buffer_size=10)
    server.configure_audit_notification_sink(1, policy="allow_all")
    return server


class AuditLogBufferTests(unittest.TestCase):
    def test_purge_signature(self) -> None:
        parameters = list(inspect.signature(BACnetServer.purge_audit_log).parameters)
        self.assertEqual(parameters, ["self", "object_id"])

    def test_buffer_size_writes_and_purge(self) -> None:
        asyncio.run(asyncio.wait_for(self._exercise(), 15))

    async def _exercise(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            storage = str(Path(directory) / "audit")
            server = logger(storage)
            await server.start()
            try:
                address = await server.local_address()
                async with BACnetClient(
                    interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                    apdu_timeout_ms=2_000,
                ) as client:
                    async def read(property_id: PropertyIdentifier) -> Any:
                        return (await client.read_property(address, AUDIT_LOG, property_id)).value

                    async def refused(
                        property_id: PropertyIdentifier, value: PropertyValue,
                    ) -> BacnetProtocolError:
                        with self.assertRaises(BacnetProtocolError) as raised:
                            await client.write_property(address, AUDIT_LOG, property_id, value)
                        return raised.exception

                    for comment in ("one", "two", "three"):
                        await client.confirmed_audit_notification_typed(
                            address, {"notifications": [report(comment)]},
                        )
                    self.assertEqual(len((await client.audit_log_query_typed(address, query()))["records"]), 3)

                    # Buffer_Size is written only while logging is off.
                    error = await refused(PropertyIdentifier.BUFFER_SIZE, PropertyValue.unsigned(2))
                    self.assertEqual(error.error_code, ErrorCode.WRITE_ACCESS_DENIED.to_raw())
                    await client.write_property(
                        address, AUDIT_LOG, PropertyIdentifier.ENABLE, PropertyValue.boolean(False),
                    )
                    await client.write_property(
                        address, AUDIT_LOG, PropertyIdentifier.BUFFER_SIZE, PropertyValue.unsigned(2),
                    )
                    self.assertEqual(await read(PropertyIdentifier.BUFFER_SIZE), 2)
                    # The newest two records stay: the third report and the
                    # log-disabled status record.
                    self.assertEqual(await read(PropertyIdentifier.RECORD_COUNT), 2)
                    records = (await client.audit_log_query_typed(address, query()))["records"]
                    self.assertEqual([record["sequence_number"] for record in records], [3])

                    # No peer purges an Audit Log.
                    error = await refused(PropertyIdentifier.RECORD_COUNT, PropertyValue.unsigned(0))
                    self.assertEqual(error.error_code, ErrorCode.WRITE_ACCESS_DENIED.to_raw())

                    await server.purge_audit_log(AUDIT_LOG)
                    self.assertEqual(await read(PropertyIdentifier.RECORD_COUNT), 1)
                    self.assertEqual(await read(PropertyIdentifier.TOTAL_RECORD_COUNT), 5)
                    self.assertEqual((await client.audit_log_query_typed(address, query()))["records"], [])

                for object_id, error_class, error_code in (
                    (ObjectIdentifier(ObjectType.AUDIT_LOG, 9), ErrorClass.OBJECT, ErrorCode.UNKNOWN_OBJECT),
                    (
                        ObjectIdentifier(ObjectType.DEVICE, 1238),
                        ErrorClass.OBJECT,
                        ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                    ),
                ):
                    with self.subTest(object_id=object_id):
                        with self.assertRaises(BacnetProtocolError) as raised:
                            await server.purge_audit_log(object_id)
                        self.assertEqual(raised.exception.error_class, error_class.to_raw())
                        self.assertEqual(raised.exception.error_code, error_code.to_raw())
            finally:
                await server.stop()
            with self.assertRaises(RuntimeError):
                await server.purge_audit_log(AUDIT_LOG)

            # Reopened, the log keeps the written size, not the configured one.
            reopened = logger(storage)
            await reopened.start()
            try:
                size = await reopened.read_property(AUDIT_LOG, PropertyIdentifier.BUFFER_SIZE)
                self.assertEqual(size.value, 2)
                count = await reopened.read_property(AUDIT_LOG, PropertyIdentifier.RECORD_COUNT)
                self.assertEqual(count.value, 1)
            finally:
                await reopened.stop()


if __name__ == "__main__":
    unittest.main()
