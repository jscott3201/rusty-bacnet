"""An endpoint's device_write_policy callable (#2033), over real B/IP.

The callable decides each WriteProperty to the endpoint's local Device with
the MutationContext and fail-closed rules of a server's mutation_policy
callable: True allows; False, an exception or a non-bool denies, and a denied
write changes nothing.
"""

import asyncio
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


class DeviceWritePolicyTests(unittest.IsolatedAsyncioTestCase):
    async def serve(self, policy):
        endpoint = BipEndpoint(INSTANCE, interface="127.0.0.1", port=0,
                               broadcast_address="127.0.0.1", device_write_policy=policy)
        endpoint.add_analog_input(1, "AI-1")
        await endpoint.start()

        async def close():
            await endpoint.close()
        self.addAsyncCleanup(close)
        client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
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

    async def test_allow_writes_and_sees_the_context(self):
        policy = Recorder()
        endpoint, client, address = await self.serve(policy)
        async with asyncio.timeout(10):
            scope = (await endpoint.server()).service_scope()
            self.assertEqual(scope["executes"], ["read_property", "write_property"])
            await client.write_property(address, DEVICE, DESCRIPTION,
                                        PropertyValue.character_string("lobby"))
            self.assertEqual(await self.description(client, address), "lobby")
        [context] = policy.contexts
        self.assertIsInstance(context, MutationContext)
        self.assertEqual(context.service, "write_property")
        self.assertEqual(context.object_identifier, DEVICE)
        self.assertEqual(context.property_identifier, DESCRIPTION)
        self.assertEqual(context.provenance, "unverified")
        self.assertIsInstance(context.invoke_id, int)

    async def test_deny_answers_service_request_denied_and_changes_nothing(self):
        policy = Recorder(lambda context: False)
        _, client, address = await self.serve(policy)
        async with asyncio.timeout(10):
            before = await self.description(client, address)
            with self.assertRaises(BacnetProtocolError) as caught:
                await client.write_property(address, DEVICE, DESCRIPTION,
                                            PropertyValue.character_string("denied"))
            self.assert_denied(caught)
            self.assertEqual(await self.description(client, address), before)
        self.assertEqual(len(policy.contexts), 1)

    async def test_other_targets_are_refused_before_the_policy(self):
        policy = Recorder()
        _, client, address = await self.serve(policy)
        async with asyncio.timeout(10):
            with self.assertRaises(BacnetProtocolError) as caught:
                await client.write_property(address, AI1, DESCRIPTION,
                                            PropertyValue.character_string("no"))
            self.assertEqual(caught.exception.error_code, ErrorCode.WRITE_ACCESS_DENIED)
        self.assertEqual(policy.contexts, [])

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
                                       device_write_policy=policy)
                await endpoint.start()
                try:
                    async with BACnetClient(interface="127.0.0.1", port=0,
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
                    self.assertIn("device_write_policy", str(capture.seen[0].exc_value))

    async def test_constructor_refuses_a_non_callable(self):
        with self.assertRaisesRegex(TypeError, "device_write_policy"):
            BipEndpoint(INSTANCE, device_write_policy="allow")
        # None, the default, leaves Device writes off.
        async with BipEndpoint(INSTANCE, interface="127.0.0.1", port=0,
                               broadcast_address="127.0.0.1",
                               device_write_policy=None) as endpoint:
            scope = (await endpoint.server()).service_scope()
            self.assertEqual(scope["executes"], ["read_property"])


if __name__ == "__main__":
    unittest.main()
