"""decode_property_as and read_context (#2618), on synthetic octets.

The vendor cases are made up: "vendor A" and "vendor B" stand for two
stations that give one proprietary property number different datatypes.
"""

import copy
import pickle
import unittest
from collections import UserDict

import rusty_bacnet as rb

from bvll_peer import RawPeer, bvll


class ForcedPropertyDecodeTests(unittest.TestCase):
    def decoded(self, wire, datatype, *shape, **kwargs):
        raw = bytes.fromhex(wire)
        result = rb.decode_property_as(raw, datatype, *shape, **kwargs)
        self.assertEqual((result.status, result.raw, result.diagnostic), ("decoded", raw, None))
        self.assertIsNone(result.value.read_context)
        return result

    def refused(self, raw, datatype, status, *shape, **kwargs):
        result = rb.decode_property_as(raw, datatype, *shape, **kwargs)
        self.assertEqual((result.status, result.raw, result.value, result.value_names),
                         (status, raw, None, None))
        self.assertTrue(result.diagnostic)

    def test_declarations_are_scoped_by_vendor_and_object_type(self):
        # Vendor A declares Device property 1330 as unsigned with named values.
        names = UserDict({2: "second"})
        vendor_a = self.decoded("220002", "unsigned", value_names=names)
        self.assertEqual((vendor_a.value.tag, vendor_a.value.value, vendor_a.value_names),
                         ("unsigned", 2, ("second",)))
        self.assertIsNone(vendor_a.value.name)
        self.refused(b"\x91\x02", "unsigned", "type_mismatch", value_names=names)
        self.assertEqual(self.decoded("2102", "unsigned").value_names, None)
        # One property number: an array of unsigned at vendor A, a scalar at vendor B.
        array = self.decoded("2102", "unsigned", "array")
        scalar = self.decoded("2102", "unsigned")
        self.assertEqual((array.value.tag, array.value.value, scalar.value.value), ("list", [2], 2))
        # One vendor's property number with another datatype per object type.
        for datatype, wire, value in [("real", "4440200000", 2.5),
                                      ("enumerated", "9101", 1), ("unsigned", "2102", 2)]:
            self.assertEqual(self.decoded(wire, datatype).value.value, value)
        with self.assertRaises(AttributeError):
            vendor_a.status = "malformed"

    def test_datatype_tokens_are_property_value_tags(self):
        for wire, tag in [
            ("00", "null"), ("11", "boolean"), ("2102", "unsigned"), ("31ff", "signed"),
            ("4400000000", "real"), ("55080000000000000000", "double"), ("6102", "octet_string"),
            ("720041", "character_string"), ("820780", "bit_string"), ("9102", "enumerated"),
            ("a47b010102", "date"), ("b400000000", "time"), ("c402000001", "object_identifier"),
        ]:
            with self.subTest(tag=tag):
                value = self.decoded(wire, tag).value
                self.assertEqual((value.tag, value.octets), (tag, bytes.fromhex(wire)))
        signed = self.decoded("31ff", "signed", value_names={-1: "minusOne"})
        self.assertEqual((signed.value.value, signed.value_names), (-1, ("minusOne",)))

    def test_index_count_fixed_and_variable_collections(self):
        for shape in ("list", "array"):
            for wire, expected in [("", []), ("2102", [2]), ("21022103", [2, 3])]:
                result = self.decoded(wire, "unsigned", shape=shape, value_names={2: "two"})
                self.assertEqual((result.value.tag, result.value.value), ("list", expected))
                self.assertEqual(result.value_names, tuple("two" if v == 2 else None for v in expected))
        count = self.decoded("2508ffffffffffffffff", "real", shape="array", array_index=0,
                             value_names={})
        self.assertEqual((count.value.value, count.value_names), (2**64 - 1, ()))
        self.assertEqual(self.decoded("229c40", "real", shape="array", array_length=40000,
                                     array_index=0).value.value, 40000)
        self.assertEqual(self.decoded("2102", "unsigned", shape="array", array_length=40000,
                                     array_index=40000).value.value, 2)
        self.assertEqual(self.decoded("", "unsigned", shape="array", array_length=0).value.value, [])
        for wire, options in [("2102", {"array_length": 2}), ("2102", {"array_length": 0, "array_index": 1}),
                              ("2102", {"array_length": 1, "array_index": 0}),
                              ("3102", {"array_index": 0}), ("21022103", {"array_index": 1})]:
            self.refused(bytes.fromhex(wire), "unsigned", "type_mismatch", shape="array", **options)

    def test_results_are_values(self):
        first = rb.decode_property_as(b"\x21\x02", "unsigned", value_names={2: "two"})
        again = rb.decode_property_as(b"\x21\x02", "unsigned", value_names={2: "two"})
        refused = rb.decode_property_as(b"\x91\x02", "unsigned")
        self.assertEqual(first, again)
        self.assertEqual(hash(first), hash(again))
        self.assertNotEqual(first, refused)
        self.assertNotEqual(first, rb.decode_property_as(b"\x21\x02", "unsigned"))
        self.assertNotEqual(first, "decoded")
        for result in (first, refused):
            for rebuilt in (copy.copy(result), copy.deepcopy(result),
                            pickle.loads(pickle.dumps(result))):
                self.assertEqual(rebuilt, result)
        self.assertEqual(repr(first),
                         "PropertyDecodeResult(status='decoded', value=PropertyValue.unsigned(2), raw=<2 bytes>)")
        self.assertTrue(repr(refused).startswith(
            "PropertyDecodeResult(status='type_mismatch', diagnostic='expected application tag 2"))
        with self.assertRaises(ValueError):
            rb.PropertyDecodeResult._restore("maybe", None, b"", None, None)

    def test_declaration_errors(self):
        for primitive, options in [("Signed", {}), ("Unsigned", {}), ("Integer", {}), ("integer", {}),
                                   ("list", {}), ("application_data", {}), ("tagged", {}),
                                   ("unsigned", {"shape": "tuple"}),
                                   ("unsigned", {"array_index": 1}), ("unsigned", {"shape": "list", "array_index": 0}),
                                   ("unsigned", {"array_length": 1}),
                                   ("unsigned", {"shape": "array", "array_length": 2**100}),
                                   ("unsigned", {"shape": "array", "array_length": -1}),
                                   ("unsigned", {"shape": "array", "array_length": 65536}),
                                   ("unsigned", {"shape": "array", "array_index": -1})]:
            with self.subTest(options=options), self.assertRaises(ValueError):
                rb.decode_property_as(b"", primitive, **options)
        with self.assertRaisesRegex(ValueError, "0 to 65535"):
            rb.decode_property_as(b"", "unsigned", "array", array_length=65536)
        self.assertEqual(self.decoded("2102", "unsigned", "array", array_length=65535,
                                      array_index=65535).value.value, 2)

    def test_complete_framing_payload_and_limits(self):
        for wire, datatype, status, *shape in [
            # The octets' tag, class or count differs from the declaration.
            ("", "unsigned", "type_mismatch"), ("21029102", "unsigned", "type_mismatch"),
            ("0902", "unsigned", "type_mismatch"), ("0e21020f", "unsigned", "type_mismatch"),
            ("0e21020f", "unsigned", "type_mismatch", "list"),
            # A 5-octet Signed under another declaration is a mismatch first.
            ("35050100000000", "unsigned", "type_mismatch"),
            # Encodings the model can't hold (Clauses 20.2.4, 20.2.5,
            # 20.2.9 and 20.2.11 set no upper integer length).
            ("35050100000000", "signed", "unsupported_encoding"),
            ("2509010000000000000000", "unsigned", "unsupported_encoding"),
            ("95050100000000", "enumerated", "unsupported_encoding"),
            ("73010000", "character_string", "unsupported_encoding"),
            ("7102", "character_string", "unsupported_encoding"),
            ("75050300000041", "character_string", "unsupported_encoding", "list"),
            # Anything else that isn't well formed, reserved tags 13 to 15 included.
            ("d0", "unsigned", "malformed"), ("e100", "unsigned", "malformed"),
            # The long length form for a 1-octet length is broken framing.
            ("350100000000", "signed", "malformed"), ("73030000", "character_string", "malformed"),
            ("d0", "unsigned", "malformed", "array"), ("0902d0", "unsigned", "malformed"),
            ("30", "signed", "malformed"), ("20", "unsigned", "malformed"),
            ("7101", "character_string", "malformed"), ("720300", "character_string", "malformed"),
            ("7106", "character_string", "malformed"),
            # Wrong lengths for fixed-size primitives (Clause 20.2).
            ("12", "boolean", "malformed"), ("0100", "null", "malformed"),
            ("a3010203", "date", "malformed"), ("b3000000", "time", "malformed"),
            ("c3000000", "object_identifier", "malformed"),
            # An empty bit string has no unused bits (Clause 20.2.10).
            ("8105", "bit_string", "malformed"), ("82050f8105", "bit_string", "malformed", "list"),
            ("910222", "unsigned", "malformed"), ("09020e", "unsigned", "malformed"),
            ("d04400", "unsigned", "malformed"), ("4100", "real", "malformed"),
        ]:
            with self.subTest(wire=wire, datatype=datatype):
                self.refused(bytes.fromhex(wire), datatype, status, *shape)
        # An over-wide integer under a fixed length that the count misses:
        # the shape is checked first.
        wide = bytes.fromhex("2509010000000000000000")
        self.refused(wide, "unsigned", "type_mismatch", "array", array_length=2)
        self.refused(wide + wide, "unsigned", "type_mismatch", "array", array_length=3)
        self.refused(wide + wide, "unsigned", "unsupported_encoding", "array", array_length=2)
        self.assertEqual(self.decoded("8100", "bit_string").value.octets, b"\x81\x00")
        self.refused(b"\x0e" * 33 + b"\x0f" * 33, "null", "limit_exceeded")
        self.refused(b"\x00" * 10001, "null", "limit_exceeded", shape="array")
        self.assertEqual(len(rb.decode_property_as(b"\x00" * 10000, "null", "list").value.value), 10000)


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
        self.assertEqual(rp.read_context, rp.read_context)
        self.assertNotEqual(rb.PropertyReadContext._restore(self.oid, 1330, 2), rp.read_context)
        self.assertNotEqual(rp.read_context, (self.oid, 1330, 1))
        self.assertEqual(repr(rp.read_context),
                         f"PropertyReadContext(object_id={self.oid!r}, "
                         "property_id=1330, array_index=1)")
        self.assertEqual(len({rp.read_context, rows[0]["value"].read_context}), 1)
        for value in [rp, rows[0]["value"]]:
            for rebuilt in [value, copy.copy(value), copy.deepcopy(value), pickle.loads(pickle.dumps(value))]:
                ctx = rebuilt.read_context
                self.assertEqual((ctx.object_id, ctx.property_id, ctx.array_index),
                                 (self.oid, 1330, 1))
                self.assertEqual(ctx, value.read_context)
                self.assertEqual(hash(ctx), hash(value.read_context))
                self.assertEqual(pickle.loads(pickle.dumps(ctx)), ctx)
                self.assertEqual(copy.deepcopy(ctx), ctx)
                self.assertFalse(hasattr(ctx, "raw"))
                # A primitive's octets are its shortest encoding, not the
                # leading-zero form received; both decode to the same value.
                self.assertEqual((rebuilt, hash(rebuilt), repr(rebuilt), rebuilt.octets),
                                 (ordinary, hash(ordinary), repr(ordinary), b"\x21\x02"))
                decoded = rb.decode_property_as(rebuilt.octets, "unsigned", "array",
                                                array_index=ctx.array_index)
                self.assertEqual((decoded.status, decoded.value.value), ("decoded", 2))
                with self.assertRaises(AttributeError):
                    ctx.property_id = 1
            await self.client.write_property(self.peer.address, self.oid, self.prop, value, array_index=1)
            self.assertEqual(self.written, bytes.fromhex("0c020000071a053229013e21023f"))

    async def test_values_the_model_cannot_hold_keep_the_octets_received(self):
        # A 5-octet Signed and a UCS-4 string read as application_data, and
        # context-tagged content reads as tagged: .octets are what was sent.
        for raw, datatype, status in [("35050100000000", "signed", "unsupported_encoding"),
                                      ("75050300000041", "character_string", "unsupported_encoding"),
                                      ("0902", "unsigned", "type_mismatch")]:
            with self.subTest(raw=raw):
                self.raw = bytes.fromhex(raw)
                value = await self.client.read_property(self.peer.address, self.oid, self.prop, 1)
                self.assertIn(value.tag, ("application_data", "tagged"))
                self.assertEqual((value.octets, value.read_context.object_id), (self.raw, self.oid))
                result = rb.decode_property_as(value.octets, datatype, "array", array_index=1)
                self.assertEqual((result.status, result.raw), (status, self.raw))
