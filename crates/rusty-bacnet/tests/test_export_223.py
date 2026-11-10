"""The experimental Standard 223 export from Python (#2436).

``BACnetClient.export_223_experimental`` reads a device's objects into an
``Inventory223Experimental``; its ``to_turtle()`` writes the Turtle the
``bacnet export 223-experimental`` command writes. The tests run a server
in this process and reach it through a UDP relay that counts the confirmed
requests and can go silent part-way, as a device that stops answering.
"""

from __future__ import annotations

import ast
import asyncio
import inspect
import pickle
import re
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetError,
    EngineeringUnits,
    Inventory223Experimental,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

DEVICE = 2436
AI = ObjectIdentifier(ObjectType.ANALOG_INPUT, 3)
BO = ObjectIdentifier(ObjectType.BINARY_OUTPUT, 1)
MSV = ObjectIdentifier(ObjectType.MULTI_STATE_VALUE, 2)


class Relay(asyncio.DatagramProtocol):
    """Passes datagrams between one client and the server, counting the
    client's confirmed requests and dropping those after ``passing``.

    A request is counted by its invoke ID, which the client gives each
    request to a peer in turn: a retransmission of a request already
    passed is passed again and not counted, so a slow reply can't shift
    the count or the point where the relay goes silent."""

    def __init__(self, server: tuple[str, int], passing: int) -> None:
        self.server = server
        self.passing = passing
        self.passed: set[int] = set()
        self.dropped: set[int] = set()
        self.client: tuple[str, int] | None = None
        self.front: asyncio.DatagramTransport | None = None
        self.back: asyncio.DatagramTransport | None = None

    def datagram_received(self, data: bytes, addr: tuple[str, int]) -> None:
        self.client = addr
        # BVLL (4 octets), NPDU version and control, then the APDU: a local
        # confirmed request has no NPDU addresses, and its invoke ID is the
        # APDU's third octet.
        if len(data) > 8 and data[5] & 0x28 == 0 and data[6] >> 4 == 0:
            invoke_id = data[8]
            if invoke_id not in self.passed:
                if len(self.passed) >= self.passing:
                    self.dropped.add(invoke_id)
                    return
                self.passed.add(invoke_id)
        assert self.back is not None
        self.back.sendto(data, self.server)


    @property
    def requests(self) -> int:
        """Distinct confirmed requests seen, passed or dropped."""
        return len(self.passed | self.dropped)


class Back(asyncio.DatagramProtocol):
    def __init__(self, relay: Relay) -> None:
        self.relay = relay

    def datagram_received(self, data: bytes, addr: tuple[str, int]) -> None:
        if self.relay.client is not None and self.relay.front is not None:
            self.relay.front.sendto(data, self.relay.client)


async def relay_to(address: str, passing: int = 1_000_000) -> Relay:
    host, port = address.rsplit(":", 1)
    loop = asyncio.get_running_loop()
    relay = Relay((host, int(port)), passing)
    relay.front, _ = await loop.create_datagram_endpoint(
        lambda: relay, local_addr=("127.0.0.1", 0)
    )
    relay.back, _ = await loop.create_datagram_endpoint(
        lambda: Back(relay), local_addr=("127.0.0.1", 0)
    )
    return relay


def relay_address(relay: Relay) -> str:
    assert relay.front is not None
    host, port = relay.front.get_extra_info("sockname")[:2]
    return f"{host}:{port}"


def close(relay: Relay) -> None:
    for transport in (relay.front, relay.back):
        if transport is not None:
            transport.close()


PREFIX = re.compile(r"^@prefix (\w*): <[^>]*> \.$")
NAME = re.compile(r"(?<![\w\"<])([A-Za-z][\w-]*):[\w.-]*")
IDENTIFIER = re.compile(r"^[A-Za-z0-9-]+,[1-9][0-9]*$")


def check_structure(text: str) -> list[str]:
    """The core's structural check, in brief: every prefix used is declared
    first, every statement ends in ``;`` or ``.``, and every identifier
    string fits the shape's pattern. Returns the statements' subjects."""
    declared: set[str] = set()
    subjects: list[str] = []
    open_statement = False
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        if line.startswith("@prefix"):
            match = PREFIX.match(line)
            assert match, line
            declared.add(match.group(1))
            continue
        without_literals = re.sub(r'"(?:[^"\\]|\\.)*"', '""', line)
        for prefix in NAME.findall(without_literals):
            assert prefix in declared, f"{prefix}: not declared ({line})"
        if not open_statement:
            assert not line.startswith(" "), f"no subject: {line}"
            subjects.append(line.split(" ", 1)[0])
        assert line.endswith((" ;", " .")), f"not ended: {line}"
        open_statement = line.endswith(" ;")
        for predicate in ("bacnet:device-identifier", "bacnet:object-identifier"):
            if predicate in line:
                value = re.search(r'"([^"]*)"', line)
                assert value and IDENTIFIER.match(value.group(1)), line
    assert not open_statement, "the last statement isn't ended"
    return subjects


def stub_method(class_name: str, name: str) -> ast.FunctionDef:
    path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    owner = next(
        node
        for node in tree.body
        if isinstance(node, ast.ClassDef) and node.name == class_name
    )
    return next(
        node
        for node in owner.body
        if isinstance(node, ast.FunctionDef) and node.name == name
    )


def lab() -> BACnetServer:
    server = BACnetServer(
        DEVICE,
        "AHU-1 Controller",
        interface="127.0.0.1",
        port=0,
        broadcast_address="127.0.0.1",
    )
    server.add_analog_input(3, "Zone Temp", units=EngineeringUnits.DEGREES_FAHRENHEIT)
    server.add_binary_output(1, "Supply Fan Start")
    server.add_multistate_value(2, "Fan Mode", 3)
    # A trend log, which the export doesn't read.
    server.add_trend_log(1, "TL-1")
    return server


async def name_states(server: BACnetServer) -> None:
    for index, text in enumerate(("Off", "Low", "High"), start=1):
        await server.write_property_local(
            MSV,
            PropertyIdentifier.STATE_TEXT,
            PropertyValue.character_string(text),
            array_index=index,
            source_object=None,
        )


class ExportTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.server = lab()
        await self.server.start()
        await name_states(self.server)
        self.address = await self.server.local_address()

    async def asyncTearDown(self) -> None:
        await self.server.stop()

    async def test_the_inventory_and_its_turtle(self) -> None:
        relay = await relay_to(self.address)
        try:
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=2000
            ) as client:
                await client.add_device(DEVICE, relay_address(relay))
                inventory = await client.export_223_experimental(DEVICE)
        finally:
            close(relay)
        self.assertIsInstance(inventory, Inventory223Experimental)
        # The device's name, its Object_List, then one RPM an object: N + 2.
        self.assertEqual(relay.requests, 3 + 2)
        self.assertEqual(inventory.device_instance, DEVICE)
        self.assertEqual(inventory.device_name, "AHU-1 Controller")
        self.assertIsNone(inventory.stopped)
        ai, bo, msv = inventory.objects
        self.assertEqual(
            [o.object_identifier for o in inventory.objects], [AI, BO, MSV]
        )
        self.assertEqual(ai.name, "Zone Temp")
        self.assertIs(type(ai.units), EngineeringUnits)
        self.assertEqual(ai.units, EngineeringUnits.DEGREES_FAHRENHEIT)
        self.assertIsNone(ai.number_of_states)
        self.assertEqual(ai.state_texts, [])
        self.assertFalse(ai.commandable)
        self.assertEqual(bo.name, "Supply Fan Start")
        self.assertIsNone(bo.units)
        self.assertTrue(bo.commandable)
        self.assertEqual(msv.number_of_states, 3)
        self.assertEqual(msv.state_texts, ["Off", "Low", "High"])
        self.assertTrue(msv.commandable)
        self.assertTrue(all(o.error is None for o in inventory.objects))

        turtle = inventory.to_turtle()
        subjects = check_structure(turtle)
        self.assertIn("@prefix d: <http://example.org/bacnet/device-2436/> .", turtle)
        for node in (
            "d:analog-input-3.present-value",
            "d:analog-input-3.present-value.ref",
            "d:binary-output-1.present-value",
            "d:binary-output-1.present-value.ref",
            "d:multi-state-value-2.states",
            "d:multi-state-value-2.states.3",
            "d:multi-state-value-2.present-value",
            "d:multi-state-value-2.present-value.ref",
        ):
            self.assertIn(node, subjects)
        for line in (
            "d:analog-input-3.present-value a s223:QuantifiableObservableProperty ;",
            "    qudt:hasUnit unit:DEG_F ;",
            "    qudt:hasQuantityKind quantitykind:Temperature ;",
            "d:binary-output-1.present-value a s223:EnumeratedActuatableProperty ;",
            "    s223:hasEnumerationKind s223:Binary-Logical ;",
            '    bacnet:device-identifier "device,2436" ;',
            '    bacnet:device-name "AHU-1 Controller" ;',
            '    bacnet:object-identifier "multi-state-value,2" ;',
            '    rdfs:label "High" .',
            "# 3 objects exported, 0 skipped, 0 with an unmapped unit, "
            "0 identifiers left out for instance 0.",
        ):
            self.assertIn(line, turtle.splitlines())
        self.assertNotIn("priority-for-writing", turtle)

        named = inventory.to_turtle(
            base_iri="https://example.com/site#", write_priority=8
        )
        check_structure(named)
        self.assertIn("@prefix d: <https://example.com/site#device-2436/> .", named)
        # The binary output and the multi-state value are commandable.
        self.assertEqual(
            named.count('bacnet:priority-for-writing "8"^^xsd:nonNegativeInteger'), 2
        )
        # The same inventory always gives the same text.
        self.assertEqual(inventory.to_turtle(), turtle)

    async def test_types_narrow_what_is_read(self) -> None:
        relay = await relay_to(self.address)
        try:
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=2000
            ) as client:
                await client.add_device(DEVICE, relay_address(relay))
                inventory = await client.export_223_experimental(
                    DEVICE, types=[ObjectType.MULTI_STATE_VALUE]
                )
        finally:
            close(relay)
        self.assertEqual([o.object_identifier for o in inventory.objects], [MSV])
        self.assertEqual(relay.requests, 1 + 2)

    async def test_a_device_that_stops_answering(self) -> None:
        # The name, the Object_List and the first object pass; then silence.
        relay = await relay_to(self.address, passing=3)
        try:
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=300
            ) as client:
                await client.add_device(DEVICE, relay_address(relay))
                inventory = await client.export_223_experimental(DEVICE)
        finally:
            close(relay)
        self.assertEqual([o.object_identifier for o in inventory.objects], [AI])
        self.assertIsNotNone(inventory.stopped)
        assert inventory.stopped is not None
        self.assertTrue(
            inventory.stopped.startswith("binary-output 1: "), inventory.stopped
        )
        turtle = inventory.to_turtle()
        check_structure(turtle)
        self.assertIn("# Incomplete: the reading stopped early", turtle)

    async def test_cancelling_stops_the_export(self) -> None:
        # The fourth request (the binary output) goes unanswered, with a
        # long timeout, so the export is still waiting when it is cancelled.
        relay = await relay_to(self.address, passing=3)
        try:
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=10_000
            ) as client:
                await client.add_device(DEVICE, relay_address(relay))
                export = asyncio.ensure_future(client.export_223_experimental(DEVICE))
                async with asyncio.timeout(10):
                    while relay.requests < 4:
                        await asyncio.sleep(0.01)
                export.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await export
                # No new request follows; the client still works.
                await asyncio.sleep(0.5)
                self.assertEqual(relay.requests, 4)
                relay.passing = 1_000_000
                inventory = await client.export_223_experimental(
                    DEVICE, types=[ObjectType.ANALOG_INPUT]
                )
                self.assertEqual([o.object_identifier for o in inventory.objects], [AI])
        finally:
            close(relay)

    async def test_a_failure_before_the_object_list_raises(self) -> None:
        relay = await relay_to(self.address, passing=1)
        try:
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=300
            ) as client:
                # Not in the discovery table.
                with self.assertRaises(BacnetError):
                    await client.export_223_experimental(DEVICE + 1)
                await client.add_device(DEVICE, relay_address(relay))
                # The name passes, the Object_List read times out.
                with self.assertRaises(BacnetError):
                    await client.export_223_experimental(DEVICE)
        finally:
            close(relay)


class RefusalTests(unittest.IsolatedAsyncioTestCase):
    async def test_options_are_refused_before_anything_is_sent(self) -> None:
        async with BACnetClient(interface="127.0.0.1", port=0) as client:
            with self.assertRaisesRegex(ValueError, "trend-log objects can't be exported"):
                client.export_223_experimental(1, types=[ObjectType.TREND_LOG])
            with self.assertRaisesRegex(ValueError, "at least one object type"):
                client.export_223_experimental(1, types=[])

    async def test_to_turtle_refuses_a_bad_base_or_priority(self) -> None:
        server = lab()
        await server.start()
        try:
            async with BACnetClient(
                interface="127.0.0.1", port=0, apdu_timeout_ms=2000
            ) as client:
                await client.add_device(DEVICE, await server.local_address())
                inventory = await client.export_223_experimental(
                    DEVICE, types=[ObjectType.BINARY_OUTPUT]
                )
        finally:
            await server.stop()
        for priority in (0, 17):
            with self.subTest(priority=priority):
                with self.assertRaisesRegex(
                    ValueError, f"^write priority {priority} is not 1 to 16$"
                ):
                    inventory.to_turtle(write_priority=priority)
        for base, why in (
            ("example.org/bacnet/", "is not an absolute IRI: it has no scheme such as http:"),
            ("http://example.org/a b/", "holds ' ', which an IRI can't"),
            ("http://example.org/bacnet", "must end in / or #"),
        ):
            with self.subTest(base=base):
                with self.assertRaises(ValueError) as refused:
                    inventory.to_turtle(base_iri=base)
                self.assertEqual(str(refused.exception), f'base IRI "{base}" {why}')
        with self.assertRaises(OverflowError):
            inventory.to_turtle(write_priority=256)
        with self.assertRaises(TypeError):
            pickle.dumps(inventory)


class SignatureTests(unittest.TestCase):
    def test_runtime_and_stub_agree(self) -> None:
        export = inspect.signature(BACnetClient.export_223_experimental).parameters
        self.assertEqual(list(export), ["self", "device_instance", "types"])
        self.assertEqual(export["types"].kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertIsNone(export["types"].default)
        method = stub_method("BACnetClient", "export_223_experimental")
        self.assertEqual([a.arg for a in method.args.args], ["self", "device_instance"])
        self.assertEqual([a.arg for a in method.args.kwonlyargs], ["types"])

        turtle = inspect.signature(Inventory223Experimental.to_turtle).parameters
        self.assertEqual(list(turtle), ["self", "base_iri", "write_priority"])
        self.assertTrue(all(turtle[p].default is None for p in ("base_iri", "write_priority")))
        method = stub_method("Inventory223Experimental", "to_turtle")
        self.assertEqual(
            [a.arg for a in method.args.args], ["self", "base_iri", "write_priority"]
        )

    def test_the_stub_names_every_getter(self) -> None:
        for class_name, runtime in (
            ("Inventory223Experimental", rusty_bacnet.Inventory223Experimental),
            ("Object223Experimental", rusty_bacnet.Object223Experimental),
        ):
            path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
            tree = ast.parse(path.read_text(encoding="utf-8"))
            owner = next(
                n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == class_name
            )
            stubbed = {
                n.name for n in owner.body if isinstance(n, ast.FunctionDef)
            } - {"__repr__"}
            public = {name for name in dir(runtime) if not name.startswith("_")}
            self.assertEqual(stubbed, public, class_name)


if __name__ == "__main__":
    unittest.main()
