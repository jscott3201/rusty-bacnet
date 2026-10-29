"""Installed-wheel credential configuration and pure rights-stage checks (#1570)."""

from __future__ import annotations

import ast
import asyncio
import inspect
import unittest
from pathlib import Path

import rusty_bacnet
from rusty_bacnet import (
    BACnetServer, BACnetTimeStamp, BacnetProtocolError, ErrorClass, ErrorCode,
    ObjectIdentifier, ObjectType, PropertyIdentifier, PropertyValue,
)

P = PropertyIdentifier
POINT = ObjectIdentifier(ObjectType.ACCESS_POINT, 1)
ZONE = ObjectIdentifier(ObjectType.ACCESS_ZONE, 1)
RIGHTS = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 1)
BADGE = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, 1)
DEVICE = ObjectIdentifier(ObjectType.DEVICE, 1_570_001)
REMOTE = ObjectIdentifier(ObjectType.DEVICE, 99)
WILDCARD = ObjectIdentifier(ObjectType.DEVICE, 4_194_303)
MISSING = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 77)
KEYWORDS = ["assigned_access_rights", "authorization_exemptions"]


def make_server():
    return BACnetServer(device_instance=1_570_001, device_name="Rights stage",
                       interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")


def assigned(reference=RIGHTS, enable=True):
    return {"assigned_access_rights": reference, "enable": enable}


def position(rights=RIGHTS, kind="positive", index=1):
    return {"access_rights": rights, "kind": kind, "index": index}


def outcome(decision, event=None, rule=None, unresolved=None):
    return {"decision": decision, "access_event": event, "rule": rule,
            "unresolved": [] if unresolved is None else unresolved}


def installed_stub():
    path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
    return ast.parse(path.read_text(encoding="utf-8"), filename=str(path))


class CredentialRightsContractTests(unittest.TestCase):
    def test_runtime_and_installed_stub_signatures_and_result_shapes(self):
        tree = installed_stub()
        server = next(n for n in tree.body if isinstance(n, ast.ClassDef)
                      and n.name == "BACnetServer")
        for name, parameters, keywords, result in [
            ("add_access_credential", ["self", "instance", "name"], KEYWORDS, "None"),
            ("evaluate_access_rights", ["self", "credential", "point"], [],
             "Awaitable[AccessRightsEvaluation]"),
        ]:
            runtime = inspect.signature(getattr(BACnetServer, name)).parameters
            self.assertEqual(list(runtime), parameters + keywords)
            method = next(n for n in server.body if isinstance(n, ast.FunctionDef)
                          and n.name == name)
            self.assertEqual([a.arg for a in method.args.args], parameters)
            self.assertEqual([a.arg for a in method.args.kwonlyargs], keywords)
            self.assertEqual(ast.unparse(method.returns), result)
            for key, default in zip(keywords, method.args.kw_defaults):
                self.assertIs(runtime[key].kind, inspect.Parameter.KEYWORD_ONLY)
                self.assertIsNone(runtime[key].default)
                self.assertIsInstance(default, ast.Constant)
                self.assertIsNone(default.value)
            self.assertIn("rights", ast.get_docstring(method).lower())
        shapes = {
            "AssignedAccessRights": {"assigned_access_rights", "enable"},
            "AccessRulePosition": {"access_rights", "kind", "index"},
            "UnresolvedAccessRights": {"index", "reference", "reason"},
            "AccessRightsEvaluation": {"decision", "access_event", "rule", "unresolved"},
        }
        for name, fields in shapes.items():
            cls = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == name)
            self.assertEqual([ast.unparse(b) for b in cls.bases], ["TypedDict"])
            self.assertEqual({n.target.id for n in cls.body if isinstance(n, ast.AnnAssign)}, fields)
        # These annotations describe plain mappings; they aren't runtime classes.
        for name in shapes:
            self.assertFalse(hasattr(rusty_bacnet, name))
        with self.assertRaises(TypeError):
            make_server().add_access_credential(1, "No positional options", [], [])


class CredentialRightsTests(unittest.IsolatedAsyncioTestCase):
    def assert_protocol(self, error, error_class, code):
        self.assertEqual(error.error_class, error_class.to_raw())
        self.assertEqual(error.error_code, code.to_raw())

    async def test_optional_exemptions_absent_empty_and_native_boundaries(self):
        server = make_server()
        server.add_access_point(1, "Lobby")
        configs = [{}, dict.fromkeys(KEYWORDS),
                   {"assigned_access_rights": [], "authorization_exemptions": []},
                   {"authorization_exemptions": [0, 6, 64, 255, 64]}]
        for index, kwargs in enumerate(configs, 1):
            server.add_access_credential(index, f"Badge {index}", **kwargs)
        await server.start()
        try:
            for index in range(1, 5):
                oid = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, index)
                self.assertEqual((await server.read_property(oid, P.ASSIGNED_ACCESS_RIGHTS, 0)).value, 0)
                props = (await server.read_property(oid, P.PROPERTY_LIST)).value
                self.assertEqual(P.AUTHORIZATION_EXEMPTIONS.to_raw() in props,
                                 index >= 3)
                if index < 3:
                    with self.assertRaises(BacnetProtocolError) as raised:
                        await server.read_property(oid, P.AUTHORIZATION_EXEMPTIONS)
                    self.assert_protocol(raised.exception, ErrorClass.PROPERTY, ErrorCode.UNKNOWN_PROPERTY)
                else:
                    values = (await server.read_property(oid, P.AUTHORIZATION_EXEMPTIONS)).value
                    self.assertEqual(values, [] if index == 3 else [0, 6, 64, 255, 64])
                self.assertEqual(await server.evaluate_access_rights(oid, POINT), outcome("denied", 135))
        finally:
            await server.stop()

    async def test_bad_configuration_registers_nothing_and_errors_keep_their_types(self):
        cases = [
            ({"assigned_access_rights": 3}, TypeError),
            ({"assigned_access_rights": [assigned(), None]}, TypeError),
            ({"assigned_access_rights": [assigned(), assigned(enable=1)]}, TypeError),
            ({"assigned_access_rights": [assigned(), {"enable": True}]}, ValueError),
            ({"assigned_access_rights": [assigned(), {"assigned_access_rights": RIGHTS}]}, ValueError),
            ({"assigned_access_rights": [assigned(), dict(assigned(), extra=True)]}, ValueError),
            ({"assigned_access_rights": [assigned(), assigned("bad")]}, TypeError),
            ({"assigned_access_rights": [assigned(), assigned((POINT, RIGHTS))]}, ValueError),
            ({"assigned_access_rights": [assigned(), assigned(POINT)]}, BacnetProtocolError),
            ({"authorization_exemptions": 2}, TypeError),
        ]
        for bad in [True, False, 2.0, "2", None]:
            cases.append(({"authorization_exemptions": [0, bad]}, TypeError))
        for bad in [-1, 2**32, 2**200]:
            cases.append(({"authorization_exemptions": [0, bad]}, OverflowError))
        for bad in [7, 63, 256, 2**32 - 1]:
            cases.append(({"authorization_exemptions": [0, bad]}, BacnetProtocolError))
        def reject(server, index, kwargs, error):
            with self.assertRaises(error) as raised:
                server.add_access_credential(index, f"Invalid {index}", **kwargs)
            if error is BacnetProtocolError:
                self.assert_protocol(raised.exception, ErrorClass.PROPERTY, ErrorCode.VALUE_OUT_OF_RANGE)

        rejected = make_server()
        rejected.add_access_point(1, "Lobby")
        rejected.add_access_rights(1, "Allow", positive_access_rules=[{"enable": True}])
        for index, (kwargs, error) in enumerate(cases, 1):
            with self.subTest(phase="reject", kwargs=kwargs):
                reject(rejected, index, kwargs, error)
        # No valid registration may replace and hide a partially queued credential.
        await rejected.start()
        try:
            for index in range(1, len(cases) + 1):
                oid = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, index)
                with self.subTest(phase="absent", credential=index):
                    with self.assertRaises(BacnetProtocolError) as read_error:
                        await rejected.read_property(oid, P.OBJECT_NAME)
                    self.assert_protocol(read_error.exception, ErrorClass.OBJECT, ErrorCode.UNKNOWN_OBJECT)
                    with self.assertRaises(BacnetProtocolError) as evaluation_error:
                        await rejected.evaluate_access_rights(oid, POINT)
                    self.assert_protocol(evaluation_error.exception, ErrorClass.OBJECT, ErrorCode.UNKNOWN_OBJECT)
        finally:
            await rejected.stop()

        # Recovery is a separate fixture, after absence has been checked directly.
        recovery = make_server()
        recovery.add_access_point(1, "Lobby")
        recovery.add_access_rights(1, "Allow", positive_access_rules=[{"enable": True}])
        for index, (kwargs, error) in enumerate(cases, 1):
            with self.subTest(phase="recover", kwargs=kwargs):
                reject(recovery, index, kwargs, error)
                recovery.add_access_credential(index, f"Valid {index}", assigned_access_rights=[assigned()])
        await recovery.start()
        try:
            for index in range(1, len(cases) + 1):
                oid = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, index)
                self.assertEqual((await recovery.read_property(oid, P.OBJECT_NAME)).value, f"Valid {index}")
                self.assertEqual(await recovery.evaluate_access_rights(oid, POINT),
                                 outcome("granted", rule=position()))
        finally:
            await recovery.stop()

    async def test_decisions_provenance_and_repeated_evaluations_are_pure(self):
        server = make_server()
        server.add_access_point(1, "Lobby")
        server.add_access_zone(1, "Zone", entry_points=[POINT])
        server.add_binary_value(1, "Closed time window")
        timer = ObjectIdentifier(ObjectType.BINARY_VALUE, 1)
        server.add_access_rights(1, "Positive", positive_access_rules=[
            {"enable": False}, {"enable": True, "location": POINT}])
        server.add_access_rights(2, "Point denial", negative_access_rules=[
            {"enable": False}, {"enable": True, "location": POINT}])
        server.add_access_rights(3, "Zone denial", negative_access_rules=[{"enable": True, "location": ZONE}])
        server.add_access_rights(4, "Out of time", positive_access_rules=[
            {"enable": False}, {"enable": True, "location": POINT,
             "time_range": {"object_identifier": timer, "property_identifier": P.PRESENT_VALUE}}])
        second = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 2)
        third = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 3)
        fourth = ObjectIdentifier(ObjectType.ACCESS_RIGHTS, 4)
        configs = [([assigned()], None), ([assigned(), assigned(second)], None),
                   ([assigned(), assigned(third)], None), ([], None),
                   ([assigned(fourth)], None), ([assigned((REMOTE, MISSING))], [2])]
        expected = [outcome("granted", rule=position(index=2)),
                    outcome("denied", 134, position(second, "negative", 2)),
                    outcome("denied", 133, position(third, "negative", 1)),
                    outcome("denied", 135), outcome("denied", 136, position(fourth, index=2)),
                    outcome("exempt")]
        for index, (rights, exemptions) in enumerate(configs, 1):
            server.add_access_credential(index, f"Badge {index}", assigned_access_rights=rights,
                                         authorization_exemptions=exemptions)
        await server.start()
        try:
            # Seed distinguishable evidence: even denials must leave this event untouched.
            await server.report_access_event_local(POINT, 128, 1570,
                time=BACnetTimeStamp.sequence_number(42), credential=BADGE,
                authentication_factor=(8, 0, b"\x12\x34\x56"))
            point_rows = [P.ACCESS_EVENT, P.ACCESS_EVENT_TAG, P.ACCESS_EVENT_TIME,
                          P.ACCESS_EVENT_CREDENTIAL, P.ACCESS_EVENT_AUTHENTICATION_FACTOR,
                          P.AUTHENTICATION_STATUS, P.EVENT_STATE]
            credential_rows = [P.CREDENTIAL_STATUS, P.REASON_FOR_DISABLE, P.CREDENTIAL_DISABLE,
                               P.ASSIGNED_ACCESS_RIGHTS, P.AUTHORIZATION_EXEMPTIONS,
                               P.LAST_ACCESS_EVENT, P.LAST_ACCESS_POINT, P.LAST_USE_TIME, P.USES_REMAINING,
                               P.DAYS_REMAINING]
            async def snapshot():
                result = []
                for oid, props in [(POINT, point_rows)] + [
                    (ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, i), credential_rows)
                    for i in range(1, 7)
                ]:
                    for prop in props:
                        try:
                            result.append(await server.read_property(oid, prop))
                        except BacnetProtocolError as error:
                            self.assert_protocol(error, ErrorClass.PROPERTY, ErrorCode.UNKNOWN_PROPERTY)
                            result.append((error.error_class, error.error_code))
                return result
            before = await snapshot()
            for repeat in range(3):
                for index, value in enumerate(expected, 1):
                    with self.subTest(repeat=repeat, credential=index):
                        oid = ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, index)
                        self.assertEqual(await server.evaluate_access_rights(oid, POINT), value)
                self.assertEqual(await snapshot(), before)
        finally:
            await server.stop()

    async def test_grant_keeps_unresolved_order_duplicates_and_sparse_indices(self):
        server = make_server()
        server.add_access_point(1, "Lobby")
        server.add_access_rights(1, "Allow", positive_access_rules=[{"enable": True}])
        unused = ObjectIdentifier(ObjectType.ANALOG_VALUE, 4_194_303)
        references = [assigned(MISSING, False), assigned((REMOTE, RIGHTS)), assigned(unused),
                      assigned((DEVICE, MISSING)), assigned((WILDCARD, RIGHTS)),
                      assigned((DEVICE, RIGHTS)), assigned(MISSING), assigned((REMOTE, RIGHTS))]
        server.add_access_credential(1, "Mixed", assigned_access_rights=references)
        await server.start()
        try:
            self.assertEqual((await server.read_property(BADGE, P.ASSIGNED_ACCESS_RIGHTS, 0)).value, 8)
            expected = outcome("granted", rule=position(), unresolved=[
                {"index": 2, "reference": (REMOTE, RIGHTS), "reason": "remote"},
                {"index": 4, "reference": (DEVICE, MISSING), "reason": "missing"},
                {"index": 5, "reference": (WILDCARD, RIGHTS), "reason": "wildcard_device"},
                {"index": 7, "reference": MISSING, "reason": "missing"},
                {"index": 8, "reference": (REMOTE, RIGHTS), "reason": "remote"},
            ])
            self.assertEqual(await server.evaluate_access_rights(BADGE, POINT), expected)
        finally:
            await server.stop()

    async def test_lifecycle_and_unknown_or_wrong_object_identifiers(self):
        server = make_server()
        server.add_access_point(1, "Lobby")
        server.add_access_credential(1, "Empty")
        with self.assertRaisesRegex(RuntimeError, "server not started"):
            await server.evaluate_access_rights(BADGE, POINT)
        await server.start()
        try:
            with self.assertRaises(RuntimeError):
                server.add_access_credential(2, "Too late", assigned_access_rights=[])
            for credential, point in [
                (ObjectIdentifier(ObjectType.ACCESS_CREDENTIAL, 99), POINT),
                (BADGE, ObjectIdentifier(ObjectType.ACCESS_POINT, 99)),
                (POINT, POINT), (BADGE, BADGE),
            ]:
                with self.assertRaises(BacnetProtocolError) as raised:
                    await server.evaluate_access_rights(credential, point)
                self.assert_protocol(raised.exception, ErrorClass.OBJECT, ErrorCode.UNKNOWN_OBJECT)
        finally:
            await server.stop()
        with self.assertRaisesRegex(RuntimeError, "server not started"):
            await server.evaluate_access_rights(BADGE, POINT)


if __name__ == "__main__":
    unittest.main()
