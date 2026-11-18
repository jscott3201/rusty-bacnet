"""An endpoint's mutation_policy (#2617), over real B/IP.

The endpoint's full server takes ``mutation_policy`` as ``BACnetServer``
takes it: ``"permissive"``, ``"deny_all"`` or a callable that decides each
mutation of every object with the MutationContext and fail-closed rules of
a server's callable. Without one the server is read-only. It replaces the
0.15.0 ``device_write_policy``, which decided writes to the local Device
alone.
"""

import asyncio
import inspect
import sys
import unittest

from rusty_bacnet import (
    BACnetClient,
    BacnetProtocolError,
    BipEndpoint,
    ErrorClass,
    ErrorCode,
    MutationContext,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
)

INSTANCE = 9301
DEVICE = ObjectIdentifier(ObjectType.DEVICE, INSTANCE)
AI1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
DESCRIPTION = PropertyIdentifier.DESCRIPTION
OUT_OF_SERVICE = PropertyIdentifier.OUT_OF_SERVICE


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


class EndpointMutationPolicyTests(unittest.IsolatedAsyncioTestCase):
    async def serve(self, **keywords):
        endpoint = BipEndpoint(INSTANCE, interface="127.0.0.1", port=0,
                               broadcast_address="127.0.0.1", **keywords)
        endpoint.add_analog_input(1, "AI-1")
        await endpoint.start()

        async def close():
            await endpoint.close()
        self.addAsyncCleanup(close)
        client = BACnetClient(interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1", apdu_timeout_ms=2000)
        await client.__aenter__()

        async def stop():
            await client.stop()
        self.addAsyncCleanup(stop)
        return endpoint, client, await endpoint.local_address()

    async def description(self, client, address):
        return (await client.read_property(address, DEVICE, DESCRIPTION)).value

    def assert_denied(self, caught):
        self.assertEqual(caught.exception.error_class, ErrorClass.SERVICES)
        self.assertEqual(caught.exception.error_code, ErrorCode.SERVICE_REQUEST_DENIED)

    async def test_a_callable_decides_each_mutation_of_every_object(self):
        # Allow the Device's Description, deny the rest: the server asks the
        # callable about every object, not the Device's two properties alone.
        policy = Recorder(lambda context: context.object_identifier == DEVICE)
        endpoint, client, address = await self.serve(mutation_policy=policy)
        async with asyncio.timeout(10):
            await client.write_property(address, DEVICE, DESCRIPTION,
                                        PropertyValue.character_string("lobby"))
            self.assertEqual(await self.description(client, address), "lobby")
            with self.assertRaises(BacnetProtocolError) as caught:
                await client.write_property(address, AI1, OUT_OF_SERVICE,
                                            PropertyValue.boolean(True))
            self.assert_denied(caught)
            # The subscriptions are mutations too, which this policy denies.
            with self.assertRaises(BacnetProtocolError) as caught:
                await client.subscribe_cov(address, 1, AI1, confirmed=False, lifetime=60)
            self.assert_denied(caught)
            server = await endpoint.server()
            self.assertEqual(await server.read_property(AI1, OUT_OF_SERVICE),
                             PropertyValue.boolean(False))
        services = [context.service for context in policy.contexts]
        self.assertEqual(services, ["write_property", "write_property", "subscribe_cov"])
        context = policy.contexts[0]
        self.assertIsInstance(context, MutationContext)
        self.assertEqual(context.object_identifier, DEVICE)
        self.assertEqual(context.property_identifier, DESCRIPTION)
        self.assertEqual(context.provenance, "unverified")
        self.assertIsInstance(context.invoke_id, int)
        self.assertEqual(policy.contexts[1].object_identifier, AI1)

    async def test_permissive_lets_any_peer_write_as_a_standalone_server(self):
        endpoint, client, address = await self.serve(mutation_policy="permissive")
        async with asyncio.timeout(10):
            await client.write_property(address, AI1, OUT_OF_SERVICE,
                                        PropertyValue.boolean(True))
            server = await endpoint.server()
            self.assertEqual(await server.read_property(AI1, OUT_OF_SERVICE),
                             PropertyValue.boolean(True))

    async def test_deny_all_denies_subscriptions_too(self):
        _, client, address = await self.serve(mutation_policy="deny_all")
        async with asyncio.timeout(10):
            for write in (
                lambda: client.write_property(address, DEVICE, DESCRIPTION,
                                              PropertyValue.character_string("no")),
                lambda: client.subscribe_cov(address, 1, AI1, confirmed=False, lifetime=60),
            ):
                with self.assertRaises(BacnetProtocolError) as caught:
                    await write()
                self.assert_denied(caught)
            # Reads are answered.
            self.assertIsInstance(await self.description(client, address), str)

    async def test_without_a_policy_the_device_is_read_only(self):
        # The 0.15.0 default (no device_write_policy: no WriteProperty) holds
        # for the full server: no peer writes, even the Device (#2617).
        _, client, address = await self.serve()
        async with asyncio.timeout(10):
            before = await self.description(client, address)
            with self.assertRaises(BacnetProtocolError) as caught:
                await client.write_property(address, DEVICE, DESCRIPTION,
                                            PropertyValue.character_string("denied"))
            self.assert_denied(caught)
            self.assertEqual(await self.description(client, address), before)

    async def test_failures_deny_without_side_effects(self):
        async def coroutine_policy(context):
            return True

        def raising(context):
            raise RuntimeError("policy bug")

        cases = [
            ("exception", raising, RuntimeError),
            ("truthy int", lambda context: 1, TypeError),
            ("coroutine", coroutine_policy, TypeError),
        ]
        for name, policy, raised in cases:
            with self.subTest(name), UnraisableCapture() as capture:
                endpoint = BipEndpoint(INSTANCE, interface="127.0.0.1", port=0,
                                       broadcast_address="127.0.0.1",
                                       mutation_policy=policy)
                await endpoint.start()
                try:
                    async with BACnetClient(interface="127.0.0.1", port=0,
                                            broadcast_address="127.0.0.1",
                                            apdu_timeout_ms=2000) as client:
                        address = await endpoint.local_address()
                        async with asyncio.timeout(10):
                            before = await self.description(client, address)
                            with self.assertRaises(BacnetProtocolError) as caught:
                                await client.write_property(
                                    address, DEVICE, DESCRIPTION,
                                    PropertyValue.character_string("bug"))
                            self.assert_denied(caught)
                            self.assertEqual(await self.description(client, address), before)
                finally:
                    await endpoint.close()
                self.assertEqual(len(capture.seen), 1)
                self.assertIsInstance(capture.seen[0].exc_value, raised)
                if raised is TypeError:
                    self.assertIn("mutation_policy", str(capture.seen[0].exc_value))

    def test_the_keywords_are_the_servers(self):
        for owner in ("BipEndpoint", "ScEndpoint", "MstpEndpoint"):
            import rusty_bacnet
            parameters = inspect.signature(getattr(rusty_bacnet, owner)).parameters
            with self.subTest(owner=owner):
                for keyword in ("mutation_policy", "cov_policy", "time_sync_policy"):
                    self.assertIs(parameters[keyword].kind, inspect.Parameter.KEYWORD_ONLY)
                    self.assertIsNone(parameters[keyword].default)
                for gone in ("device_write_policy", "cov", "services"):
                    self.assertNotIn(gone, parameters)
        with self.assertRaisesRegex(ValueError, "mutation_policy must be"):
            BipEndpoint(INSTANCE, mutation_policy="allow")
        with self.assertRaisesRegex(TypeError, "mutation_policy must be"):
            BipEndpoint(INSTANCE, mutation_policy=1)
        with self.assertRaisesRegex(TypeError, "device_write_policy"):
            BipEndpoint(INSTANCE, device_write_policy=lambda context: True)
        # time_sync_policy is read as BACnetServer's is.
        with self.assertRaisesRegex(TypeError,
                                    r"^time_sync_policy got an unexpected key 'enable'$"):
            BipEndpoint(INSTANCE, time_sync_policy={"enable": True})


if __name__ == "__main__":
    unittest.main()
