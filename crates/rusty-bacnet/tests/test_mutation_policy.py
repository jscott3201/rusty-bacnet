"""Installed native policy transfer, with real B/IP service-family controls."""

import ast
import asyncio
import inspect
from pathlib import Path
import unittest

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


class MutationPolicyConfigurationTests(unittest.TestCase):
    def test_runtime_and_installed_stub_contract(self):
        parameter = inspect.signature(BACnetServer).parameters["mutation_policy"]
        self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertEqual(parameter.default, "permissive")
        tree = ast.parse(Path(rusty_bacnet.__file__).with_suffix(".pyi").read_text())
        server = next(
            n for n in tree.body
            if isinstance(n, ast.ClassDef) and n.name == "BACnetServer"
        )
        constructor = next(
            n for n in server.body
            if isinstance(n, ast.FunctionDef) and n.name == "__init__"
        )
        index = next(
            i for i, arg in enumerate(constructor.args.kwonlyargs)
            if arg.arg == "mutation_policy"
        )
        self.assertEqual(
            ast.unparse(constructor.args.kwonlyargs[index].annotation),
            "Literal['permissive', 'deny_all']",
        )
        self.assertEqual(ast.literal_eval(constructor.args.kw_defaults[index]), "permissive")

    def test_invalid_mode_precedes_transport_preparation(self):
        for transport in ("bip", "ipv6", "sc", "mstp"):
            for mode in ("", "allow_all", "DenyAll"):
                with self.subTest(transport=transport, mode=mode):
                    # Missing SC credentials / MS/TP device must not mask mode validation.
                    with self.assertRaisesRegex(ValueError, "mutation_policy"):
                        BACnetServer(503001, transport=transport, mutation_policy=mode)
            for mode in (None, True, 1, lambda: True):
                with self.subTest(transport=transport, wrong_type=type(mode)):
                    with self.assertRaises(TypeError):
                        BACnetServer(503001, transport=transport, mutation_policy=mode)
        server = BACnetServer(503001, mutation_policy="deny_all", port=0)
        server.add_analog_value(1, "valid after rejection")


class MutationPolicyWireTests(unittest.IsolatedAsyncioTestCase):
    async def test_default_and_explicit_permissive_write_controls(self):
        for options in ({}, {"mutation_policy": "permissive"}):
            with self.subTest(options=options):
                server = BACnetServer(503001, interface="127.0.0.1", port=0, **options)
                server.add_analog_value(1, "AV")
                await server.start()
                try:
                    async with BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000) as client:
                        target = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
                        value = PropertyValue.real(42.0)
                        async with asyncio.timeout(5):
                            await client.write_property(await server.local_address(), target, PropertyIdentifier.PRESENT_VALUE, value)
                            self.assertEqual(await server.read_property(target, PropertyIdentifier.PRESENT_VALUE), value)
                finally:
                    await server.stop()

    async def test_deny_all_preserves_service_families_and_local_control(self):
        server = BACnetServer(503002, interface="127.0.0.1", port=0, mutation_policy="deny_all")
        server.add_analog_value(1, "AV")
        server.add_multistate_input(2, "MSI", number_of_states=3)
        server.add_file(3, "File")
        server.set_file_data(3, b"sentinel")
        av = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
        msi = ObjectIdentifier(ObjectType.MULTI_STATE_INPUT, 2)
        file = ObjectIdentifier(ObjectType.FILE, 3)
        pv = PropertyIdentifier.PRESENT_VALUE
        alarms = PropertyIdentifier.ALARM_VALUES
        await server.start()
        try:
            async with BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000) as client:
                address = await server.local_address()
                async with asyncio.timeout(15):
                    before_value = await client.read_property(address, av, pv)
                    before_list = await server.read_property(msi, alarms)
                    before_file = await client.atomic_read_file(address, file, "stream", requested_octet_count=64)
                    self.assertIn(b"sentinel", before_file)
                    operations = (
                        ("property", lambda: client.write_property(address, av, pv, PropertyValue.real(42.0))),
                        ("object", lambda: client.delete_object(address, av)),
                        ("list", lambda: client.add_list_element(address, msi, alarms, b"\x21\x02")),
                        ("file", lambda: client.atomic_write_file(address, file, "stream", file_data=b"changed!")),
                        ("cov", lambda: client.subscribe_cov(address, 7, av, False, lifetime=60)),
                    )
                    for family, operation in operations:
                        with self.subTest(family=family):
                            with self.assertRaises(BacnetProtocolError) as caught:
                                await operation()
                            self.assertEqual(caught.exception.error_class, ErrorClass.SERVICES.to_raw())
                            self.assertEqual(caught.exception.error_code, ErrorCode.SERVICE_REQUEST_DENIED.to_raw())
                    self.assertEqual(await client.read_property(address, av, pv), before_value)
                    self.assertEqual(await server.read_property(msi, alarms), before_list)
                    self.assertEqual(await client.atomic_read_file(address, file, "stream", requested_octet_count=64), before_file)
                    local_value = PropertyValue.real(17.0)
                    await server.write_property_local(av, pv, local_value)
                    self.assertEqual(await client.read_property(address, av, pv), local_value)
        finally:
            await server.stop()
