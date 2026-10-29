"""Decoded returns in place of raw octets (#2080): the CreateObject ACK as an
ObjectIdentifier, AtomicReadFile as a dict, bit strings as BitString with
named bits, and enumerated values with names.

A raw UDP peer answers with exact ACK octets, so each decoder is checked
against the wire; a rusty-bacnet server covers the same calls end to end."""
import asyncio
import copy
import pickle
import socket
import unittest

from rusty_bacnet import (
    BACnetClient,
    BACnetServer,
    BACnetTimeStamp,
    BacnetError,
    BitString,
    EventState,
    NotifyType,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
    decode_log_records,
    enum_name,
)

AV7 = ObjectIdentifier(ObjectType.ANALOG_VALUE, 7)


class RawPeerTests(unittest.IsolatedAsyncioTestCase):
    async def answered(self, peer, operation, body):
        """Answer the request `operation` sends with a ComplexACK carrying
        `body`, and return what the call returns."""
        loop = asyncio.get_running_loop()
        packet, sender = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 2)
        npdu = b"\x01\x00" + bytes((0x30, packet[8], packet[9])) + body
        await loop.sock_sendto(
            peer, b"\x81\x0a" + (4 + len(npdu)).to_bytes(2, "big") + npdu, sender
        )
        return await asyncio.wait_for(operation, 2)

    async def asyncSetUp(self):
        self.peer = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.addCleanup(self.peer.close)
        self.peer.bind(("127.0.0.1", 0))
        self.peer.setblocking(False)
        self.address = f"127.0.0.1:{self.peer.getsockname()[1]}"
        self.client = BACnetClient(interface="127.0.0.1", port=0)
        await self.client.__aenter__()

        async def stop_client():
            await self.client.__aexit__(None, None, None)

        self.addAsyncCleanup(stop_client)

    async def test_create_object_returns_the_acknowledged_identifier(self):
        created = await self.answered(
            self.peer,
            self.client.create_object(self.address, ObjectType.ANALOG_VALUE),
            bytes.fromhex("c400800007"),
        )
        self.assertIsInstance(created, ObjectIdentifier)
        self.assertEqual(created, AV7)

    async def test_create_object_ack_that_does_not_decode_raises(self):
        # An application Unsigned where the identifier belongs.
        with self.assertRaises(BacnetError):
            await self.answered(
                self.peer,
                self.client.create_object(self.address, ObjectType.ANALOG_VALUE),
                bytes.fromhex("2107"),
            )

    async def test_alarm_summary_names_acknowledged_transitions(self):
        # (ANALOG_INPUT, 1), HIGH_LIMIT, bit string 101 (five unused bits).
        summary = await self.answered(
            self.peer,
            self.client.get_alarm_summary(self.address),
            bytes.fromhex("c4000000019103" "8205a0"),
        )
        self.assertEqual(len(summary), 1)
        entry = summary[0]
        self.assertEqual(entry["object_id"], ObjectIdentifier(ObjectType.ANALOG_INPUT, 1))
        self.assertEqual(entry["alarm_state"], EventState.HIGH_LIMIT)
        self.assertEqual(entry["alarm_state"].name, "HIGH_LIMIT")
        transitions = entry["acknowledged_transitions"]
        self.assertIsInstance(transitions, BitString)
        self.assertEqual((transitions.unused_bits, transitions.data), (5, b"\xa0"))
        self.assertEqual(transitions.bits, [True, False, True])
        self.assertEqual(transitions.names, ["TO_OFFNORMAL", "TO_NORMAL"])

    async def test_event_information_is_decoded(self):
        # One summary for (ANALOG_INPUT, 1): HIGH_LIMIT, acked 011, the
        # three timestamp choices (time, sequence 42, date and time),
        # notify type ALARM, enable 111, priorities 3, 5, 200; More_Events
        # TRUE.
        body = bytes.fromhex(
            "0e"
            "0c00000001" "1903" "2a0560"
            "3e" "0c07000000" "192a" "2ea47e0a1907b40e1e05002f" "3f"
            "4900" "5a05e0"
            "6e" "2103" "2105" "21c8" "6f"
            "0f" "1901"
        )
        result = await self.answered(
            self.peer, self.client.get_event_information(self.address), body
        )
        self.assertEqual(set(result), {"summaries", "more_events"})
        self.assertIs(result["more_events"], True)
        self.assertEqual(len(result["summaries"]), 1)
        summary = result["summaries"][0]
        self.assertEqual(
            set(summary),
            {
                "object_id",
                "event_state",
                "acknowledged_transitions",
                "event_timestamps",
                "notify_type",
                "event_enable",
                "event_priorities",
            },
        )
        self.assertEqual(summary["object_id"], ObjectIdentifier(ObjectType.ANALOG_INPUT, 1))
        self.assertEqual(summary["event_state"], EventState.HIGH_LIMIT)
        acked = summary["acknowledged_transitions"]
        self.assertIsInstance(acked, BitString)
        self.assertEqual(acked.bits, [False, True, True])
        self.assertEqual(acked.names, ["TO_FAULT", "TO_NORMAL"])
        self.assertEqual(
            summary["event_enable"].names, ["TO_OFFNORMAL", "TO_FAULT", "TO_NORMAL"]
        )
        self.assertEqual(
            summary["event_timestamps"],
            [
                BACnetTimeStamp.time(7, 0, 0, 0),
                BACnetTimeStamp.sequence_number(42),
                BACnetTimeStamp.date_time((2026, 10, 25, 7), (14, 30, 5, 0)),
            ],
        )
        self.assertEqual(summary["notify_type"], NotifyType.ALARM)
        self.assertEqual(summary["notify_type"].name, "ALARM")
        self.assertEqual(summary["event_priorities"], [3, 5, 200])

    async def test_event_information_that_does_not_decode_raises(self):
        with self.assertRaises(BacnetError):
            await self.answered(
                self.peer,
                self.client.get_event_information(self.address),
                bytes.fromhex("0e0c0000"),
            )

    async def test_atomic_read_file_stream_ack_is_decoded(self):
        file = ObjectIdentifier(ObjectType.FILE, 1)
        # end-of-file FALSE, [0] { start 5, "abc" }.
        result = await self.answered(
            self.peer,
            self.client.atomic_read_file(
                self.address, file, "stream", start_position=5, requested_octet_count=64
            ),
            bytes.fromhex("10" "0e" "3105" "63616263" "0f"),
        )
        self.assertEqual(
            result,
            {"access_method": "stream", "end_of_file": False, "start_position": 5,
             "data": b"abc"},
        )

    async def test_atomic_read_file_record_ack_is_decoded(self):
        file = ObjectIdentifier(ObjectType.FILE, 1)
        # end-of-file TRUE, [1] { start 2, count 2, "one", "two" }.
        result = await self.answered(
            self.peer,
            self.client.atomic_read_file(
                self.address, file, "record", start_record=2, requested_record_count=10
            ),
            bytes.fromhex("11" "1e" "3102" "2102" "636f6e65" "6374776f" "1f"),
        )
        self.assertEqual(
            result,
            {"access_method": "record", "end_of_file": True, "start_record": 2,
             "record_count": 2, "records": [b"one", b"two"]},
        )

    async def test_atomic_write_file_acks_are_decoded(self):
        file = ObjectIdentifier(ObjectType.FILE, 1)
        # [0] start position 12.
        result = await self.answered(
            self.peer,
            self.client.atomic_write_file(self.address, file, "stream", file_data=b"x"),
            bytes.fromhex("090c"),
        )
        self.assertEqual(result, {"access_method": "stream", "start_position": 12})
        # [1] start record 3.
        result = await self.answered(
            self.peer,
            self.client.atomic_write_file(
                self.address, file, "record", record_count=1, file_record_data=[b"r"]
            ),
            bytes.fromhex("1903"),
        )
        self.assertEqual(result, {"access_method": "record", "start_record": 3})
        # A record ACK to a stream write raises.
        with self.assertRaises(BacnetError):
            await self.answered(
                self.peer,
                self.client.atomic_write_file(self.address, file, "stream", file_data=b"x"),
                bytes.fromhex("1903"),
            )
        # And a stream ACK to a record write.
        with self.assertRaises(BacnetError):
            await self.answered(
                self.peer,
                self.client.atomic_write_file(
                    self.address, file, "record", record_count=1, file_record_data=[b"r"]
                ),
                bytes.fromhex("090c"),
            )

    async def test_read_range_result_flags_are_named(self):
        trend = ObjectIdentifier(ObjectType.TREND_LOG, 1)
        # (TREND_LOG, 1), LOG_BUFFER, flags 110, count 0, empty item data.
        result = await self.answered(
            self.peer,
            self.client.read_range(self.address, trend, PropertyIdentifier.LOG_BUFFER),
            bytes.fromhex("0c05000001" "19 83" "3a05c0" "4900" "5e5f".replace(" ", "")),
        )
        flags = result["result_flags"]
        self.assertIsInstance(flags, BitString)
        self.assertEqual(flags.bits, [True, True, False])
        self.assertEqual(flags.names, ["FIRST_ITEM", "LAST_ITEM"])
        self.assertTrue(flags[0])

    async def test_atomic_read_file_ack_in_the_other_access_method_raises(self):
        file = ObjectIdentifier(ObjectType.FILE, 1)
        with self.assertRaises(BacnetError):
            await self.answered(
                self.peer,
                self.client.atomic_read_file(
                    self.address, file, "record", requested_record_count=10
                ),
                bytes.fromhex("10" "0e" "3100" "63616263" "0f"),
            )


class ServerRoundTripTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.server = BACnetServer(9873, interface="127.0.0.1", port=0)
        self.server.add_analog_input(1, "AI")
        self.server.add_file(1, "Stream")
        self.server.set_file_data(1, b"hello world")
        await self.server.start()

        async def stop_server():
            await self.server.stop()

        self.addAsyncCleanup(stop_server)
        self.address = await self.server.local_address()
        self.client = BACnetClient(interface="127.0.0.1", port=0)
        await self.client.__aenter__()

        async def stop_client():
            await self.client.__aexit__(None, None, None)

        self.addAsyncCleanup(stop_client)

    async def test_create_object_by_type_returns_the_server_chosen_instance(self):
        created = await asyncio.wait_for(
            self.client.create_object(self.address, ObjectType.BINARY_VALUE), 3
        )
        self.assertEqual(created.object_type, ObjectType.BINARY_VALUE)
        name = await self.server.read_property(created, PropertyIdentifier.OBJECT_NAME)
        self.assertEqual(name.value, f"BINARY_VALUE-{created.instance}")

    async def test_atomic_read_file_stream_window(self):
        result = await asyncio.wait_for(
            self.client.atomic_read_file(
                self.address, ObjectIdentifier(ObjectType.FILE, 1), "stream",
                start_position=6, requested_octet_count=64,
            ),
            3,
        )
        self.assertEqual(
            result,
            {"access_method": "stream", "end_of_file": True, "start_position": 6,
             "data": b"world"},
        )

    async def test_reads_name_bits_and_enumerations(self):
        ai = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        await self.server.write_property_local(
            ai, PropertyIdentifier.OUT_OF_SERVICE, PropertyValue.boolean(True),
            source_object=None,
        )
        flags = (await asyncio.wait_for(
            self.client.read_property(self.address, ai, PropertyIdentifier.STATUS_FLAGS), 3
        )).value
        self.assertIsInstance(flags, BitString)
        self.assertEqual(flags.bits, [False, False, False, True])
        self.assertEqual(flags.names, ["OUT_OF_SERVICE"])

        state = await self.server.read_property(ai, PropertyIdentifier.EVENT_STATE)
        self.assertEqual((state.value, state.name), (0, "NORMAL"))
        kind = await self.server.read_property(ai, PropertyIdentifier.OBJECT_TYPE)
        self.assertEqual(kind.name, "ANALOG_INPUT")
        # A name needs a property whose enumeration is known.
        object_name = await self.server.read_property(ai, PropertyIdentifier.OBJECT_NAME)
        self.assertIsNone(object_name.name)

        # Copies and pickles keep the read's property, so the names too.
        for clone in (copy.copy(state), copy.deepcopy(state), pickle.loads(pickle.dumps(state))):
            self.assertEqual(clone, state)
            self.assertEqual(clone.name, "NORMAL")
        read_flags = await self.server.read_property(ai, PropertyIdentifier.STATUS_FLAGS)
        clone = pickle.loads(pickle.dumps(read_flags))
        self.assertEqual(clone.value.names, ["OUT_OF_SERVICE"])
        self.assertEqual(hash(clone), hash(PropertyValue.bit_string(4, b"\x10")))

        device = ObjectIdentifier(ObjectType.DEVICE, 9873)
        services = (await self.server.read_property(
            device, PropertyIdentifier.PROTOCOL_SERVICES_SUPPORTED
        )).value
        self.assertIn("READ_PROPERTY", services.names)
        self.assertEqual(len(services.bits), len(services))


class BitStringTests(unittest.TestCase):
    def test_shape_names_and_equality(self):
        named = BitString(5, b"\xa0", PropertyIdentifier.EVENT_ENABLE)
        plain = BitString(5, b"\xa0")
        self.assertEqual(named.names, ["TO_OFFNORMAL", "TO_NORMAL"])
        self.assertIsNone(plain.names)
        self.assertEqual(named, plain)
        self.assertEqual(hash(named), hash(plain))
        self.assertEqual(plain.bits, [True, False, True])
        self.assertEqual(len(plain), 3)
        self.assertNotEqual(plain, BitString(4, b"\xa0"))
        self.assertEqual(repr(named), "BitString(5, b'\\xa0')")
        self.assertEqual(repr(BitString(4, b"@")), "BitString(4, b'@')")
        self.assertEqual(eval(repr(named), {"BitString": BitString}), named)
        self.assertEqual((plain[0], plain[1], plain[-1]), (True, False, True))
        self.assertEqual(list(plain), [True, False, True])
        first, second, third = plain
        self.assertEqual((first, second, third), (True, False, True))
        for index in (3, -4):
            with self.assertRaises(IndexError):
                plain[index]
        self.assertEqual(BitString(0, b"").bits, [])
        # Status flags carry four bits; the AI's In_Alarm is the top one.
        self.assertEqual(
            BitString(4, b"\x80", PropertyIdentifier.STATUS_FLAGS).names, ["IN_ALARM"]
        )
        # A property of no known bit-string type gives no names.
        self.assertIsNone(BitString(0, b"\xff", PropertyIdentifier.DESCRIPTION).names)

    def test_invalid_unused_bits_raise(self):
        for unused, data in [(8, b"\x00"), (1, b"")]:
            with self.subTest(unused=unused, data=data), self.assertRaises(ValueError):
                BitString(unused, data)

    def test_copy_and_pickle_keep_names(self):
        named = BitString(4, b"\x40", PropertyIdentifier.STATUS_FLAGS)
        for clone in (copy.copy(named), copy.deepcopy(named), pickle.loads(pickle.dumps(named))):
            self.assertEqual(clone, named)
            self.assertEqual(clone.names, ["FAULT"])

    def test_constructed_bit_string_value_reads_unnamed(self):
        value = PropertyValue.bit_string(6, b"\x40").value
        self.assertEqual(value, BitString(6, b"\x40"))
        self.assertIsNone(value.names)
        self.assertIsNone(PropertyValue.enumerated(3).name)


class LogRecordBitStringTests(unittest.TestCase):
    def test_log_status_status_flags_and_bitstring_data(self):
        stamp = "0e a4 7e 0a 05 01 b4 09 00 00 00 0f"
        # A log-status datum with LOG_INTERRUPTED, then Status_Flags FAULT.
        status = bytes.fromhex(stamp + "1e 0a 05 20 1f" + "2a 04 40")
        # A bitstring datum, 10100, and no status flags.
        bits = bytes.fromhex(stamp + "1e 6a 03 a0 1f")
        first, second = decode_log_records({
            "object_id": ObjectIdentifier(ObjectType.TREND_LOG, 1),
            "property_id": PropertyIdentifier.LOG_BUFFER,
            "item_count": 2,
            "item_data": status + bits,
        })
        log_status = first["datum"]["log_status"]
        self.assertEqual((log_status.unused_bits, log_status.data), (5, b"\x20"))
        self.assertEqual(log_status.names, ["LOG_INTERRUPTED"])
        flags = first["status_flags"]
        self.assertEqual((flags.unused_bits, flags.data), (4, b"\x40"))
        self.assertEqual(flags.bits, [False, True, False, False])
        self.assertEqual(flags.names, ["FAULT"])
        datum = second["datum"]["bitstring"]
        self.assertEqual(datum, BitString(3, b"\xa0"))
        self.assertEqual(datum.bits, [True, False, True, False, False])
        self.assertIsNone(datum.names)
        self.assertIsNone(second["status_flags"])


class EventInformationPagingTests(unittest.IsolatedAsyncioTestCase):
    async def test_pages_follow_the_last_object_id(self):
        # Two analog inputs over their high limit, one summary an answer.
        server = BACnetServer(9874, interface="127.0.0.1", port=0,
                              event_information_max_returned_summaries=1)
        server.add_notification_class(0, "NC")
        for instance in (1, 2):
            server.add_analog_input(instance, f"AI-{instance}")
        await server.start()

        async def stop_server():
            await server.stop()

        self.addAsyncCleanup(stop_server)
        for instance in (1, 2):
            oid = ObjectIdentifier(ObjectType.ANALOG_INPUT, instance)
            await server.write_property_local(
                oid, PropertyIdentifier.HIGH_LIMIT, PropertyValue.real(1), source_object=None)
            await server.write_property_local(
                oid, PropertyIdentifier.LIMIT_ENABLE, PropertyValue.bit_string(6, b"\x40"),
                source_object=None)
            await server.set_present_value_local(oid, PropertyValue.real(2))
        address = await server.local_address()
        client = BACnetClient(interface="127.0.0.1", port=0)
        await client.__aenter__()

        async def stop_client():
            await client.__aexit__(None, None, None)

        self.addAsyncCleanup(stop_client)
        seen = []
        after = None
        for expected_more in (True, False):
            page = await asyncio.wait_for(client.get_event_information(address, after), 3)
            self.assertIs(page["more_events"], expected_more)
            self.assertEqual(len(page["summaries"]), 1)
            summary = page["summaries"][0]
            self.assertEqual(summary["event_state"], EventState.HIGH_LIMIT)
            self.assertEqual(summary["acknowledged_transitions"].names,
                             ["TO_OFFNORMAL", "TO_FAULT", "TO_NORMAL"])
            self.assertEqual(summary["event_timestamps"][0].kind, "date_time")
            after = summary["object_id"]
            seen.append(after.instance)
        self.assertEqual(seen, [1, 2])


class EnumNameTests(unittest.TestCase):
    def test_enum_name_resolves_by_property(self):
        self.assertEqual(enum_name(PropertyIdentifier.EVENT_STATE, 3), "HIGH_LIMIT")
        self.assertEqual(enum_name(PropertyIdentifier.RELIABILITY, 0), "NO_FAULT_DETECTED")
        self.assertIsNone(enum_name(PropertyIdentifier.EVENT_STATE, 900))
        self.assertIsNone(enum_name(PropertyIdentifier.PRESENT_VALUE, 1))

    def test_enum_classes_name_their_values(self):
        self.assertEqual(EventState.HIGH_LIMIT.name, "HIGH_LIMIT")
        self.assertEqual(ObjectType.from_raw(0).name, "ANALOG_INPUT")
        self.assertIsNone(EventState.from_raw(900).name)


if __name__ == "__main__":
    unittest.main()
