"""The endpoint owners' ``min_request_interval_ms`` keyword (#1542).

Each owner takes it keyword-only, defaults to 0 and validates it at
construction exactly as ``BACnetClient`` does. A real B/IP endpoint client
then spaces its reads of one device by at least the interval.
"""

from __future__ import annotations

import asyncio
import inspect
import time
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BipEndpoint,
    MstpEndpoint,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    ScEndpoint,
)

HOUR_MS = 3_600_000

# No I/O happens in a constructor, so placeholder addresses and paths serve.
OWNERS: list[tuple[type, dict[str, Any]]] = [
    (BipEndpoint, {"port": 0}),
    (
        ScEndpoint,
        {
            "sc_hub": "wss://localhost:1",
            "sc_vmac": b"\x02\x00\x00\x00\x00\x01",
            "sc_ca_cert": "ca.pem",
            "sc_client_cert": "cert.pem",
            "sc_client_key": "key.pem",
            "sc_device_uuid": bytes.fromhex("8e62ac46d7084226913776a32b619315"),
        },
    ),
    (MstpEndpoint, {"serial_port": "/tmp/nonexistent"}),
]


def client_refusal(value: int) -> tuple[type, str]:
    """The exception ``BACnetClient`` raises for ``value``."""
    try:
        BACnetClient(interface="127.0.0.1", port=0, min_request_interval_ms=value)
    except Exception as error:  # noqa: BLE001 - the type is what's compared
        return type(error), str(error)
    raise AssertionError(f"BACnetClient accepted {value}")


class RequestIntervalConstructorTests(unittest.TestCase):
    def test_default_keyword_only_and_stub(self) -> None:
        stub = Path(rusty_bacnet.__file__).with_suffix(".pyi").read_text(encoding="utf-8")
        # BACnetClient's and each owner's.
        self.assertEqual(stub.count("min_request_interval_ms: int = 0"), 1 + len(OWNERS))
        for cls, _ in OWNERS:
            with self.subTest(owner=cls.__name__):
                parameter = inspect.signature(cls).parameters["min_request_interval_ms"]
                self.assertEqual(parameter.default, 0)
                self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)

    def test_validated_as_the_client_validates_it(self) -> None:
        for cls, args in OWNERS:
            with self.subTest(owner=cls.__name__):
                cls(123, **args, min_request_interval_ms=0)
                cls(123, **args, min_request_interval_ms=HOUR_MS)
                for value in (HOUR_MS + 1, -1, 1 << 64):
                    error_type, message = client_refusal(value)
                    with self.subTest(owner=cls.__name__, value=value):
                        with self.assertRaises(error_type) as refused:
                            cls(123, **args, min_request_interval_ms=value)
                        self.assertEqual(str(refused.exception), message)
                with self.assertRaisesRegex(ValueError, "min_request_interval_ms"):
                    cls(123, **args, min_request_interval_ms=HOUR_MS + 1)
                with self.assertRaises(TypeError):
                    cls(123, **args, min_request_interval_ms=1.5)


class RequestIntervalWireTests(unittest.IsolatedAsyncioTestCase):
    async def test_reads_of_one_device_are_spaced_by_the_interval(self) -> None:
        target = BACnetServer(1_542_000, interface="127.0.0.1", port=0)
        target.add_analog_input(1, "AI-1", present_value=21.5)
        endpoint = BipEndpoint(
            device_instance=1_542_001,
            interface="127.0.0.1",
            port=0,
            min_request_interval_ms=150,
        )
        await target.start()
        await endpoint.start()
        try:
            role = await endpoint.client()
            address = await target.local_address()
            oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
            started = time.monotonic()
            async with asyncio.timeout(10):
                for _ in range(3):
                    await role.read_property(address, oid, PropertyIdentifier.PRESENT_VALUE)
            elapsed = time.monotonic() - started
            # Each read after the first waits at least 150 ms from the answer
            # to the one before it.
            self.assertGreaterEqual(elapsed, 2 * 0.15)
        finally:
            await endpoint.close()
            await target.stop()


if __name__ == "__main__":
    unittest.main()
