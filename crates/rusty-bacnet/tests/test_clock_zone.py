"""BACnetServer.set_clock_zone (#2395).

A host changes the Device clock's zone while the server runs. The Device's
UTC_Offset and Daylight_Savings_Status read the new pair, Local_Date and
Local_Time move by the difference, and a clock a UTCTimeSynchronization set
keeps the instant it was given. An offset the clock doesn't take is refused
when the method is called, and changes nothing.
"""

from __future__ import annotations

import ast
import asyncio
import datetime
import importlib.util
import socket
import unittest
from pathlib import Path

from rusty_bacnet import BACnetClient, BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier

DEVICE = 2395
UTC_TIME_SYNCHRONIZATION = 9
UTC = datetime.timezone.utc
P = PropertyIdentifier


def utc_sync_frame(when: datetime.datetime) -> bytes:
    """An unconfirmed UTCTimeSynchronization carrying `when` as Date, Time."""
    date = bytes([0xA4, when.year - 1900, when.month, when.day, when.isoweekday()])
    time = bytes([0xB4, when.hour, when.minute, when.second, when.microsecond // 10_000])
    npdu = b"\x01\x00" + bytes([0x10, UTC_TIME_SYNCHRONIZATION]) + date + time
    return b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu


class ClockZoneTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.server = BACnetServer(DEVICE, interface="127.0.0.1", port=0,
                                   broadcast_address="127.0.0.1")
        self.device = ObjectIdentifier(ObjectType.DEVICE, DEVICE)

    async def start(self) -> None:
        await self.server.start()

        async def stop() -> None:
            await self.server.stop()

        self.addAsyncCleanup(stop)
        self.address = await self.server.local_address()

    async def zone(self) -> tuple[int, bool]:
        """The Device's UTC_Offset and Daylight_Savings_Status."""
        offset = (await self.server.read_property(self.device, P.UTC_OFFSET)).value
        summer = (await self.server.read_property(self.device, P.DAYLIGHT_SAVINGS_STATUS)).value
        return offset, summer

    async def local(self) -> datetime.datetime:
        """The Device's Local_Date and Local_Time, as a naive datetime."""
        date = (await self.server.read_property(self.device, P.LOCAL_DATE)).value
        time = (await self.server.read_property(self.device, P.LOCAL_TIME)).value
        return datetime.datetime(date[0], date[1], date[2], time[0], time[1], time[2],
                                 time[3] * 10_000)

    async def synchronize(self, when: datetime.datetime) -> None:
        """Send a UTCTimeSynchronization and wait until the server ran it."""
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        sock.setblocking(False)
        self.addCleanup(sock.close)
        before = (await self.server.request_admission_counters())["unconfirmed_admitted_total"]
        ip, port = self.address.rsplit(":", 1)
        await asyncio.get_running_loop().sock_sendto(sock, utc_sync_frame(when), (ip, int(port)))
        async with asyncio.timeout(3):
            while True:
                counters = await self.server.request_admission_counters()
                if (counters["unconfirmed_admitted_total"] > before
                        and counters["unconfirmed_active"] == 0):
                    return
                await asyncio.sleep(0.01)

    async def test_a_zone_change_moves_local_time_and_keeps_the_sync(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "^server not started$"):
            await self.server.set_clock_zone(300, False)
        await self.start()
        self.assertEqual(await self.zone(), (0, False))
        # Thursday 2024-07-04, 13:15 UTC, a date the system clock isn't near.
        synced = datetime.datetime(2024, 7, 4, 13, 15, tzinfo=UTC)
        await self.synchronize(synced)
        tolerance = datetime.timedelta(seconds=30)
        utc = synced.replace(tzinfo=None)
        self.assertLess(abs(await self.local() - utc), tolerance)

        # Minutes west, daylight saving, and local time's distance from UTC:
        # five hours west; an hour ahead of that in daylight saving; an hour
        # east in daylight saving; 10:45 east, which is Friday there.
        for offset, summer, hours in ((300, False, -5), (300, True, -4), (-60, True, 2),
                                      (-645, False, 10.75)):
            with self.subTest(offset=offset, summer=summer):
                self.assertIsNone(await self.server.set_clock_zone(offset, summer))
                self.assertEqual(await self.zone(), (offset, summer))
                expected = utc + datetime.timedelta(hours=hours)
                self.assertLess(abs(await self.local() - expected), tolerance)
        self.assertEqual((await self.local()).date(), datetime.date(2024, 7, 5))

        # A peer reads the same pair.
        async with BACnetClient(interface="127.0.0.1", port=0) as client:
            offset = await client.read_property(self.address, self.device, P.UTC_OFFSET)
            summer = await client.read_property(self.address, self.device,
                                                P.DAYLIGHT_SAVINGS_STATUS)
            self.assertEqual((offset.value, summer.value), (-645, False))

    async def test_a_refused_zone_changes_nothing(self) -> None:
        await self.start()
        await self.server.set_clock_zone(-345, True)
        # Off the 15-minute grid, or more than a day: refused when called.
        for offset in (14, -1, 1441, -1455):
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                self.server.set_clock_zone(offset, False)
        # Past 16 bits the integer doesn't convert.
        with self.assertRaises(OverflowError):
            self.server.set_clock_zone(1 << 20, False)
        with self.assertRaises(TypeError):
            self.server.set_clock_zone("300", False)
        self.assertEqual(await self.zone(), (-345, True))

    def test_installed_stub_shape(self) -> None:
        spec = importlib.util.find_spec("rusty_bacnet")
        assert spec is not None and spec.origin is not None
        origin = Path(spec.origin)
        candidates = [origin.with_suffix(".pyi"), origin.parent / "rusty_bacnet.pyi"]
        stub = next(path for path in candidates if path.exists())
        classes = {node.name: node for node in ast.parse(stub.read_text()).body
                   if isinstance(node, ast.ClassDef)}
        method = next(node for node in classes["BACnetServer"].body
                      if isinstance(node, ast.FunctionDef) and node.name == "set_clock_zone")
        self.assertEqual([arg.arg for arg in method.args.args],
                         ["self", "utc_offset_minutes", "daylight_savings"])
        self.assertEqual([ast.unparse(arg.annotation) for arg in method.args.args[1:]],
                         ["int", "bool"])
        assert method.returns is not None
        self.assertEqual(ast.unparse(method.returns), "Awaitable[None]")


if __name__ == "__main__":
    unittest.main()
