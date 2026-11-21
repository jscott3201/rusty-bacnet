"""Whole Network Port table properties read through one shared B/IP endpoint."""

import asyncio
import copy
import pickle
import unittest

import rusty_bacnet as rb

from bvll_peer import RawPeer, bvll


PORT = rb.ObjectIdentifier(rb.ObjectType.NETWORK_PORT, 1)
BDT = rb.PropertyIdentifier.from_raw(414)
FDT = rb.PropertyIdentifier.from_raw(415)
BDT4 = bytes.fromhex("0e0e1cc00002020f1abac00f1cffffff00")
FDT4 = bytes.fromhex("0d06c0000203bac0193c2932")
BDT4_ROWS = [dict(address="192.0.2.2:47808", broadcast_mask="255.255.255.0")]
FDT4_ROWS = [dict(address="192.0.2.3:47808", ttl=60, seconds_remaining=50)]


class NetworkPortTableTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.endpoint = rb.BipEndpoint(
            2690, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
            apdu_timeout_ms=2000, apdu_retries=0,
        )
        await self.endpoint.start()
        self.addAsyncCleanup(self.close_endpoint)
        self.client = await self.endpoint.client()
        self.peer = RawPeer(self.reply)
        self.addCleanup(self.peer.close)

    async def close_endpoint(self):
        await self.endpoint.close()

    def reply(self, wire, source):
        if len(wire) < 10 or wire[:2] != b"\x81\x0a":
            return
        invoke, service = wire[8:10]
        if service == 12:
            self.read_fields = wire[10:]
            apdu = bytes((0x30, invoke, service)) + self.read_fields + b"\x3e" + self.octets + b"\x3f"
        elif service == 14:
            # One result: the Network Port's property 414 or 415.
            prop = self.rpm_property.to_raw().to_bytes(2, "big")
            apdu = (bytes((0x30, invoke, service)) + bytes.fromhex("0c0e000001") + b"\x1e"
                    + b"\x2a" + prop + b"\x4e" + self.octets + b"\x4f\x1f")
        elif service == 15:
            self.written_fields = wire[10:]
            apdu = bytes((0x20, invoke, service))
        else:
            return
        return [bvll(0x0a, b"\x01\x00" + apdu)]

    async def read(self, prop, octets, obj=PORT, index=None):
        self.octets = octets
        return await self.client.read_property(self.peer.address, obj, prop, index)

    async def test_both_tables_keep_rows_and_original_write_encoding(self):
        # The second FDT row encodes the same small TTL in two octets.
        fixtures = [
            (BDT, "bdt", BDT4, BDT4_ROWS),
            (FDT, "fdt", FDT4 + bytes.fromhex("0d06c0000203bac11a003c2932"),
             [dict(address=f"192.0.2.3:{port}", ttl=60, seconds_remaining=50)
              for port in (47808, 47809)]),
            (BDT, "bdt", b"", []),
            (FDT, "fdt", b"", []),
        ]
        for prop, tag, octets, rows in fixtures:
            with self.subTest(tag=tag, octets=octets.hex()):
                value = await self.read(prop, octets)
                self.assertEqual((value.tag, value.value, value.octets), (tag, rows, octets))
                self.assertNotEqual(value, rb.PropertyValue.application_data(octets))
                for rebuilt in (copy.copy(value), copy.deepcopy(value), pickle.loads(pickle.dumps(value))):
                    self.assertEqual(rebuilt, value)
                    self.assertEqual(hash(rebuilt), hash(value))
                    self.assertEqual((rebuilt.tag, rebuilt.value, rebuilt.octets), (tag, rows, octets))
                projected = value.value
                if projected:
                    projected[0]["address"] = "changed"
                projected.append({})
                self.assertEqual(value.value, rows)
                await self.client.write_property(self.peer.address, PORT, prop, value)
                self.assertEqual(self.written_fields, self.read_fields + b"\x3e" + octets + b"\x3f")

    async def test_equality_is_by_octets(self):
        one = await self.read(FDT, FDT4)
        two = await self.read(FDT, bytes.fromhex("0d06c0000203bac01a003c2932"))
        self.assertEqual(one.value, two.value)
        self.assertNotEqual(one, two)

    async def test_address_choices_and_row_order(self):
        ip6 = bytes.fromhex("20010db8000000000000000000000002")
        bdt6 = bytes.fromhex("0e0e1d10") + ip6 + bytes.fromhex("0f1abac00f")
        name = bytes.fromhex("0e0e2c006875620f197b0f")
        no_host = bytes.fromhex("0e0e080f19000f")
        bdt = await self.read(BDT, name + bdt6 + no_host)
        self.assertEqual(bdt.value, [
            dict(address="hub:123", broadcast_mask=None),
            dict(address="[2001:db8::2]:47808", broadcast_mask=None),
            dict(address=None, broadcast_mask=None),
        ])
        fdt = await self.read(FDT, bytes.fromhex("0d12") + ip6 + bytes.fromhex("bac119042903"))
        self.assertEqual(fdt.value, [dict(address="[2001:db8::2]:47809", ttl=4, seconds_remaining=3)])

    async def test_semantic_failures_keep_exact_octets(self):
        # Well framed but not the entries: the generic reading of any typed
        # collection, the context-tagged octets kept.
        for prop, octets in (
            (BDT, BDT4[:-5] + bytes.fromhex("1bffff00")),
            (FDT, FDT4[:-4] + bytes.fromhex("2932193c")),
        ):
            with self.subTest(prop=prop, octets=octets.hex()):
                value = await self.read(prop, octets)
                self.assertEqual((value.tag, value.octets), ("tagged", octets))
                self.assertEqual(b"".join(node.octets for node in value.value), octets)
        # Application values that aren't entries keep their octets as
        # received (#2690), so a write-back sends the same octets.
        junk = bytes.fromhex("220002")
        for prop in (BDT, FDT):
            with self.subTest(prop=prop, octets=junk.hex()):
                value = await self.read(prop, junk)
                self.assertEqual((value.tag, value.octets), ("application_data", junk))
                await self.client.write_property(self.peer.address, PORT, prop, value)
                self.assertEqual(self.written_fields,
                                 self.read_fields + b"\x3e" + junk + b"\x3f")
        with self.assertRaisesRegex(rb.BacnetError, "(?i)tag"):
            await self.read(BDT, bytes.fromhex("0e0901"))

    async def test_wrong_object_keeps_generic_shape_and_an_index_one_entry(self):
        analog = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1)
        value = await self.read(BDT, BDT4, analog)
        self.assertEqual((value.tag, value.octets), ("tagged", BDT4))
        value = await self.read(BDT, BDT4, PORT, 1)
        self.assertEqual((value.tag, value.value, value.octets), ("bdt_entry", BDT4_ROWS[0], BDT4))

    async def test_read_property_multiple_gives_the_same_tables(self):
        for prop, octets, tag, rows in ((BDT, BDT4, "bdt", BDT4_ROWS), (FDT, FDT4, "fdt", FDT4_ROWS),
                                        (FDT, b"", "fdt", [])):
            with self.subTest(tag=tag, octets=octets.hex()):
                self.octets, self.rpm_property = octets, prop
                [result] = await self.client.read_property_multiple(
                    self.peer.address, [(PORT, [(prop, None)])])
                [element] = result["results"]
                value = element["value"]
                self.assertEqual((value.tag, value.value, value.octets), (tag, rows, octets))


class LocalBbmdTableTests(unittest.IsolatedAsyncioTestCase):
    """Our own BBMD's Network Port encodes its tables, and the binding reads
    them back as the rows the BVLC reads give."""

    @staticmethod
    async def stop(server):
        await asyncio.wait_for(server.stop(), 10)

    async def test_a_local_read_matches_read_bdt_and_read_fdt(self):
        server = rb.BACnetServer(
            2691, interface="127.0.0.1", port=0, broadcast_address="127.255.255.255",
            registered_network_port=1,
            bbmd=rb.Bbmd(bdt=[("127.0.0.2:47808", "255.255.255.0")], accept_foreign_devices=True))
        server.add_bip_network_port(1, "bbmd", ip_address="127.0.0.1", udp_port=0)
        await asyncio.wait_for(server.start(), 10)
        self.addAsyncCleanup(self.stop, server)
        address = await server.local_address()
        async with rb.BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000) as client:
            await client.register_foreign_device_bvlc(address, 60)
            bdt_rows = [dict(address=row, broadcast_mask=mask)
                        for row, mask in await client.read_bdt(address)]
            fdt_rows = await client.read_fdt(address)
            for prop, tag, rows in ((BDT, "bdt", bdt_rows), (FDT, "fdt", fdt_rows)):
                with self.subTest(tag=tag):
                    local = await server.read_property(PORT, prop)
                    self.assertEqual(local.tag, tag)
                    self.assertTrue(local.value)
                    if tag == "bdt":
                        read = await client.read_property(address, PORT, prop)
                        self.assertEqual(read, local)
                        self.assertEqual(sorted(local.value, key=str), sorted(rows, key=str))
                    else:
                        # The time left counts down between the two reads.
                        self.assertEqual([(row["address"], row["ttl"]) for row in local.value],
                                         [(row["address"], row["ttl"]) for row in rows])



class TypedListRebuildTests(unittest.TestCase):
    """``_typed_list``, which pickling uses to rebuild a ``bdt`` or ``fdt``."""

    def test_rebuilds_a_table_and_shows_it_as_a_list(self):
        entry = rb.PropertyValue._typed_element("bdt_entry", BDT4)
        table = rb.PropertyValue._typed_list("bdt_entry", [entry])
        self.assertEqual((table.tag, table.value, table.octets), ("bdt", BDT4_ROWS, BDT4))
        self.assertEqual(repr(table), "PropertyValue.list(<1 bdt_entry elements>)")
        self.assertEqual(rb.PropertyValue._typed_list("fdt_entry", []).tag, "fdt")

    def test_refuses_unknown_tags_non_tables_and_other_items(self):
        bdt_entry = rb.PropertyValue._typed_element("bdt_entry", BDT4)
        fdt_entry = rb.PropertyValue._typed_element("fdt_entry", FDT4)
        for tag, items in [
            ("no_such_element", []),
            ("bdt", [bdt_entry]),            # the collection's tag, not its element's
            ("destination", []),             # an element with no table tag of its own
            ("bdt_entry", [fdt_entry]),      # another table's entry
            ("bdt_entry", [rb.PropertyValue.application_data(BDT4)]),
            ("bdt_entry", [rb.PropertyValue.unsigned(2)]),
            ("bdt_entry", [bdt_entry, rb.PropertyValue.null()]),
        ]:
            with self.subTest(tag=tag, items=items), self.assertRaises(ValueError):
                rb.PropertyValue._typed_list(tag, items)


if __name__ == "__main__":
    unittest.main()
