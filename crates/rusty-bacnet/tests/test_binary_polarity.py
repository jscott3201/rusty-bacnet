"""Polarity and the physical side of Binary Input and Output (#2580)."""

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
    BipEndpoint,
    ErrorClass,
    ErrorCode,
    MstpEndpoint,
    ObjectIdentifier,
    ObjectType,
    Polarity,
    PropertyIdentifier,
    PropertyValue,
    ScEndpoint,
)

NOTIFICATION_TIMEOUT = 2.0
INACTIVE = PropertyValue.enumerated(0)
ACTIVE = PropertyValue.enumerated(1)
BI = ObjectIdentifier(ObjectType.BINARY_INPUT, 2)
BO = ObjectIdentifier(ObjectType.BINARY_OUTPUT, 3)
NORMAL_BI = ObjectIdentifier(ObjectType.BINARY_INPUT, 4)
HELD_BO = ObjectIdentifier(ObjectType.BINARY_OUTPUT, 5)
# Every int but NORMAL and REVERSE, however far out, is VALUE_OUT_OF_RANGE.
OUT_OF_RANGE = (2, -1, 2**32, -(2**70), 2**70, Polarity.from_raw(7))


def stub_methods() -> dict[str, ast.FunctionDef]:
    stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
    server = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == "BACnetServer"
    )
    return {
        node.name: node for node in server.body if isinstance(node, ast.FunctionDef)
    }


def make_server() -> BACnetServer:
    server = BACnetServer(
        device_instance=503_080,
        device_name="Binary Polarity Test",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_binary_input(2, "BI-2", polarity=1)
    server.add_binary_output(3, "BO-3", polarity=Polarity.REVERSE)
    server.add_binary_input(4, "BI-4")
    server.add_binary_output(5, "BO-5", polarity=Polarity.REVERSE)
    return server


class BinaryPolarityTests(unittest.TestCase):
    def test_runtime_and_stub_agree(self) -> None:
        stubs = stub_methods()
        for name, parameters, returns in (
            ("set_physical_input_local", ["object_id", "on"], "Awaitable[None]"),
            ("set_polarity_local", ["object_id", "polarity"], "Awaitable[None]"),
            ("physical_output_local", ["object_id"], "Awaitable[bool]"),
        ):
            with self.subTest(method=name):
                runtime = list(inspect.signature(getattr(BACnetServer, name)).parameters)
                self.assertEqual(runtime, ["self", *parameters])
                stub = stubs[name]
                self.assertEqual(
                    [argument.arg for argument in stub.args.args], ["self", *parameters]
                )
                assert stub.returns is not None
                self.assertEqual(ast.unparse(stub.returns), returns)
        for name in ("add_binary_input", "add_binary_output"):
            with self.subTest(builder=name):
                parameter = inspect.signature(getattr(BACnetServer, name)).parameters[
                    "polarity"
                ]
                self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertEqual(parameter.default, 0)
                stub = stubs[name]
                keywords = {
                    argument.arg: ast.unparse(argument.annotation)
                    for argument in stub.args.kwonlyargs
                    if argument.annotation is not None
                }
                self.assertEqual(keywords["polarity"], "Polarity | int")
        set_polarity = stubs["set_polarity_local"].args.args[2]
        assert set_polarity.annotation is not None
        self.assertEqual(ast.unparse(set_polarity.annotation), "Polarity | int")

    def test_builders_refuse_a_polarity_past_reverse(self) -> None:
        server = make_server()
        for add in (server.add_binary_input, server.add_binary_output):
            for polarity in OUT_OF_RANGE:
                with self.subTest(builder=add.__name__, polarity=polarity):
                    with self.assertRaises(BacnetProtocolError) as raised:
                        add(9, f"{add.__name__}-9", polarity=polarity)
                    self.assertEqual(
                        raised.exception.error_code,
                        ErrorCode.VALUE_OUT_OF_RANGE.to_raw(),
                    )
            for polarity in (True, "1", 1.0):
                with self.subTest(builder=add.__name__, polarity=polarity):
                    with self.assertRaises(TypeError):
                        add(9, f"{add.__name__}-9", polarity=polarity)

    def test_endpoint_binary_input_takes_polarity(self) -> None:
        for cls in (BipEndpoint, ScEndpoint, MstpEndpoint):
            with self.subTest(endpoint=cls.__name__):
                parameter = inspect.signature(cls.add_binary_input).parameters["polarity"]
                self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertEqual(parameter.default, 0)
        endpoint = BipEndpoint(
            device_instance=503_081,
            vendor_id=42,
            interface="127.0.0.1",
            port=0,
            broadcast_address="127.0.0.1",
        )
        endpoint.add_binary_input(1, "BI-1", polarity=Polarity.REVERSE)
        endpoint.add_binary_input(2, "BI-2", polarity=0)
        for polarity in OUT_OF_RANGE:
            with self.subTest(polarity=polarity):
                with self.assertRaises(BacnetProtocolError) as raised:
                    endpoint.add_binary_input(3, "BI-3", polarity=polarity)
                self.assertEqual(
                    raised.exception.error_code, ErrorCode.VALUE_OUT_OF_RANGE.to_raw()
                )

    def test_live_server_applies_polarity_on_the_physical_side(self) -> None:
        asyncio.run(self._exercise_live_server())

    async def _exercise_live_server(self) -> None:
        server = make_server()
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
                # Polarity reads as configured over the wire and refuses a write.
                for oid, expected in ((BI, 1), (BO, 1), (NORMAL_BI, 0), (HELD_BO, 1)):
                    self.assertEqual(
                        await client.read_property(
                            address, oid, PropertyIdentifier.POLARITY
                        ),
                        PropertyValue.enumerated(expected),
                    )
                with self.assertRaises(BacnetProtocolError) as raised:
                    await client.write_property(
                        address,
                        BI,
                        PropertyIdentifier.POLARITY,
                        PropertyValue.enumerated(0),
                    )
                self.assertEqual(
                    raised.exception.error_code,
                    ErrorCode.WRITE_ACCESS_DENIED.to_raw(),
                )

                iterator = await client.cov_notifications()

                async def collect() -> None:
                    async for notification in iterator:
                        notifications.put_nowait(notification)

                listener = asyncio.create_task(collect())
                await client.subscribe_cov(
                    address,
                    subscriber_process_identifier=1,
                    monitored_object_identifier=BI,
                    confirmed=False,
                    lifetime=60,
                )
                await asyncio.wait_for(notifications.get(), timeout=NOTIFICATION_TIMEOUT)

                # An OFF input under REVERSE is ACTIVE, and subscribers hear
                # the logical state.
                await server.set_physical_input_local(BI, False)
                self.assertEqual(
                    await client.read_property(
                        address, BI, PropertyIdentifier.PRESENT_VALUE
                    ),
                    ACTIVE,
                )
                notification = await asyncio.wait_for(
                    notifications.get(), timeout=NOTIFICATION_TIMEOUT
                )
                self.assertIn(
                    ACTIVE,
                    [
                        item["value"]
                        for item in notification.values
                        if item["property_id"] == PropertyIdentifier.PRESENT_VALUE
                    ],
                )
                # A run-time Polarity change derives Present_Value again.
                await server.set_polarity_local(BI, 0)
                self.assertEqual(
                    await server.read_property(BI, PropertyIdentifier.PRESENT_VALUE),
                    INACTIVE,
                )

                # Out of service the input is kept, Present_Value isn't moved,
                # and the return to service derives it.
                await server.write_property_local(
                    BI,
                    PropertyIdentifier.OUT_OF_SERVICE,
                    PropertyValue.boolean(True),
                    source_object=None,
                )
                await server.set_physical_input_local(BI, True)
                self.assertEqual(
                    await server.read_property(BI, PropertyIdentifier.PRESENT_VALUE),
                    INACTIVE,
                )
                await server.write_property_local(
                    BI,
                    PropertyIdentifier.OUT_OF_SERVICE,
                    PropertyValue.boolean(False),
                    source_object=None,
                )
                self.assertEqual(
                    await server.read_property(BI, PropertyIdentifier.PRESENT_VALUE),
                    ACTIVE,
                )

                # The output drives the commanded value through Polarity.
                self.assertTrue(await server.physical_output_local(BO))
                await client.write_property(
                    address,
                    BO,
                    PropertyIdentifier.PRESENT_VALUE,
                    ACTIVE,
                    priority=8,
                )
                self.assertFalse(await server.physical_output_local(BO))
                await server.set_polarity_local(BO, Polarity.NORMAL)
                self.assertTrue(await server.physical_output_local(BO))

                # Out of service a REVERSE output holds the state Polarity
                # gave it: ON for the INACTIVE it had, whatever it is
                # commanded to meanwhile.
                await server.write_property_local(
                    HELD_BO,
                    PropertyIdentifier.OUT_OF_SERVICE,
                    PropertyValue.boolean(True),
                    source_object=None,
                )
                self.assertTrue(await server.physical_output_local(HELD_BO))
                await client.write_property(
                    address,
                    HELD_BO,
                    PropertyIdentifier.PRESENT_VALUE,
                    ACTIVE,
                    priority=8,
                )
                self.assertTrue(await server.physical_output_local(HELD_BO))

                unsupported = ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
                for call, code in (
                    (lambda: server.physical_output_local(BI), unsupported),
                    (lambda: server.set_physical_input_local(BO, True), unsupported),
                    *(
                        (lambda p=p: server.set_polarity_local(BI, p), ErrorCode.VALUE_OUT_OF_RANGE)
                        for p in OUT_OF_RANGE
                    ),
                ):
                    with self.assertRaises(BacnetProtocolError) as refused:
                        await call()
                    self.assertEqual(refused.exception.error_code, code.to_raw())
                self.assertEqual(
                    await server.read_property(BI, PropertyIdentifier.POLARITY),
                    PropertyValue.enumerated(0),
                )
                self.assertEqual(
                    refused.exception.error_class,
                    ErrorClass.PROPERTY.to_raw(),
                )
        finally:
            if listener is not None:
                listener.cancel()
                with contextlib.suppress(asyncio.CancelledError):
                    await listener
            await server.stop()


if __name__ == "__main__":
    unittest.main()
