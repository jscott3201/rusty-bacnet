"""Synthetic property octets for declarations in ace-bacnet-devices (591a219b)."""

import copy
import pickle
import unittest
from collections import UserDict

import rusty_bacnet as rb

from bvll_peer import RawPeer, bvll


class ForcedPropertyDecodeTests(unittest.TestCase):
    def decoded(self, wire, primitive, **kwargs):
        raw = bytes.fromhex(wire)
        result = rb.decode_property_as(raw, primitive, **kwargs)
        self.assertEqual((result.status, result.raw, result.diagnostic), ("decoded", raw, None))
        self.assertIsNone(result.value.read_context)
        return result

    def refused(self, raw, primitive, status, **kwargs):
        result = rb.decode_property_as(raw, primitive, **kwargs)
        self.assertEqual((result.status, result.raw, result.value, result.value_names),
                         (status, raw, None, None))
        self.assertTrue(result.diagnostic)

    def test_catalogue_declarations_are_scoped_by_vendor_and_object_type(self):
        # TAC 11, Device property 1330 is Unsigned with enumeration semantics.
        names = UserDict({2: "foreignDevice"})
        tac = self.decoded("220002", "Unsigned", value_names=names)
        self.assertEqual((tac.value.tag, tac.value.value, tac.value_names),
                         ("unsigned", 2, ("foreignDevice",)))
        self.assertIsNone(tac.value.name)
        self.refused(b"\x91\x02", "Unsigned", "type_mismatch", value_names=names)
        self.assertEqual(self.decoded("2102", "Unsigned").value_names, None)
        # Device property 4151 is Array<Unsigned> at Carrier (16) and scalar at OEMCtrl (446).
        carrier = self.decoded("2102", "Unsigned", shape="array")
        oem = self.decoded("2102", "Unsigned")
        self.assertEqual((carrier.value.tag, carrier.value.value, oem.value.value), ("list", [2], 2))
        # Honeywell (17), property 760 on AI, BI and MSI respectively.
        for primitive, wire, value in [("Real", "4440200000", 2.5),
                                       ("Enumerated", "9101", 1), ("Unsigned", "2102", 2)]:
            self.assertEqual(self.decoded(wire, primitive).value.value, value)
        with self.assertRaises(AttributeError):
            tac.status = "malformed"

    def test_primitive_tokens(self):
        for primitive, wire, tag in [
            ("Null", "00", "null"), ("Boolean", "11", "boolean"),
            ("Unsigned", "2102", "unsigned"), ("Integer", "31ff", "signed"),
            ("Real", "4400000000", "real"), ("Double", "55080000000000000000", "double"),
            ("OctetString", "6102", "octet_string"), ("CharacterString", "720041", "character_string"),
            ("BitString", "820780", "bit_string"), ("Enumerated", "9102", "enumerated"),
            ("Date", "a47b010102", "date"), ("Time", "b400000000", "time"),
            ("ObjectIdentifier", "c402000001", "object_identifier"),
        ]:
            with self.subTest(primitive=primitive):
                self.assertEqual(self.decoded(wire, primitive).value.tag, tag)
        signed = self.decoded("31ff", "Integer", value_names={-1: "minusOne"})
        self.assertEqual((signed.value.value, signed.value_names), (-1, ("minusOne",)))

    def test_index_count_fixed_and_variable_collections(self):
        for shape in ("list", "array"):
            for wire, expected in [("", []), ("2102", [2]), ("21022103", [2, 3])]:
                result = self.decoded(wire, "Unsigned", shape=shape, value_names={2: "two"})
                self.assertEqual((result.value.tag, result.value.value), ("list", expected))
                self.assertEqual(result.value_names, tuple("two" if v == 2 else None for v in expected))
        count = self.decoded("2508ffffffffffffffff", "Real", shape="array", array_index=0,
                             value_names={})
        self.assertEqual((count.value.value, count.value_names), (2**64 - 1, ()))
        self.assertEqual(self.decoded("229c40", "Real", shape="array", array_length=40000,
                                     array_index=0).value.value, 40000)
        self.assertEqual(self.decoded("2102", "Unsigned", shape="array", array_length=40000,
                                     array_index=40000).value.value, 2)
        self.assertEqual(self.decoded("", "Unsigned", shape="array", array_length=0).value.value, [])
        for wire, options in [("2102", {"array_length": 2}), ("2102", {"array_length": 0, "array_index": 1}),
                              ("2102", {"array_length": 1, "array_index": 0}),
                              ("3102", {"array_index": 0}), ("21022103", {"array_index": 1})]:
            self.refused(bytes.fromhex(wire), "Unsigned", "type_mismatch", shape="array", **options)

    def test_declaration_errors(self):
        for primitive, options in [("Signed", {}), ("unsigned", {}), ("Unsigned", {"shape": "tuple"}),
                                   ("Unsigned", {"array_index": 1}), ("Unsigned", {"shape": "list", "array_index": 0}),
                                   ("Unsigned", {"array_length": 1}),
                                   ("Unsigned", {"shape": "array", "array_length": 2**100}),
                                   ("Unsigned", {"shape": "array", "array_length": -1}),
                                   ("Unsigned", {"shape": "array", "array_index": -1})]:
            with self.subTest(options=options), self.assertRaises(ValueError):
                rb.decode_property_as(b"", primitive, **options)

    def test_complete_framing_payload_and_limits(self):
        for wire, primitive, status in [
            ("", "Unsigned", "type_mismatch"), ("21029102", "Unsigned", "type_mismatch"),
            ("0902", "Unsigned", "unsupported_encoding"), ("d0", "Unsigned", "unsupported_encoding"),
            ("0e21020f", "Unsigned", "unsupported_encoding"),
            ("95050100000000", "Enumerated", "unsupported_encoding"),
            ("73010000", "CharacterString", "unsupported_encoding"),
            ("7102", "CharacterString", "unsupported_encoding"),
            ("7101", "CharacterString", "malformed"), ("720300", "CharacterString", "malformed"),
            ("910222", "Unsigned", "malformed"), ("09020e", "Unsigned", "malformed"),
            ("d04400", "Unsigned", "malformed"), ("4100", "Real", "malformed"),
        ]:
            with self.subTest(wire=wire, primitive=primitive):
                self.refused(bytes.fromhex(wire), primitive, status)
        self.refused(b"\x0e" * 33 + b"\x0f" * 33, "Null", "limit_exceeded")
        self.refused(b"\x00" * 10001, "Null", "limit_exceeded", shape="array")
        self.assertEqual(len(rb.decode_property_as(b"\x00" * 10000, "Null", shape="list").value.value), 10000)


class ReturnedPropertyContextTests(unittest.IsolatedAsyncioTestCase):
    # Synthetic Device 7, property 1330, array index 1, Unsigned 2 with a leading zero.
    oid = rb.ObjectIdentifier(rb.ObjectType.DEVICE, 7)
    prop = rb.PropertyIdentifier.from_raw(1330)
    missing = rb.PropertyIdentifier.from_raw(1331)
    raw = bytes.fromhex("220002")

    async def asyncSetUp(self):
        self.endpoint = rb.BipEndpoint(2618, interface="127.0.0.1", port=0,
                                       broadcast_address="127.0.0.1", apdu_retries=0)
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
        if service == 12:  # Replace the wildcard Device instance with the actual object.
            fields = bytes.fromhex("0c020000071a053229013e") + self.raw + b"\x3f"
        elif service == 14:  # A value row and a property/unknown-property error row.
            fields = bytes.fromhex("0c020000071e2a053239014e") + self.raw + bytes.fromhex("4f2a05335e910291205f1f")
        elif service == 15:
            self.written = wire[10:]
            return [bvll(0x0a, bytes((1, 0, 0x20, invoke, service)))]
        else:
            return
        return [bvll(0x0a, bytes((1, 0, 0x30, invoke, service)) + fields)]

    async def test_read_and_rpm_own_context_and_ordinary_write(self):
        wildcard = rb.ObjectIdentifier(rb.ObjectType.DEVICE, 4194303)
        rp = await self.client.read_property(self.peer.address, wildcard, self.prop, 1)
        rows = (await self.client.read_property_multiple(
            self.peer.address, [(self.oid, [(self.prop, 1), (self.missing, None)])]))[0]["results"]
        self.assertIsNone(rows[1]["value"])
        self.assertEqual(rows[1]["error"], (rb.ErrorClass.PROPERTY, rb.ErrorCode.UNKNOWN_PROPERTY))
        ordinary = rb.PropertyValue.unsigned(2)
        self.assertIsNone(ordinary.read_context)
        for value in [rp, rows[0]["value"]]:
            for rebuilt in [value, copy.copy(value), copy.deepcopy(value), pickle.loads(pickle.dumps(value))]:
                ctx = rebuilt.read_context
                self.assertEqual((ctx.object_id, ctx.property_id, ctx.array_index, ctx.raw),
                                 (self.oid, 1330, 1, self.raw))
                self.assertEqual((rebuilt, hash(rebuilt), repr(rebuilt), rebuilt.octets),
                                 (ordinary, hash(ordinary), repr(ordinary), b"\x21\x02"))
                with self.assertRaises(AttributeError):
                    ctx.raw = b""
            await self.client.write_property(self.peer.address, self.oid, self.prop, value, array_index=1)
            self.assertEqual(self.written, bytes.fromhex("0c020000071a053229013e21023f"))
