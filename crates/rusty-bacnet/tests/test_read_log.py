"""Paged log reads, typed records, By-Time and lenient ReadRange from Python
(#1530, #1531, #1533, #1534, #1535).

An Event Log that collects received notifications fills deterministically:
each raw UnconfirmedEventNotification sent to the server adds one record.
A scripted UDP device stands in for one that numbers its first record after
a sequence wrap 0.
"""

from __future__ import annotations

import asyncio
import datetime
import inspect
import json
import socket
import time
import unittest

import rusty_bacnet
from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BacnetError,
    BacnetLogNotAdvancingError,
    BacnetReadRangeViolationError,
    BipEndpoint,
    EndpointClient,
    EventType,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
)

LOG = ObjectIdentifier(ObjectType.EVENT_LOG, 1)
TREND_LOG = ObjectIdentifier(ObjectType.TREND_LOG, 1)
WAIT = 5.0
# The server logs at most five received notifications a second from one
# source, so five fill the log at once.
NOTIFICATIONS = 5

# Device 50's OUT_OF_RANGE alarm about its Analog Input 3: process 7,
# sequence-number timestamp 9, class 4, priority 100, ALARM, no ack, NORMAL
# to HIGH_LIMIT, no event values; in a B/IP Original-Unicast-NPDU.
NOTIFICATION = bytes.fromhex(
    "09 07 1c 02000032 2c 00000003 3e 19 09 3f 49 04 59 64 69 05"
    " 89 00 99 00 a9 00 b9 03"
)
APDU = bytes.fromhex("01 00 10 03") + NOTIFICATION
FRAME = bytes.fromhex("81 0a") + (4 + len(APDU)).to_bytes(2, "big") + APDU

# One Trend Log record: 2026-10-05 (Monday) 09:00:00.00, unsigned 7.
TREND_RECORD = bytes.fromhex("0e a4 7e 0a 05 01 b4 09 00 00 00 0f 1e 49 07 1f")


def wrapped_page_ack(invoke_id: int) -> bytes:
    """A ComplexAck to ReadRange of Trend Log 1: one record, first and last
    item, first sequence number 0."""
    service = (
        bytes.fromhex("0c 05000001 19 83 3a 05 c0 49 01 5e")
        + TREND_RECORD
        + bytes.fromhex("5f 69 00")
    )
    apdu = bytes([0x30, invoke_id, 0x1A]) + service
    npdu = bytes([0x01, 0x00]) + apdu
    return bytes([0x81, 0x0A]) + (4 + len(npdu)).to_bytes(2, "big") + npdu


async def serve_wrapped_pages(sock: socket.socket, answers: int) -> None:
    """Answer `answers` confirmed requests with the wrapped page."""
    loop = asyncio.get_running_loop()
    for _ in range(answers):
        frame, peer = await loop.sock_recvfrom(sock, 1500)
        # BVLC (4 octets), NPDU 01 04 (expecting reply), then the
        # confirmed-request header: type, max segments/APDU, invoke ID.
        invoke_id = frame[8]
        await loop.sock_sendto(sock, wrapped_page_ack(invoke_id), peer)


class ReadLogLiveTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.server = BACnetServer(
            1_530_000, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1"
        )
        self.server.add_event_log(1, "EL-1", 64, log_received_notifications=True)
        await self.server.start()
        self.address = await self.server.local_address()
        ip, port = self.address.split(":")
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            for _ in range(NOTIFICATIONS):
                await loop.sock_sendto(peer, FRAME, (ip, int(port)))
        async with BACnetClient(interface="127.0.0.1", port=0) as client:
            async with asyncio.timeout(WAIT):
                while True:
                    result = await client.read_range(
                        self.address, LOG, PropertyIdentifier.LOG_BUFFER
                    )
                    if result["item_count"] >= NOTIFICATIONS:
                        break
                    await asyncio.sleep(0.05)
        self.everything = rusty_bacnet.decode_log_records(result)

    async def asyncTearDown(self) -> None:
        await self.server.stop()

    async def read_all(self, reader, cursor=None) -> tuple[list[dict], list[dict]]:
        records: list[dict] = []
        pages: list[dict] = []
        for _ in range(20):
            page = await reader.read_log_page(self.address, LOG, cursor, page_size=2)
            records.extend(page["records"])
            pages.append(page)
            cursor = page["next"]
            if page["done"]:
                return records, pages
        self.fail("the read did not finish")

    async def test_pages_through_the_whole_log_and_resumes_from_the_checkpoint(self) -> None:
        async with BACnetClient(
            interface="127.0.0.1", port=0, min_request_interval_ms=20
        ) as client:
            started = time.monotonic()
            records, pages = await self.read_all(client)
            elapsed = time.monotonic() - started
        self.assertEqual(records, self.everything)
        notifications = [r for r in records if r["datum"]["kind"] == "notification"]
        self.assertEqual(len(notifications), NOTIFICATIONS)
        first = notifications[0]["datum"]["notification"]
        self.assertEqual(first["process_identifier"], 7)
        self.assertEqual(first["event_type"], EventType.OUT_OF_RANGE)
        self.assertIsNone(first["event_values"])
        self.assertEqual(pages[0]["first_sequence_number"], 1)
        self.assertTrue(pages[0]["result_flags"][0])
        self.assertTrue(all(page["gap"] is None for page in pages))
        self.assertTrue(all(page["violations"] == [] for page in pages))
        self.assertTrue(all(page["wrapped"] is False for page in pages))
        self.assertEqual(pages[-1]["next"], ("sequence", len(records) + 1))
        # Three count reads and a ReadRange a page, each sent at least 20 ms
        # after the one before it was answered.
        requests = 3 + len(pages)
        self.assertGreaterEqual(elapsed, 0.02 * (requests - 1))

        # A checkpoint survives JSON as a list; nothing new reads as done.
        async with BACnetClient(interface="127.0.0.1", port=0) as client:
            page = await client.read_log_page(
                self.address, LOG, list(pages[-1]["next"]), page_size=2
            )
        self.assertTrue(page["done"])
        self.assertEqual(page["records"], [])
        self.assertEqual(page["next"], pages[-1]["next"])

    async def test_the_endpoint_client_reads_the_same_pages(self) -> None:
        endpoint = BipEndpoint(device_instance=1_530_001, interface="127.0.0.1", port=0)
        await endpoint.start()
        try:
            role = await endpoint.client()
            records, _ = await self.read_all(role)
            self.assertEqual(records, self.everything)
        finally:
            await endpoint.close()

    async def test_a_time_cursor_survives_json(self) -> None:
        notifications = [
            r for r in self.everything if r["datum"]["kind"] == "notification"
        ]
        async with BACnetClient(interface="127.0.0.1", port=0) as client:
            # Nothing is newer than the last record: an empty page whose next
            # cursor is the same time, ready to persist.
            last = self.everything[-1]["timestamp"]
            empty = await client.read_log_page(self.address, LOG, ("time", last))
            self.assertTrue(empty["done"])
            self.assertEqual(empty["records"], [])
            self.assertEqual(empty["next"], ("time", last))
            stored = json.loads(json.dumps(empty["next"]))
            self.assertIsInstance(stored[1][0], list)
            again = await client.read_log_page(self.address, LOG, stored)
            self.assertEqual(again["next"], ("time", last))
            # A time cursor with records reads the same through JSON.
            first = ("time", notifications[0]["timestamp"])
            direct = await client.read_log_page(self.address, LOG, first, page_size=100)
            through_json = await client.read_log_page(
                self.address, LOG, json.loads(json.dumps(first)), page_size=100
            )
            self.assertEqual(direct["records"], through_json["records"])
            # read_range takes the nested-list form too.
            listed = await client.read_range(
                self.address,
                LOG,
                PropertyIdentifier.LOG_BUFFER,
                range_type="time",
                reference_time=json.loads(json.dumps(notifications[0]["timestamp"])),
                count=100,
            )
            self.assertEqual(rusty_bacnet.decode_log_records(listed), direct["records"])

    async def test_by_time_reads_the_records_newer_than_the_reference(self) -> None:
        notifications = [
            r for r in self.everything if r["datum"]["kind"] == "notification"
        ]
        (year, month, day, _), (hour, minute, second, hundredths) = notifications[0][
            "timestamp"
        ]
        naive = datetime.datetime(year, month, day, hour, minute, second, hundredths * 10_000)
        async with BACnetClient(interface="127.0.0.1", port=0) as client:
            results = []
            for reference in (notifications[0]["timestamp"], naive):
                results.append(
                    await client.read_range(
                        self.address,
                        LOG,
                        PropertyIdentifier.LOG_BUFFER,
                        range_type="time",
                        reference_time=reference,
                        count=100,
                    )
                )
            self.assertEqual(results[0], results[1])
            newer = rusty_bacnet.decode_log_records(results[0])
            self.assertTrue(all(r["timestamp"] > notifications[0]["timestamp"] for r in newer))
            if newer:
                self.assertIsNotNone(results[0]["first_sequence_number"])
            page = await client.read_log_page(
                self.address, LOG, ("time", notifications[0]["timestamp"]), page_size=100
            )
            self.assertEqual(page["records"], newer)

            aware = naive.replace(tzinfo=datetime.timezone.utc)
            for options in (
                {"range_type": "time", "count": 1},
                {"range_type": "time", "count": 1, "reference_time": aware},
                {"range_type": "time", "count": 1, "reference_time": ((2026, 10, 5, 255), (9, 0, 0, 0))},
                {"validation": "loose"},
            ):
                with self.subTest(options=options):
                    with self.assertRaises(ValueError):
                        client.read_range("not-an-address", LOG, PropertyIdentifier.LOG_BUFFER, **options)


class WrappedDeviceTests(unittest.IsolatedAsyncioTestCase):
    async def test_strict_refuses_and_lenient_keeps_a_zero_first_sequence_page(self) -> None:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as device:
            device.bind(("127.0.0.1", 0))
            device.setblocking(False)
            address = "127.0.0.1:%d" % device.getsockname()[1]
            serving = asyncio.ensure_future(serve_wrapped_pages(device, 3))
            try:
                async with BACnetClient(
                    interface="127.0.0.1", port=0, apdu_timeout_ms=2000
                ) as client:
                    options = {"range_type": "sequence", "reference_seq": 1, "count": 5}
                    with self.assertRaises(BacnetReadRangeViolationError) as refused:
                        await client.read_range(address, TREND_LOG, PropertyIdentifier.LOG_BUFFER, **options)
                    self.assertIn("first sequence number 0", str(refused.exception))
                    self.assertEqual(refused.exception.rule, "zero_first_sequence_number")
                    self.assertIsInstance(refused.exception, BacnetError)
                    kept = await client.read_range(
                        address, TREND_LOG, PropertyIdentifier.LOG_BUFFER,
                        validation="lenient", **options,
                    )
                    self.assertEqual(kept["violations"], ["zero_first_sequence_number"])
                    self.assertEqual(kept["first_sequence_number"], 0)
                    [record] = rusty_bacnet.decode_log_records(kept)
                    self.assertEqual(record["datum"], {"kind": "unsigned", "unsigned": 7})
                    self.assertEqual(record["timestamp"], ((2026, 10, 5, 1), (9, 0, 0, 0)))
                    self.assertIsNone(record["status_flags"])
                    # The pager reads leniently and lists the rule.
                    page = await client.read_log_page(address, TREND_LOG, ("sequence", 1))
                    self.assertEqual(page["violations"], ["zero_first_sequence_number"])
                    self.assertEqual(page["next"], ("sequence", 1))
                    self.assertTrue(page["done"])
            finally:
                serving.cancel()


class ReadLogContractTests(unittest.TestCase):
    def test_client_and_endpoint_share_signatures(self) -> None:
        for name in ("read_range", "read_log_page"):
            with self.subTest(method=name):
                self.assertEqual(
                    inspect.signature(getattr(EndpointClient, name)),
                    inspect.signature(getattr(BACnetClient, name)),
                )
        parameters = inspect.signature(BACnetClient.read_range).parameters
        self.assertEqual(parameters["reference_time"].kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertEqual(parameters["validation"].default, "strict")
        constructor = inspect.signature(BACnetClient).parameters
        self.assertEqual(constructor["min_request_interval_ms"].default, 0)
        self.assertTrue(issubclass(BacnetLogNotAdvancingError, BacnetError))

    def test_bad_pages_fail_before_io(self) -> None:
        client = BACnetClient(interface="127.0.0.1", port=0)
        analog = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        for object_id, cursor, page_size in (
            (analog, None, 10),
            (LOG, None, 0),
            (LOG, None, 32_768),
            (LOG, None, -1),
            (LOG, None, 1 << 70),
            (LOG, ("backwards", 1), 10),
            (LOG, "newest", 10),
        ):
            with self.subTest(object_id=object_id, cursor=cursor, page_size=page_size):
                with self.assertRaises(ValueError):
                    client.read_log_page("not-an-address", object_id, cursor, page_size)

    def test_page_size_must_be_an_int_and_interval_at_most_an_hour(self) -> None:
        client = BACnetClient(interface="127.0.0.1", port=0)
        with self.assertRaises(TypeError):
            client.read_log_page("not-an-address", LOG, None, "10")
        BACnetClient(interface="127.0.0.1", port=0, min_request_interval_ms=3_600_000)
        with self.assertRaises(ValueError):
            BACnetClient(interface="127.0.0.1", port=0, min_request_interval_ms=3_600_001)

    def test_decode_log_records_refuses_other_results_and_names_the_record(self) -> None:
        result = {
            "object_id": TREND_LOG,
            "property_id": PropertyIdentifier.LOG_BUFFER,
            "item_count": 2,
            "item_data": TREND_RECORD + b"\x0e",
        }
        with self.assertRaisesRegex(ValueError, "log record 1 at item-data offset 16"):
            rusty_bacnet.decode_log_records(result)
        result["item_count"] = 1
        result["item_data"] = TREND_RECORD
        self.assertEqual(len(rusty_bacnet.decode_log_records(result)), 1)
        result["object_id"] = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        with self.assertRaises(ValueError):
            rusty_bacnet.decode_log_records(result)


if __name__ == "__main__":
    unittest.main()
