"""Installed-artifact tests for application-owned Input Present_Value updates."""

from __future__ import annotations

import ast
import asyncio
import contextlib
import inspect
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import (
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


NOTIFICATION_TIMEOUT = 2.0
SILENCE_TIMEOUT = 0.25


def installed_stub_method() -> ast.AsyncFunctionDef:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    server = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == "BACnetServer"
    )
    method = next(
        node
        for node in server.body
        if isinstance(node, ast.AsyncFunctionDef)
        and node.name == "set_present_value_local"
    )
    return method


def annotation_text(annotation: ast.expr | None) -> str:
    assert annotation is not None
    return ast.unparse(annotation)


def make_server() -> BACnetServer:
    server = BACnetServer(
        device_instance=503_001,
        device_name="Input Present Value Artifact Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_analog_input(1, "AI-1", present_value=10.0)
    server.add_binary_input(2, "BI-2")
    server.add_multistate_input(3, "MSI-3", number_of_states=3)
    server.add_analog_value(4, "AV-4")
    return server


class InputPresentValueArtifactTests(unittest.TestCase):
    def test_runtime_and_installed_stub_expose_the_exact_async_contract(self) -> None:
        runtime_parameters = list(
            inspect.signature(BACnetServer.set_present_value_local).parameters.values()
        )
        self.assertEqual(
            [parameter.name for parameter in runtime_parameters],
            ["self", "object_id", "value"],
        )
        self.assertTrue(
            all(
                parameter.default is inspect.Parameter.empty
                for parameter in runtime_parameters
            )
        )

        method = installed_stub_method()
        stub_args = [*method.args.posonlyargs, *method.args.args]
        self.assertEqual(
            [argument.arg for argument in stub_args],
            ["self", "object_id", "value"],
        )
        self.assertEqual(
            [annotation_text(argument.annotation) for argument in stub_args[1:]],
            ["ObjectIdentifier", "PropertyValue"],
        )
        self.assertEqual(annotation_text(method.returns), "None")

        docs = ast.get_docstring(method)
        self.assertIsNotNone(docs)
        assert docs is not None
        for phrase in (
            "finite REAL",
            "Enumerated 0/1",
            "INACTIVE/ACTIVE",
            "Polarity",
            "raw hardware/interface levels",
            "Unsigned 1..=Number_Of_States",
            "Out_Of_Service",
            "network simulation ownership",
            "non-generic",
        ):
            with self.subTest(documented=phrase):
                self.assertIn(phrase, docs)

    def test_live_server_enforces_input_ownership_and_error_atomicity(self) -> None:
        asyncio.run(self._exercise_live_server())

    async def _exercise_live_server(self) -> None:
        server = make_server()
        ai = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        bi = ObjectIdentifier(ObjectType.BINARY_INPUT, 2)
        msi = ObjectIdentifier(ObjectType.MULTI_STATE_INPUT, 3)
        av = ObjectIdentifier(ObjectType.ANALOG_VALUE, 4)
        unknown = ObjectIdentifier(ObjectType.ANALOG_INPUT, 999)
        notifications: asyncio.Queue[Any] = asyncio.Queue()
        listener: asyncio.Task[None] | None = None

        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(
                interface="127.0.0.1",
                port=0,
                broadcast_address="127.0.0.1",
                apdu_timeout_ms=2_000,
            ) as client:
                iterator = await client.cov_notifications()

                async def collect_notifications() -> None:
                    async for notification in iterator:
                        notifications.put_nowait(notification)

                listener = asyncio.create_task(collect_notifications())

                for process_id, oid in enumerate((ai, bi, msi), start=1):
                    await client.subscribe_cov(
                        address,
                        subscriber_process_identifier=process_id,
                        monitored_object_identifier=oid,
                        confirmed=False,
                        lifetime=60,
                    )
                    initial = await asyncio.wait_for(
                        notifications.get(), timeout=NOTIFICATION_TIMEOUT
                    )
                    self.assertEqual(initial.monitored_object_identifier, oid)

                successful_updates = (
                    (ai, PropertyValue.real(21.5)),
                    (bi, PropertyValue.enumerated(1)),
                    (bi, PropertyValue.enumerated(0)),
                    (msi, PropertyValue.unsigned(2)),
                    (msi, PropertyValue.unsigned(3)),
                )
                for oid, value in successful_updates:
                    await server.set_present_value_local(oid, value)
                    self.assertEqual(await self._read_present_value(server, oid), value)
                    notification = await asyncio.wait_for(
                        notifications.get(), timeout=NOTIFICATION_TIMEOUT
                    )
                    self.assertEqual(notification.monitored_object_identifier, oid)
                    present_values = [
                        item["value"]
                        for item in notification.values
                        if item["property_id"] == PropertyIdentifier.PRESENT_VALUE
                    ]
                    self.assertIn(value, present_values)

                invalid_updates = (
                    (ai, PropertyValue.enumerated(1), ErrorCode.INVALID_DATA_TYPE),
                    (
                        ai,
                        PropertyValue.real(float("nan")),
                        ErrorCode.VALUE_OUT_OF_RANGE,
                    ),
                    (bi, PropertyValue.boolean(False), ErrorCode.INVALID_DATA_TYPE),
                    (bi, PropertyValue.enumerated(2), ErrorCode.VALUE_OUT_OF_RANGE),
                    (msi, PropertyValue.enumerated(1), ErrorCode.INVALID_DATA_TYPE),
                    (msi, PropertyValue.unsigned(0), ErrorCode.VALUE_OUT_OF_RANGE),
                    (msi, PropertyValue.unsigned(4), ErrorCode.VALUE_OUT_OF_RANGE),
                )
                for oid, value, code in invalid_updates:
                    await self._assert_rejected_without_mutation(
                        server, oid, value, ErrorClass.PROPERTY, code
                    )
                await self._assert_notification_silence(notifications)

                await server.write_property_local(
                    ai,
                    PropertyIdentifier.OUT_OF_SERVICE,
                    PropertyValue.boolean(True),
                )
                oos_notification = await asyncio.wait_for(
                    notifications.get(), timeout=NOTIFICATION_TIMEOUT
                )
                self.assertEqual(oos_notification.monitored_object_identifier, ai)
                await self._assert_rejected_without_mutation(
                    server,
                    ai,
                    PropertyValue.real(30.0),
                    ErrorClass.PROPERTY,
                    ErrorCode.WRITE_ACCESS_DENIED,
                )
                self.assertEqual(
                    await server.read_property(
                        ai, PropertyIdentifier.OUT_OF_SERVICE
                    ),
                    PropertyValue.boolean(True),
                )
                await self._assert_notification_silence(notifications)

                with self.assertRaises(BacnetProtocolError) as raised:
                    await server.set_present_value_local(
                        unknown, PropertyValue.real(1.0)
                    )
                self.assertEqual(
                    raised.exception.error_class, ErrorClass.OBJECT.to_raw()
                )
                self.assertEqual(
                    raised.exception.error_code, ErrorCode.UNKNOWN_OBJECT.to_raw()
                )
                await self._assert_rejected_without_mutation(
                    server,
                    av,
                    PropertyValue.real(42.0),
                    ErrorClass.OBJECT,
                    ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                )
                await self._assert_notification_silence(notifications)
        finally:
            if listener is not None:
                listener.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await listener
            await server.stop()

    async def _read_present_value(
        self, server: BACnetServer, oid: ObjectIdentifier
    ) -> PropertyValue:
        return await server.read_property(oid, PropertyIdentifier.PRESENT_VALUE)

    async def _assert_rejected_without_mutation(
        self,
        server: BACnetServer,
        oid: ObjectIdentifier,
        value: PropertyValue,
        error_class: ErrorClass,
        error_code: ErrorCode,
    ) -> None:
        before = await self._read_present_value(server, oid)
        with self.assertRaises(BacnetProtocolError) as raised:
            await server.set_present_value_local(oid, value)
        self.assertEqual(raised.exception.error_class, error_class.to_raw())
        self.assertEqual(raised.exception.error_code, error_code.to_raw())
        self.assertEqual(await self._read_present_value(server, oid), before)

    async def _assert_notification_silence(
        self, notifications: asyncio.Queue[Any]
    ) -> None:
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(
                notifications.get(), timeout=SILENCE_TIMEOUT
            )


if __name__ == "__main__":
    unittest.main()
