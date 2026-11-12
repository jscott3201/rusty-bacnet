"""A running Load Control's configuration (#2497): Shed_Levels,
Full_Duty_Baseline and State_Description change through the server's local
routes, a request under way is put to `shed` again, and `storage_path` keeps
a request, and the Shed_Levels elements a client wrote (#2502), across a
restart."""

import asyncio
import datetime
import gc
import inspect
import os
import tempfile
import unittest

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetError,
    BacnetProtocolError,
    ErrorClass,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier as P,
    PropertyValue as V,
)

LC1 = ObjectIdentifier(ObjectType.LOAD_CONTROL, 1)
AV1 = ObjectIdentifier(ObjectType.ANALOG_VALUE, 1)
INACTIVE, PENDING, COMPLIANT, NON_COMPLIANT = 0, 1, 2, 3
SETTERS = (
    "set_load_control_shed_levels_local",
    "set_load_control_full_duty_baseline_local",
    "set_load_control_state_description_local",
)


def level(value):
    return {"kind": "level", "value": value}


def yesterday():
    """Start_Time at 00:00 yesterday (UTC, the default Device clock), so
    that a three-day request is under way whatever the time now."""
    day = datetime.datetime.now(datetime.timezone.utc).date() - datetime.timedelta(days=1)
    return V.list([
        V.date(day.year, day.month, day.day, day.isoweekday()),
        V.time(0, 0, 0, 0),
    ])


class Shedder:
    """Records each request and stop, and answers with `answer`."""

    def __init__(self, answer=("compliant", level(2), level(2))):
        self.answer = answer
        self.requests = []
        self.stops = 0

    def shed(self, request):
        self.requests.append(request)
        return self.answer(request) if callable(self.answer) else self.answer

    def stop(self):
        self.stops += 1


def server():
    return BACnetServer(503202, interface="127.0.0.1", port=0)


def assert_code(case, raised, error_class, code):
    case.assertEqual(raised.error_class, error_class.to_raw())
    case.assertEqual(raised.error_code, code.to_raw())


class SignatureTests(unittest.TestCase):
    def test_signatures(self):
        for name, second in zip(SETTERS, ("shed_levels", "kilowatts", "text")):
            with self.subTest(name=name):
                parameters = inspect.signature(getattr(BACnetServer, name)).parameters
                self.assertEqual(list(parameters), ["self", "object_id", second])
        self.assertIn("storage_path", inspect.signature(BACnetServer.add_load_control).parameters)

    def test_a_server_not_started_refuses(self):
        async def run():
            node = server()
            node.add_load_control(1, "LC-1")
            with self.assertRaises(RuntimeError):
                await node.set_load_control_state_description_local(LC1, "x")
        asyncio.run(run())


class SetterTests(unittest.IsolatedAsyncioTestCase):
    async def serve(self, shedder, **keywords):
        node = server()
        node.add_load_control(
            1, "LC-1", shed_levels=[(1, "lights"), (2, "chillers"), (3, "pumps")],
            shed=shedder.shed, stop=shedder.stop, **keywords)
        node.add_analog_value(1, "AV-1")
        await node.start()

        async def stop(owner):
            await owner.stop()
        self.addAsyncCleanup(stop, node)
        client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await client.__aenter__()
        self.addAsyncCleanup(stop, client)
        return node, client, await node.local_address()

    async def request(self, client, address):
        await client.write_property(address, LC1, P.REQUESTED_SHED_LEVEL,
                                    V.application_data(b"\x19\x02"))
        await client.write_property(address, LC1, P.SHED_DURATION, V.unsigned(3 * 24 * 60))
        await client.write_property(address, LC1, P.START_TIME, yesterday())

    async def state(self, node):
        return (await node.read_property(LC1, P.PRESENT_VALUE)).value

    async def test_shed_levels_change_and_reach_a_request_under_way(self):
        shedder = Shedder()
        node, client, address = await self.serve(shedder)
        async with asyncio.timeout(10):
            await self.request(client, address)
            self.assertEqual(await self.state(node), COMPLIANT)
            self.assertEqual(shedder.requests[-1].shed_level, 1)
            # The loads fall short; the route puts the request to them
            # again, so the state follows with no wait for the pass.
            shedder.answer = ("shedding", level(2), level(1))
            await node.set_load_control_shed_levels_local(
                LC1, [(2, "Raise setpoints"), (4, "Stop the chillers")])
            self.assertEqual(await self.state(node), NON_COMPLIANT)
            self.assertEqual(shedder.requests[-1].shed_level, 0)
            self.assertEqual(await node.read_property(LC1, P.SHED_LEVELS),
                             V.list([V.unsigned(2), V.unsigned(4)]))
            self.assertEqual(
                await node.read_property(LC1, P.SHED_LEVEL_DESCRIPTIONS),
                V.list([V.character_string("Raise setpoints"),
                        V.character_string("Stop the chillers")]))
            # A level of 0 is refused, and nothing changes.
            with self.assertRaises(BacnetProtocolError) as caught:
                await node.set_load_control_shed_levels_local(LC1, [(1, "a"), (0, "b")])
            assert_code(self, caught.exception, ErrorClass.PROPERTY, ErrorCode.VALUE_OUT_OF_RANGE)
            self.assertEqual(await node.read_property(LC1, P.SHED_LEVELS),
                             V.list([V.unsigned(2), V.unsigned(4)]))
            with self.assertRaises(TypeError):
                await node.set_load_control_shed_levels_local(LC1, "12")

    async def test_the_baseline_is_set_cleared_and_checked(self):
        shedder = Shedder()
        node, client, address = await self.serve(shedder)
        async with asyncio.timeout(10):
            await self.request(client, address)
            self.assertIsNone(shedder.requests[-1].full_duty_baseline)
            await node.set_load_control_full_duty_baseline_local(LC1, 250.5)
            self.assertEqual(await node.read_property(LC1, P.FULL_DUTY_BASELINE), V.real(250.5))
            self.assertEqual(shedder.requests[-1].full_duty_baseline, 250.5)
            self.assertEqual(await client.read_property(address, LC1, P.FULL_DUTY_BASELINE),
                             V.real(250.5))
            for bad in (-1.0, float("nan"), float("inf"), 1e300):
                with self.subTest(bad=bad):
                    with self.assertRaises(BacnetProtocolError) as caught:
                        await node.set_load_control_full_duty_baseline_local(LC1, bad)
                    assert_code(self, caught.exception, ErrorClass.PROPERTY,
                                ErrorCode.VALUE_OUT_OF_RANGE)
            self.assertEqual(await node.read_property(LC1, P.FULL_DUTY_BASELINE), V.real(250.5))
            await node.set_load_control_full_duty_baseline_local(LC1, None)
            self.assertIsNone(shedder.requests[-1].full_duty_baseline)
            with self.assertRaises(BacnetProtocolError) as caught:
                await node.read_property(LC1, P.FULL_DUTY_BASELINE)
            self.assertEqual(caught.exception.error_code, ErrorCode.UNKNOWN_PROPERTY.to_raw())

    async def test_the_state_description_follows_the_state(self):
        shedder = Shedder()
        node, _, _ = await self.serve(shedder, state_description="Idle")
        async with asyncio.timeout(10):
            asked = len(shedder.requests)
            await node.set_load_control_state_description_local(LC1, "Shedding 2 of 3")
            self.assertEqual(await node.read_property(LC1, P.STATE_DESCRIPTION),
                             V.character_string("Shedding 2 of 3"))
            self.assertEqual(len(shedder.requests), asked)
            await node.set_load_control_state_description_local(LC1, None)
            with self.assertRaises(BacnetProtocolError) as caught:
                await node.read_property(LC1, P.STATE_DESCRIPTION)
            self.assertEqual(caught.exception.error_code, ErrorCode.UNKNOWN_PROPERTY.to_raw())

    async def test_the_errors_of_an_unknown_object_and_another_type(self):
        node, _, _ = await self.serve(Shedder())
        calls = (
            lambda oid: node.set_load_control_shed_levels_local(oid, []),
            lambda oid: node.set_load_control_full_duty_baseline_local(oid, None),
            lambda oid: node.set_load_control_state_description_local(oid, None),
        )
        async with asyncio.timeout(10):
            for call in calls:
                with self.assertRaises(BacnetProtocolError) as caught:
                    await call(ObjectIdentifier(ObjectType.LOAD_CONTROL, 9))
                assert_code(self, caught.exception, ErrorClass.OBJECT, ErrorCode.UNKNOWN_OBJECT)
                with self.assertRaises(BacnetProtocolError) as caught:
                    await call(AV1)
                assert_code(self, caught.exception, ErrorClass.OBJECT,
                            ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED)

    async def test_shed_cannot_call_a_setter(self):
        failures = []

        def shed(request):
            try:
                node.set_load_control_state_description_local(LC1, "from shed")
            except RuntimeError as error:
                failures.append(str(error))
            return ("compliant", level(2), level(2))

        node, client, address = await self.serve(Shedder(shed))
        async with asyncio.timeout(10):
            await self.request(client, address)
            self.assertEqual(await self.state(node), COMPLIANT)
        self.assertTrue(failures)
        self.assertIn("cannot start a binding operation", failures[0])


class StorageTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_request_survives_a_restart_and_the_configuration_does_not(self):
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "lc1.bin")
            first = Shedder()
            node = server()
            node.add_load_control(
                1, "LC-1", shed_levels=[(1, "lights"), (2, "chillers")],
                shed=first.shed, stop=first.stop, storage_path=path)
            await node.start()
            client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
            await client.__aenter__()
            try:
                async with asyncio.timeout(10):
                    address = await node.local_address()
                    await client.write_property(address, LC1, P.REQUESTED_SHED_LEVEL,
                                                V.application_data(b"\x19\x02"))
                    await client.write_property(address, LC1, P.SHED_DURATION,
                                                V.unsigned(3 * 24 * 60))
                    await client.write_property(address, LC1, P.START_TIME, yesterday())
                    self.assertEqual((await node.read_property(LC1, P.PRESENT_VALUE)).value,
                                     COMPLIANT)
                    # Configuration set on the running object is not saved.
                    await node.set_load_control_state_description_local(LC1, "Shedding")
            finally:
                await client.stop()
                await node.stop()
            self.assertTrue(os.path.getsize(path) > 0)

            # The device comes back over the same file: the request is
            # decided again as the server starts, and `shed` hears of it.
            second = Shedder()
            node = server()
            node.add_load_control(
                1, "LC-1", shed_levels=[(1, "lights"), (2, "chillers")],
                shed=second.shed, stop=second.stop, storage_path=path)
            await node.start()
            try:
                async with asyncio.timeout(10):
                    self.assertEqual((await node.read_property(LC1, P.PRESENT_VALUE)).value,
                                     COMPLIANT)
                    self.assertTrue(second.requests)
                    self.assertEqual(second.requests[-1].requested, level(2))
                    self.assertEqual(second.requests[-1].shed_duration, 3 * 24 * 60)
                    self.assertEqual(await node.read_property(LC1, P.SHED_LEVELS),
                                     V.list([V.unsigned(1), V.unsigned(2)]))
                    with self.assertRaises(BacnetProtocolError) as caught:
                        await node.read_property(LC1, P.STATE_DESCRIPTION)
                    self.assertEqual(caught.exception.error_code,
                                     ErrorCode.UNKNOWN_PROPERTY.to_raw())
            finally:
                await node.stop()

    async def test_written_shed_levels_survive_a_restart_until_the_route_replaces_them(self):
        # #2502: the file keeps the elements a client wrote, which win over
        # the keyword's levels at the next start.
        keywords = [(1, "lights"), (2, "chillers"), (3, "pumps")]
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "lc1.bin")

            async def serve():
                node = server()
                node.add_load_control(1, "LC-1", shed_levels=keywords, storage_path=path)
                await node.start()
                return node

            node = await serve()
            client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
            await client.__aenter__()
            try:
                async with asyncio.timeout(10):
                    address = await node.local_address()
                    await client.write_property(address, LC1, P.SHED_LEVELS, V.unsigned(5),
                                                array_index=2)
            finally:
                await client.stop()
                await node.stop()

            node = await serve()
            try:
                async with asyncio.timeout(10):
                    self.assertEqual(await node.read_property(LC1, P.SHED_LEVELS),
                                     V.list([V.unsigned(1), V.unsigned(5), V.unsigned(3)]))
                    # The application's route replaces them, and the file
                    # drops them.
                    await node.set_load_control_shed_levels_local(LC1, keywords)
            finally:
                await node.stop()

            node = await serve()
            try:
                async with asyncio.timeout(10):
                    self.assertEqual(await node.read_property(LC1, P.SHED_LEVELS),
                                     V.list([V.unsigned(1), V.unsigned(2), V.unsigned(3)]))
            finally:
                await node.stop()

    async def test_a_file_that_is_not_this_objects_request_raises(self):
        with tempfile.TemporaryDirectory() as directory:
            garbage = os.path.join(directory, "other.bin")
            with open(garbage, "wb") as handle:
                handle.write(b"not a load control file")
            with self.assertRaises(BacnetError):
                server().add_load_control(1, "LC-1", storage_path=garbage)
            # A file another Load Control saved is refused too.
            theirs = os.path.join(directory, "lc1.bin")
            first = server()
            first.add_load_control(1, "LC-1", agreed_duty_window=15, storage_path=theirs)
            del first
            gc.collect()
            self.assertTrue(os.path.getsize(theirs) > 0)
            with self.assertRaises(BacnetError):
                server().add_load_control(2, "LC-2", storage_path=theirs)


if __name__ == "__main__":
    unittest.main()
