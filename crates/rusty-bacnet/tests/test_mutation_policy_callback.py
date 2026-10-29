"""A Python mutation policy callable (#1966), over real B/IP.

The callable gets a frozen, redacted MutationContext and decides each covered
mutation: True allows, False denies. An exception, a non-bool answer or a
coroutine denies too, and a denied request changes nothing.
"""

import asyncio
import copy
import pickle
import sys
import threading
import unittest
import warnings

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetProtocolError,
    ErrorClass,
    ErrorCode,
    MutationContext,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

AV1 = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
AV2 = ObjectIdentifier(ObjectType.ANALOG_VALUE, 2)
AO1 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 1)
AO2 = ObjectIdentifier(ObjectType.ANALOG_OUTPUT, 2)
CH2 = ObjectIdentifier(ObjectType.CHANNEL, 2)
PV = PropertyIdentifier.PRESENT_VALUE


class Recorder:
    """A policy that records every context and answers with `decide`."""

    def __init__(self, decide=lambda context: True):
        self.decide = decide
        self.contexts = []

    def __call__(self, context):
        self.contexts.append(context)
        return self.decide(context)


class UnraisableCapture:
    """Collect what the binding reports through sys.unraisablehook."""

    def __enter__(self):
        self.seen = []
        self.previous = sys.unraisablehook
        sys.unraisablehook = self.seen.append
        return self

    def __exit__(self, *exc):
        sys.unraisablehook = self.previous


class MutationCallbackTests(unittest.IsolatedAsyncioTestCase):
    async def serve(self, policy, *extra):
        server = BACnetServer(503101, interface="127.0.0.1", port=0, mutation_policy=policy)
        server.add_analog_value(1, "AV-1")
        server.add_analog_value(2, "AV-2")
        for add in extra:
            add(server)
        await server.start()

        async def stop(node):
            await node.stop()
        self.addAsyncCleanup(stop, server)
        client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await client.__aenter__()
        self.addAsyncCleanup(stop, client)
        return server, client, await server.local_address()

    def assert_denied(self, caught):
        self.assertEqual(caught.exception.error_class, ErrorClass.SERVICES)
        self.assertEqual(caught.exception.error_code, ErrorCode.SERVICE_REQUEST_DENIED)

    async def test_allow_sees_a_redacted_frozen_context(self):
        policy = Recorder()
        server, client, address = await self.serve(policy)
        async with asyncio.timeout(10):
            await client.write_property(address, AV1, PV, PropertyValue.real(42.0), priority=9)
            self.assertEqual((await client.read_property(address, AV1, PV)).value, 42.0)
        [context] = policy.contexts
        self.assertIsInstance(context, MutationContext)
        self.assertEqual(context.service, "write_property")
        self.assertEqual(context.object_identifier, AV1)
        self.assertEqual(context.object_type, ObjectType.ANALOG_VALUE)
        self.assertEqual(context.property_identifier, PV)
        self.assertIsNone(context.array_index)
        self.assertEqual(context.priority, 9)
        self.assertEqual(context.provenance, "unverified")
        # B/IP: no principal and no certificate (Python has no direct SC).
        self.assertIsNone(context.principal)
        self.assertIsNone(context.forwarding_peer_principal)
        self.assertIsNone(context.certificate)
        self.assertIsNone(context.forwarding_peer_certificate)
        self.assertIsInstance(context.invoke_id, int)
        self.assertIsInstance(context.source_mac, bytes)
        self.assertEqual(len(context.source_mac), 6)
        self.assertIsNone(context.source_network)
        self.assertIsNone(context.source_address)
        self.assertEqual(
            repr(context), f"MutationContext(service='write_property', invoke_id={context.invoke_id})")
        with self.assertRaises(AttributeError):
            context.service = "delete_object"
        for clone in (pickle.dumps, copy.copy):
            with self.assertRaises(TypeError):
                clone(context)

    async def test_deny_answers_service_request_denied_and_changes_nothing(self):
        policy = Recorder(lambda context: context.object_identifier != AV2)
        server, client, address = await self.serve(policy)
        async with asyncio.timeout(10):
            before = await client.read_property(address, AV2, PV)
            with self.assertRaises(BacnetProtocolError) as caught:
                await client.write_property(address, AV2, PV, PropertyValue.real(7.0))
            self.assert_denied(caught)
            self.assertEqual(await client.read_property(address, AV2, PV), before)
            with self.assertRaises(BacnetProtocolError) as caught:
                await client.delete_object(address, AV2)
            self.assert_denied(caught)
            self.assertEqual(await server.read_property(AV2, PV), before)
            await client.write_property(address, AV1, PV, PropertyValue.real(8.0))
            self.assertEqual((await client.read_property(address, AV1, PV)).value, 8.0)
        self.assertEqual([c.service for c in policy.contexts],
                         ["write_property", "delete_object", "write_property"])

    async def test_create_and_delete_contexts(self):
        policy = Recorder(lambda context: context.service == "delete_object")
        server, client, address = await self.serve(policy)
        async with asyncio.timeout(10):
            with self.assertRaises(BacnetProtocolError) as caught:
                await client.create_object(address, ObjectType.ANALOG_VALUE)
            self.assert_denied(caught)
            await client.delete_object(address, AV1)
            with self.assertRaises(BacnetProtocolError):
                await client.read_property(address, AV1, PV)
        create, delete = policy.contexts
        self.assertEqual(create.service, "create_object")
        self.assertIsNone(create.object_identifier)
        self.assertEqual(create.object_type, ObjectType.ANALOG_VALUE)
        self.assertIsNone(create.property_identifier)
        self.assertEqual((delete.service, delete.object_identifier), ("delete_object", AV1))

    async def test_write_property_multiple_asks_each_element(self):
        policy = Recorder(lambda context: context.object_identifier == AV1)
        server, client, address = await self.serve(policy)
        async with asyncio.timeout(10):
            with self.assertRaises(BacnetProtocolError):
                await client.write_property_multiple(address, [
                    (AV1, [(PV, PropertyValue.real(11.0), None, None)]),
                    (AV2, [(PV, PropertyValue.real(12.0), 10, None)]),
                ])
            self.assertEqual((await client.read_property(address, AV1, PV)).value, 11.0)
            self.assertEqual((await client.read_property(address, AV2, PV)).value, 0.0)
        self.assertEqual([(c.service, c.object_identifier, c.priority) for c in policy.contexts],
                         [("write_property_multiple", AV1, None),
                          ("write_property_multiple", AV2, 10)])

    async def test_write_group_asks_each_channel(self):
        def channels(server):
            server.add_analog_output(1, "AO-1")
            server.add_analog_output(2, "AO-2")
            server.add_channel(1, "CH-1", 11, [(AO1, PV)], control_groups=[5])
            server.add_channel(2, "CH-2", 11, [(AO2, PV)], control_groups=[5])

        policy = Recorder(lambda context: context.object_identifier != CH2)
        server, client, address = await self.serve(policy, channels)
        async with asyncio.timeout(10):
            await client.write_group(address, 5, 8, [(11, None, PropertyValue.real(72.0))])
            while (await server.read_property(AO1, PV)).value != 72.0:
                await asyncio.sleep(0.01)
            self.assertEqual((await server.read_property(AO2, PV)).value, 0.0)
        self.assertEqual(sorted(c.object_identifier.instance for c in policy.contexts), [1, 2])
        for context in policy.contexts:
            self.assertEqual((context.service, context.priority), ("write_group", 8))
            self.assertIsNone(context.invoke_id)
            self.assertEqual(repr(context), "MutationContext(service='write_group')")

    async def test_a_blocked_callable_stalls_only_its_own_request(self):
        # The callable waits on another thread, as one calling
        # run_coroutine_threadsafe(...).result() would. The server asks it
        # with no database lock held and off the request task (#2032), so a
        # second client's read and stop() both go ahead meanwhile.
        entered, release = threading.Event(), threading.Event()

        def policy(context):
            if context.service == "write_property_multiple":
                entered.set()
                release.wait(10)
            return True

        server, client, address = await self.serve(policy)
        self.addCleanup(release.set)
        other = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await other.__aenter__()

        async def stop_other():
            await other.stop()
        self.addAsyncCleanup(stop_other)
        writing = asyncio.ensure_future(client.write_property_multiple(
            address, [(AV1, [(PV, PropertyValue.real(5.0), None, None)])]))
        try:
            self.assertTrue(await asyncio.to_thread(entered.wait, 5))
            async with asyncio.timeout(5):
                self.assertEqual((await other.read_property(address, AV2, PV)).value, 0.0)
                self.assertFalse(writing.done())
                await server.stop()
                # Stopped while the callable was still blocked.
                self.assertFalse(release.is_set())
        finally:
            release.set()
            writing.cancel()
            await asyncio.gather(writing, return_exceptions=True)

    async def test_failures_deny_without_side_effects(self):
        async def coroutine_policy(context):
            return True

        def raising(context):
            raise RuntimeError("policy bug")

        cases = [
            ("exception", raising, RuntimeError),
            ("truthy int", lambda context: 1, TypeError),
            ("None", lambda context: None, TypeError),
            ("wrong arity", lambda: True, TypeError),
            ("coroutine", coroutine_policy, TypeError),
        ]
        for name, policy, raised in cases:
            with self.subTest(name), UnraisableCapture() as capture:
                with warnings.catch_warnings(record=True) as caught_warnings:
                    warnings.simplefilter("always")
                    server = BACnetServer(503102, interface="127.0.0.1", port=0,
                                          mutation_policy=policy)
                    server.add_analog_value(1, "AV-1")
                    await server.start()
                    try:
                        async with BACnetClient(interface="127.0.0.1", port=0,
                                                apdu_timeout_ms=2000) as client:
                            address = await server.local_address()
                            async with asyncio.timeout(10):
                                with self.assertRaises(BacnetProtocolError) as caught:
                                    await client.write_property(
                                        address, AV1, PV, PropertyValue.real(5.0))
                                self.assert_denied(caught)
                                self.assertEqual(
                                    (await client.read_property(address, AV1, PV)).value, 0.0)
                    finally:
                        await server.stop()
                self.assertEqual(len(capture.seen), 1)
                self.assertIsInstance(capture.seen[0].exc_value, raised)
                self.assertFalse([w for w in caught_warnings
                                  if "never awaited" in str(w.message)])


if __name__ == "__main__":
    unittest.main()
