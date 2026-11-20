"""Registration, command defaults and discovery of points on one shared endpoint."""

import asyncio
import socket
import unittest

import rusty_bacnet as rb
from test_bip_modes import accepted_registration


P = rb.PropertyIdentifier
O = rb.ObjectType
V = rb.PropertyValue

# One instance shared by all nine object types is a legal set of identifiers.
POINTS = (
    ("analog_input", O.ANALOG_INPUT, "real", 17.5),
    ("analog_output", O.ANALOG_OUTPUT, "real", 21.5),
    ("analog_value", O.ANALOG_VALUE, "real", 23.5),
    ("binary_input", O.BINARY_INPUT, "enumerated", 1),
    ("binary_output", O.BINARY_OUTPUT, "enumerated", 1),
    ("binary_value", O.BINARY_VALUE, "enumerated", 1),
    ("multistate_input", O.MULTI_STATE_INPUT, "unsigned", 3),
    ("multistate_output", O.MULTI_STATE_OUTPUT, "unsigned", 2),
    ("multistate_value", O.MULTI_STATE_VALUE, "unsigned", 4),
)


def endpoint(**options):
    return rb.BipEndpoint(2688, device_name="Point host", interface="127.0.0.1",
                          port=0, broadcast_address="127.0.0.1", **options)


async def finish(method):
    await method()


class EndpointPointBuilders(unittest.IsolatedAsyncioTestCase):
    async def test_nine_points_are_served_with_initial_values_and_empty_priorities(self):
        owner = endpoint(mutation_policy="permissive", bbmd=rb.Bbmd(accept_foreign_devices=True))
        for kind, _, _, initial in POINTS:
            args = (4,) if kind.startswith("multistate") else ()
            options = {"polarity": 1} if kind == "binary_input" else {}
            getattr(owner, "add_" + kind)(1, kind, *args, present_value=initial, **options)
        await owner.start()
        self.addAsyncCleanup(finish, owner.close)
        address = await owner.local_address()
        async with rb.BACnetClient(interface="127.0.0.1", port=0,
                                    foreign_device=rb.ForeignDevice(address, ttl=60, renewal=10)) as peer:
            await accepted_registration(peer)
            for kind, typ, tag, initial in POINTS:
                with self.subTest(kind=kind):
                    oid = rb.ObjectIdentifier(typ, 1)

                    async def read(prop):
                        return await peer.read_property(address, oid, prop)

                    self.assertEqual((await read(P.OBJECT_NAME)).value, kind)
                    present = await read(P.PRESENT_VALUE)
                    self.assertEqual((present.tag, present.value), (tag, initial))
                    if kind.startswith("multistate"):
                        self.assertEqual((await read(P.NUMBER_OF_STATES)).value, 4)
                    if kind.endswith("_input"):
                        continue
                    self.assertEqual(await read(P.RELINQUISH_DEFAULT), present)
                    self.assertEqual((await read(P.PRIORITY_ARRAY)).value, [None] * 16)
                    command = getattr(V, tag)(0 if tag == "enumerated" else 1)
                    await peer.write_property(address, oid, P.PRESENT_VALUE, command, priority=8)
                    self.assertEqual(await read(P.PRESENT_VALUE), command)
                    await peer.write_property(address, oid, P.PRESENT_VALUE, V.null(), priority=8)
                    self.assertEqual(await read(P.PRESENT_VALUE), present)
                    self.assertEqual((await read(P.PRIORITY_ARRAY)).value, [None] * 16)

            frames = await owner.frames()
            found = await peer.who_has_replies("multistate_output", scope=address, wait=1)
            self.assertEqual([(row.object_identifier, row.object_name) for row in found],
                             [(rb.ObjectIdentifier(O.MULTI_STATE_OUTPUT, 1), "multistate_output")])
            self.assertEqual(await peer.who_has_replies("Unregistered", scope=address, wait=1), [])
            async with asyncio.timeout(10):
                async for frame in frames:
                    summary = rb.decode_frame(frame.octets, frame.link)
                    service = (summary.service or "").lower().replace("-", "").replace("_", "")
                    if frame.direction == "inbound" and service == "whohas":
                        break
                else:
                    self.fail("endpoint closed the frame stream during discovery")
            await frames.aclose()
            final = await peer.read_property(address, rb.ObjectIdentifier(O.ANALOG_OUTPUT, 1), P.PRESENT_VALUE)
            self.assertEqual(final.value, 21.5)
        await owner.close()
        # Closing the owner releases its original port.
        host, port = address.rsplit(":", 1)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
            probe.bind((host, int(port)))

    async def test_omitted_binary_input_does_not_seed_a_physical_state(self):
        owner = endpoint()
        owner.add_binary_input(1, "Unsampled", polarity=1)
        owner.add_binary_input(2, "Sampled", polarity=1, present_value=0)
        await owner.start()
        self.addAsyncCleanup(finish, owner.close)
        server = await owner.server()
        for instance, expected in ((1, 0), (2, 1)):
            oid = rb.ObjectIdentifier(O.BINARY_INPUT, instance)
            await server.set_polarity_local(oid, 0)
            value = await server.read_property(oid, P.PRESENT_VALUE)
            self.assertEqual(value.value, expected)

    def test_identity_reservations_follow_the_endpoint_transport(self):
        sc = rb.ScEndpoint(2688, sc_hub="wss://localhost:1", sc_ca_cert="ca.pem",
                           sc_client_cert="client.pem", sc_client_key="key.pem",
                           sc_vmac=bytes.fromhex("020000000001"),
                           sc_device_uuid=bytes.fromhex("0123456789abcdef0123456789abcdef"))
        mstp = rb.MstpEndpoint(2688, serial_port="unused")
        for owner, port_name in ((endpoint(), "port-1"), (sc, "port-2")):
            with self.assertRaisesRegex(ValueError, "duplicate object name"):
                owner.add_multistate_value(1, port_name, 2)
        mstp.add_multistate_value(1, "port-1", 2)
        with self.assertRaisesRegex(ValueError, "duplicate object name"):
            mstp.add_multistate_value(2, "BACnet Device", 2)

    async def test_noncommandable_values_get_their_logical_input(self):
        owner = endpoint()
        for instance, access in enumerate(("writable", "read_only"), 1):
            for kind, _, _, initial in POINTS:
                if kind.endswith("_value"):
                    args = (4,) if kind.startswith("multistate") else ()
                    getattr(owner, "add_" + kind)(instance, f"{kind} {access}", *args,
                                                  present_value=initial, present_value_access=access)
        await owner.start()
        self.addAsyncCleanup(finish, owner.close)
        address = await owner.local_address()
        async with rb.BACnetClient(interface="127.0.0.1", port=0) as peer:
            for instance in (1, 2):
                for kind, typ, tag, initial in POINTS:
                    if kind.endswith("_value"):
                        oid = rb.ObjectIdentifier(typ, instance)
                        value = await peer.read_property(address, oid, P.PRESENT_VALUE)
                        self.assertEqual((value.tag, value.value), (tag, initial))

    async def test_invalid_initial_values_and_duplicate_keys_refuse_only_that_registration(self):
        owner = endpoint(network_port_instance=31)
        owner.add_group(7, "Members")
        owner.add_audit_reporter(8, "Recorder", audit_level="none", auditable_operations=0)
        owner.add_analog_input(1, "Original", 62, 2.5)
        invalid = (rb.BacnetError, ValueError)
        for kind, _, _, _ in POINTS:
            register = getattr(owner, "add_" + kind)
            with self.subTest(kind=kind):
                if kind.startswith("analog"):
                    for value in (float("nan"), float("inf"), float("-inf")):
                        with self.assertRaises(invalid):
                            register(2, kind, present_value=value)
                elif kind.startswith("binary"):
                    with self.assertRaises(invalid):
                        register(2, kind, present_value=2)
                else:
                    for count, value in ((0, 1), (4, 0), (4, 5)):
                        with self.assertRaises(invalid):
                            register(2, kind, count, present_value=value)
        for name in ("Point host", "port-31", "Members", "Recorder", "Original"):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "duplicate object name"):
                owner.add_binary_output(1, name)
        with self.assertRaisesRegex(ValueError, "duplicate object identifier"):
            owner.add_analog_input(1, "Another name")
        for instance in (4194303, 4194304):
            with self.subTest(instance=instance), self.assertRaisesRegex(invalid, "instance"):
                owner.add_binary_output(instance, "Out of range")
        self.assertEqual(owner._pending_registration_count(), 3)
        await owner.start()
        self.addAsyncCleanup(finish, owner.close)
        address = await owner.local_address()
        async with rb.BACnetClient(interface="127.0.0.1", port=0) as peer:
            value = await peer.read_property(address, rb.ObjectIdentifier(O.ANALOG_INPUT, 1), P.PRESENT_VALUE)
            self.assertEqual(value.value, 2.5)
