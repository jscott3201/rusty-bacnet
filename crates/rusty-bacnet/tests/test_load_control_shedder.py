"""A Load Control built from Python (#2479): its shed levels, optional rows
and agreed duty window come from add_load_control, and the shed and stop
callables answer for the loads, under the server's lock, over real B/IP."""

import asyncio
import collections.abc
import copy
import datetime
import gc
import pickle
import sys
import unittest
import warnings

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetProtocolError,
    ErrorCode,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier as P,
    PropertyValue as V,
    ShedRequest,
)

LC1 = ObjectIdentifier(ObjectType.LOAD_CONTROL, 1)
INACTIVE, PENDING, COMPLIANT, NON_COMPLIANT = 0, 1, 2, 3
UNSPECIFIED_START = V.list([V.date(255, 255, 255, 255), V.time(255, 255, 255, 255)])


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
        if isinstance(self.answer, BaseException):
            raise self.answer
        return self.answer(request) if callable(self.answer) else self.answer

    def stop(self):
        self.stops += 1


class UnraisableCapture:
    """Collect what the binding reports through sys.unraisablehook."""

    def __enter__(self):
        self.seen = []
        self.previous = sys.unraisablehook
        sys.unraisablehook = self.seen.append
        return self

    def __exit__(self, *exc):
        sys.unraisablehook = self.previous


def server():
    return BACnetServer(503201, interface="127.0.0.1", port=0)


class ConstructorTests(unittest.TestCase):
    def test_keywords_are_checked(self):
        s = server()
        with self.assertRaisesRegex(TypeError, "shed must be callable"):
            s.add_load_control(1, "LC-1", shed=1)
        with self.assertRaisesRegex(TypeError, "stop must be callable"):
            s.add_load_control(1, "LC-1", shed=lambda request: None, stop="later")
        with self.assertRaisesRegex(ValueError, "stop needs shed"):
            s.add_load_control(1, "LC-1", stop=lambda: None)
        with self.assertRaises(TypeError):
            s.add_load_control(1, "LC-1", shed_levels="12")
        # Level 0 is no shed, which Shed_Levels doesn't hold; a baseline is
        # finite and not negative.
        for keywords in (
            {"shed_levels": [(1, "lights"), (0, "none")]},
            {"full_duty_baseline": -1.0},
            {"full_duty_baseline": float("nan")},
            {"full_duty_baseline": 1e300},
        ):
            with self.subTest(keywords=keywords):
                with self.assertRaises(BacnetProtocolError) as caught:
                    s.add_load_control(1, "LC-1", **keywords)
                self.assertEqual(caught.exception.error_code, ErrorCode.VALUE_OUT_OF_RANGE)


class LoadControlTests(unittest.IsolatedAsyncioTestCase):
    async def serve(self, shedder, **keywords):
        node = server()
        node.add_load_control(
            1, "LC-1",
            shed_levels=[(1, "lights"), (2, "chillers"), (3, "pumps")],
            shed=shedder.shed, stop=shedder.stop, **keywords)
        await node.start()

        async def stop(owner):
            await owner.stop()
        self.addAsyncCleanup(stop, node)
        client = BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000)
        await client.__aenter__()
        self.addAsyncCleanup(stop, client)
        return node, client, await node.local_address()

    async def request(self, client, address, requested=b"\x19\x02"):
        """Level 2 (or `requested`) for three days from 00:00 yesterday."""
        await client.write_property(address, LC1, P.REQUESTED_SHED_LEVEL,
                                    V.application_data(requested))
        await client.write_property(address, LC1, P.SHED_DURATION, V.unsigned(3 * 24 * 60))
        await client.write_property(address, LC1, P.START_TIME, yesterday())

    async def state(self, node):
        return (await node.read_property(LC1, P.PRESENT_VALUE)).value

    async def until_state(self, node, state):
        """Wait for the server's once-a-second pass to reach `state`."""
        while await self.state(node) != state:
            await asyncio.sleep(0.05)

    async def test_rows_come_from_the_keywords(self):
        node, _, _ = await self.serve(
            Shedder(), full_duty_baseline=250.5, state_description="Normal",
            agreed_duty_window=15)
        async with asyncio.timeout(10):
            self.assertEqual(await node.read_property(LC1, P.SHED_LEVELS),
                             V.list([V.unsigned(1), V.unsigned(2), V.unsigned(3)]))
            self.assertEqual(
                await node.read_property(LC1, P.SHED_LEVEL_DESCRIPTIONS),
                V.list([V.character_string(text) for text in ("lights", "chillers", "pumps")]))
            self.assertEqual(await node.read_property(LC1, P.FULL_DUTY_BASELINE), V.real(250.5))
            self.assertEqual(await node.read_property(LC1, P.STATE_DESCRIPTION),
                             V.character_string("Normal"))
            self.assertEqual(await node.read_property(LC1, P.DUTY_WINDOW), V.unsigned(15))

    async def test_optional_rows_are_absent_unless_given(self):
        node, _, _ = await self.serve(Shedder())
        async with asyncio.timeout(10):
            for prop in (P.FULL_DUTY_BASELINE, P.STATE_DESCRIPTION):
                with self.assertRaises(BacnetProtocolError) as caught:
                    await node.read_property(LC1, prop)
                self.assertEqual(caught.exception.error_code, ErrorCode.UNKNOWN_PROPERTY)

    async def test_a_request_is_shed_and_stopped_through_the_callables(self):
        shedder = Shedder()
        node, client, address = await self.serve(shedder, full_duty_baseline=250.0,
                                                 agreed_duty_window=15)
        async with asyncio.timeout(10):
            await self.request(client, address)
            self.assertEqual(await self.state(node), COMPLIANT)
            request = shedder.requests[-1]
            self.assertIsInstance(request, ShedRequest)
            self.assertEqual(request.requested, level(2))
            # Index 1 of shed_levels, the level 2 entry.
            self.assertEqual(request.shed_level, 1)
            self.assertEqual(request.shed_duration, 3 * 24 * 60)
            self.assertEqual(request.duty_window, 15)
            self.assertEqual(request.full_duty_baseline, 250.0)
            self.assertTrue(request.started)
            self.assertEqual(request.start_time[1], (0, 0, 0, 0))
            self.assertEqual(
                await client.read_property(address, LC1, P.ACTUAL_SHED_LEVEL),
                await node.read_property(LC1, P.REQUESTED_SHED_LEVEL))
            self.assertIn("ShedRequest(", repr(request))
            for clone in (pickle.dumps, copy.copy):
                with self.assertRaises(TypeError):
                    clone(request)
            # The loads fall short: the server's pass hears it.
            shedder.answer = ("shedding", level(2), level(1))
            await self.until_state(node, NON_COMPLIANT)
            self.assertEqual(shedder.stops, 0)
            # The unspecified Start_Time cancels it: stop runs once.
            await client.write_property(address, LC1, P.START_TIME, UNSPECIFIED_START)
            self.assertEqual(await self.state(node), INACTIVE)
            self.assertEqual(shedder.stops, 1)

    async def test_deleting_the_object_during_a_request_stops_the_loads(self):
        shedder = Shedder()
        node, client, address = await self.serve(shedder)
        async with asyncio.timeout(10):
            await self.request(client, address)
            self.assertEqual(await self.state(node), COMPLIANT)
            await client.delete_object(address, LC1)
            self.assertEqual(shedder.stops, 1)

    async def test_a_failing_shed_counts_as_nothing_shed(self):
        cases = (
            ("raises", RuntimeError("loads offline"), RuntimeError),
            ("wrong shape", lambda request: "compliant", TypeError),
            ("wrong progress", lambda request: ("done", level(2), level(2)), ValueError),
            ("wrong level", lambda request: ("compliant", {"kind": "watts"}, level(2)), ValueError),
        )
        for label, answer, error in cases:
            with self.subTest(label):
                shedder = Shedder(answer)
                node, client, address = await self.serve(shedder)
                with UnraisableCapture() as capture:
                    async with asyncio.timeout(10):
                        await self.request(client, address)
                        # Nothing shed after Start_Time: non-compliant, and
                        # the levels at the choice's default.
                        self.assertEqual(await self.state(node), NON_COMPLIANT)
                        self.assertEqual(
                            await client.read_property(address, LC1, P.ACTUAL_SHED_LEVEL),
                            await client.read_property(address, LC1, P.EXPECTED_SHED_LEVEL))
                self.assertTrue(capture.seen)
                self.assertIsInstance(capture.seen[0].exc_value, error)

    async def test_a_coroutine_shed_is_refused_and_closed(self):
        async def shed(request):
            return ("compliant", level(2), level(2))

        shedder = Shedder(shed)
        node, client, address = await self.serve(shedder)
        with UnraisableCapture() as capture:
            async with asyncio.timeout(10):
                await self.request(client, address)
                self.assertEqual(await self.state(node), NON_COMPLIANT)
        self.assertIsInstance(capture.seen[0].exc_value, TypeError)
        self.assertIn("coroutine", str(capture.seen[0].exc_value))

    async def test_a_failing_stop_still_ends_the_request(self):
        shedder = Shedder()

        def stop():
            shedder.stops += 1
            raise RuntimeError("relay stuck")
        shedder.stop = stop
        node, client, address = await self.serve(shedder)
        with UnraisableCapture() as capture:
            async with asyncio.timeout(10):
                await self.request(client, address)
                await client.write_property(address, LC1, P.START_TIME, UNSPECIFIED_START)
                self.assertEqual(await self.state(node), INACTIVE)
        self.assertEqual(shedder.stops, 1)
        self.assertIsInstance(capture.seen[0].exc_value, RuntimeError)

    async def test_a_callable_cannot_start_a_binding_operation(self):
        failures = []

        def shed(request):
            try:
                node.read_property(LC1, P.PRESENT_VALUE)
            except RuntimeError as error:
                failures.append(str(error))
            return ("compliant", level(2), level(2))

        node, client, address = await self.serve(Shedder(shed))
        async with asyncio.timeout(10):
            await self.request(client, address)
        self.assertTrue(failures)
        self.assertIn("cannot start a binding operation", failures[0])

    async def test_reading_the_answer_cannot_start_a_binding_operation_either(self):
        failures = []

        class Level(collections.abc.Mapping):
            """A level whose reading tries a binding operation."""

            def __init__(self):
                self.items = level(2)

            def __getitem__(self, key):
                try:
                    node.read_property(LC1, P.PRESENT_VALUE)
                except RuntimeError as error:
                    failures.append(str(error))
                return self.items[key]

            def __iter__(self):
                return iter(self.items)

            def __len__(self):
                return len(self.items)

        node, client, address = await self.serve(
            Shedder(lambda request: ("compliant", Level(), level(2))))
        async with asyncio.timeout(10):
            await self.request(client, address)
            self.assertEqual(await self.state(node), COMPLIANT)
        self.assertTrue(failures)
        self.assertIn("cannot start a binding operation", failures[0])

    async def test_a_coroutine_stop_is_refused_and_closed(self):
        shedder = Shedder()
        stopped = []

        async def stop():
            stopped.append(True)
        shedder.stop = stop
        node, client, address = await self.serve(shedder)
        with UnraisableCapture() as capture, warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            async with asyncio.timeout(10):
                await self.request(client, address)
                await client.write_property(address, LC1, P.START_TIME, UNSPECIFIED_START)
                # The request ends all the same.
                self.assertEqual(await self.state(node), INACTIVE)
            gc.collect()
        self.assertIsInstance(capture.seen[0].exc_value, TypeError)
        self.assertIn("coroutine", str(capture.seen[0].exc_value))
        # Closed, never run, and never warned about as never awaited.
        self.assertEqual(stopped, [])
        self.assertFalse([w for w in caught if issubclass(w.category, RuntimeWarning)])


if __name__ == "__main__":
    unittest.main()
