"""Whole Network Port table properties read through one shared B/IP endpoint."""

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
            (BDT, "bdt", BDT4, [dict(address_kind="ip", address="192.0.2.2", port=47808,
                                      broadcast_mask="255.255.255.0")]),
            (FDT, "fdt", FDT4 + bytes.fromhex("0d06c0000203bac11a003c2932"),
             [dict(address="192.0.2.3", port=port, ttl=60, seconds_remaining=50)
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
                    projected[0]["port"] = 0
                projected.append({})
                self.assertEqual(value.value, rows)
                await self.client.write_property(self.peer.address, PORT, prop, value)
                self.assertEqual(self.written_fields, self.read_fields + b"\x3e" + octets + b"\x3f")

    async def test_address_choices_and_row_order(self):
        ip6 = bytes.fromhex("20010db8000000000000000000000002")
        bdt6 = bytes.fromhex("0e0e1d10") + ip6 + bytes.fromhex("0f1abac00f")
        name = bytes.fromhex("0e0e2c006875620f197b0f")
        no_host = bytes.fromhex("0e0e080f19000f")
        bdt = await self.read(BDT, name + bdt6 + no_host)
        self.assertEqual(bdt.value, [
            dict(address_kind="name", address="hub", port=123, broadcast_mask=None),
            dict(address_kind="ip", address="2001:db8::2", port=47808, broadcast_mask=None),
            dict(address_kind="none", address=None, port=0, broadcast_mask=None),
        ])
        fdt = await self.read(FDT, bytes.fromhex("0d12") + ip6 + bytes.fromhex("bac119042903"))
        self.assertEqual(fdt.value, [dict(address="2001:db8::2", port=47809, ttl=4, seconds_remaining=3)])

    async def test_semantic_failures_keep_exact_octets(self):
        for prop, octets, tag in (
            (BDT, BDT4[:-5] + bytes.fromhex("1bffff00"), "tagged"),
            (FDT, FDT4[:-4] + bytes.fromhex("2932193c"), "tagged"),
            (FDT, bytes.fromhex("220002"), "application_data"),
        ):
            with self.subTest(prop=prop, octets=octets.hex()):
                value = await self.read(prop, octets)
                self.assertEqual((value.tag, value.octets), (tag, octets))
                if tag == "tagged":
                    self.assertEqual(b"".join(node.octets for node in value.value), octets)
                else:
                    self.assertEqual(value.value, octets)
        with self.assertRaisesRegex(rb.BacnetError, "(?i)tag"):
            await self.read(BDT, bytes.fromhex("0e0901"))

    async def test_wrong_object_or_index_keeps_generic_shape(self):
        analog = rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1)
        for obj, index in ((analog, None), (PORT, 1)):
            with self.subTest(obj=obj, index=index):
                value = await self.read(BDT, BDT4, obj, index)
                self.assertEqual((value.tag, value.octets), ("tagged", BDT4))
