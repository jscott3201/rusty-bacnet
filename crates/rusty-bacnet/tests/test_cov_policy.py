"""Installed-wheel COV policy: BACnetServer(cov_policy=...) (#1100)."""
import ast
import asyncio
import importlib.util
import inspect
from pathlib import Path
import socket
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, BacnetProtocolError, ErrorClass, ErrorCode,
    ObjectIdentifier, ObjectType, PropertyValue,
)
from port_retry import rerun_on_lost_port

KEYS = {
    "max_subscriptions_global": "int",
    "max_subscriptions_per_peer": "int",
    "reserved_capacity": "int",
    "reserved_peers": "list[bytes]",
    "reserved_recipients": "list[tuple[int | None, bytes]]",
    "allow_indefinite_subscriptions": "bool",
    "max_indefinite_per_peer": "int",
    "max_notifications_per_event": "int",
    "max_notification_bytes_per_event": "int",
    "max_confirmed_in_flight_per_peer": "int",
}

TARGET = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)


def local_server(**policy):
    server = BACnetServer(503_807, interface="127.0.0.1", port=0,
                          broadcast_address="127.0.0.1", **policy)
    server.add_analog_input(1, "COV policy", present_value=10.0)
    return server


def local_client(port=0):
    return BACnetClient(interface="127.0.0.1", port=port,
                        broadcast_address="127.0.0.1", apdu_timeout_ms=2000)


def free_udp_port():
    """A loopback port to give a client, so its B/IP MAC is known up front.
    Another process can take it before the client binds it, so the test that
    uses one reruns on a lost port (#2216)."""
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


async def settle(server, **fields):
    """Wait until each named counter reaches its value, then sample all."""
    async with asyncio.timeout(2):
        while True:
            counters = await server.cov_counters()
            if all(counters[name] == value for name, value in fields.items()):
                return counters
            await asyncio.sleep(0.01)


class CovPolicyRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_tightened_policy_moves_quota_capacity_indefinite_and_fanout_counters(self):
        await rerun_on_lost_port(self.tightened_policy)

    async def tightened_policy(self):
        reserved_port = free_udp_port()
        reserved_mac = bytes([127, 0, 0, 1]) + reserved_port.to_bytes(2, "big")
        # Unreserved peers share 3 - 1 = 2 slots; the reserved peer may use all 3.
        server = local_server(cov_policy={
            "max_subscriptions_global": 3,
            "max_subscriptions_per_peer": 2,
            "reserved_capacity": 1,
            "reserved_peers": [reserved_mac],
            "allow_indefinite_subscriptions": False,
            "max_notifications_per_event": 1,
        })
        await server.start()
        try:
            address = await server.local_address()
            async with local_client() as ordinary, local_client(reserved_port) as reserved:
                no_space = (ErrorClass.RESOURCES, ErrorCode.NO_SPACE_TO_ADD_LIST_ELEMENT)
                unsupported = (ErrorClass.SERVICES, ErrorCode.OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED)

                async def refused(client, process_id, lifetime, error):
                    with self.assertRaises(BacnetProtocolError) as caught:
                        await client.subscribe_cov(address, process_id, TARGET,
                                                   confirmed=False, lifetime=lifetime)
                    self.assertEqual((caught.exception.error_class, caught.exception.error_code),
                                     (error[0].to_raw(), error[1].to_raw()))

                await ordinary.subscribe_cov(address, 1, TARGET, confirmed=False, lifetime=60)
                await ordinary.subscribe_cov(address, 2, TARGET, confirmed=False, lifetime=60)
                # The ordinary peer is at its quota of 2.
                await refused(ordinary, 3, 60, no_space)
                counters = await server.cov_counters()
                self.assertEqual(counters["subscriptions_rejected_quota"], 1)
                # Indefinite lifetimes are refused outright, before any quota.
                await refused(ordinary, 4, None, unsupported)
                counters = await server.cov_counters()
                self.assertEqual(counters["subscriptions_rejected_indefinite"], 1)

                # The unreserved share is full, but the reserved peer gets the
                # third slot; its next request finds the table full.
                await reserved.subscribe_cov(address, 1, TARGET, confirmed=False, lifetime=60)
                await refused(reserved, 2, 60, no_space)
                counters = await settle(server, notifications_sent=3)
                self.assertEqual(
                    {name: counters[name] for name in (
                        "subscriptions_active", "subscriptions_created",
                        "subscriptions_rejected_quota", "subscriptions_rejected_capacity",
                        "subscriptions_rejected_indefinite", "notifications_throttled_fanout")},
                    {"subscriptions_active": 3, "subscriptions_created": 3,
                     "subscriptions_rejected_quota": 1, "subscriptions_rejected_capacity": 1,
                     "subscriptions_rejected_indefinite": 1, "notifications_throttled_fanout": 0},
                )

                # One change, three subscribers, a budget of one notification.
                await server.set_present_value_local(TARGET, PropertyValue.real(15.0))
                counters = await settle(server, notifications_throttled_fanout=2)
                self.assertEqual(counters["notifications_sent"], 4)
                self.assertEqual(counters["notifications_unconfirmed"], 4)
        finally:
            await server.stop()

    async def test_omitted_none_and_empty_policy_keep_the_defaults(self):
        for policy in [{}, {"cov_policy": None}, {"cov_policy": {}}]:
            with self.subTest(policy=policy):
                server = local_server(**policy)
                await server.start()
                try:
                    address = await server.local_address()
                    async with local_client() as client:
                        # Past the tightened caps above: 3 timed and an
                        # indefinite subscription from one peer.
                        for process_id in range(1, 4):
                            await client.subscribe_cov(address, process_id, TARGET,
                                                       confirmed=False, lifetime=60)
                        await client.subscribe_cov(address, 4, TARGET, confirmed=False)
                        await settle(server, notifications_sent=4)
                        await server.set_present_value_local(TARGET, PropertyValue.real(15.0))
                        counters = await settle(server, notifications_sent=8)
                        self.assertEqual(counters["subscriptions_active"], 4)
                        for name in ("subscriptions_rejected_quota",
                                     "subscriptions_rejected_capacity",
                                     "subscriptions_rejected_indefinite",
                                     "notifications_throttled_fanout",
                                     "notifications_throttled_peer"):
                            self.assertEqual(counters[name], 0, name)
                finally:
                    await server.stop()


class CovPolicyConstructorTests(unittest.TestCase):
    def test_keyword_only_with_a_none_default(self):
        parameter = inspect.signature(BACnetServer).parameters["cov_policy"]
        self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertIsNone(parameter.default)

    def test_invalid_policies_fail_at_construction(self):
        for policy, error, message in [
            ([("max_subscriptions_global", 1)], TypeError, r"is not an instance of 'dict'"),
            ({"max_subscriptions": 1}, TypeError,
             r"^cov_policy got an unexpected key 'max_subscriptions'$"),
            ({1: 1}, TypeError, r"^cov_policy keys must be str$"),
            ({"max_subscriptions_global": 1.5}, TypeError,
             r"^cov_policy\['max_subscriptions_global'\]: "),
            ({"max_subscriptions_global": "10"}, TypeError,
             r"^cov_policy\['max_subscriptions_global'\]: "),
            ({"allow_indefinite_subscriptions": 1}, TypeError,
             r"^cov_policy\['allow_indefinite_subscriptions'\]: "),
            ({"reserved_peers": b"\x01"}, TypeError, r"^cov_policy\['reserved_peers'\]: "),
            ({"reserved_recipients": [b"\x01"]}, TypeError,
             r"^cov_policy\['reserved_recipients'\]: "),
            ({"max_subscriptions_per_peer": -1}, OverflowError,
             r"^cov_policy\['max_subscriptions_per_peer'\]: "),
            ({"max_notification_bytes_per_event": 1 << 200}, OverflowError,
             r"^cov_policy\['max_notification_bytes_per_event'\]: "),
            ({"reserved_recipients": [(70000, b"\x01")]}, OverflowError,
             r"^cov_policy\['reserved_recipients'\]: "),
            ({"max_subscriptions_global": 0}, ValueError,
             r"max_subscriptions_global must be positive$"),
            ({"max_confirmed_in_flight_per_peer": 0}, ValueError,
             r"max_confirmed_in_flight_per_peer must be positive$"),
            ({"reserved_peers": [b""]}, ValueError,
             r"reserved_peers entries need a MAC of 1\.\.=18 octets$"),
            ({"reserved_peers": [bytes(19)]}, ValueError,
             r"reserved_peers entries need a MAC of 1\.\.=18 octets$"),
            ({"reserved_recipients": [(None, bytes(19))]}, ValueError,
             r"reserved_recipients entries need a MAC of 1\.\.=18 octets$"),
            ({"reserved_recipients": [(65534, bytes(19))]}, ValueError,
             r"reserved_recipients entries need a MAC of 1\.\.=18 octets$"),
            ({"reserved_recipients": [(0, b"\x01")]}, ValueError,
             r"reserved_recipients networks must be 1\.\.=65534$"),
        ]:
            for transport in ["bip", "sc"]:
                with self.subTest(policy=policy, transport=transport):
                    # Refused before the SC credential check, let alone I/O.
                    with self.assertRaisesRegex(error, message):
                        BACnetServer(123, transport=transport, cov_policy=policy)

    def test_valid_policies_construct(self):
        for policy in [
            {"max_subscriptions_global": (1 << 64) - 1},
            {"reserved_capacity": 0, "max_indefinite_per_peer": 0},
            {"reserved_recipients": [(None, b"\x01"), (65534, bytes(18))]},
            {"reserved_peers": [bytes(18)], "reserved_recipients": [(None, bytes(18))]},
        ]:
            with self.subTest(policy=policy):
                BACnetServer(123, cov_policy=policy)

    def test_installed_stub_shape(self):
        spec = importlib.util.find_spec("rusty_bacnet")
        assert spec is not None and spec.origin is not None
        origin = Path(spec.origin)
        candidates = [origin.with_suffix(".pyi"), origin.parent / "rusty_bacnet.pyi"]
        stub = next(path for path in candidates if path.exists())
        classes = {node.name: node for node in ast.parse(stub.read_text()).body
                   if isinstance(node, ast.ClassDef)}
        policy = classes["CovPolicy"]
        self.assertEqual([ast.unparse(base) for base in policy.bases], ["TypedDict"])
        self.assertEqual({keyword.arg: ast.unparse(keyword.value)
                          for keyword in policy.keywords}, {"total": "False"})
        fields = {node.target.id: ast.unparse(node.annotation)
                  for node in policy.body
                  if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name)}
        self.assertEqual(fields, KEYS)
        init = next(node for node in classes["BACnetServer"].body
                    if isinstance(node, ast.FunctionDef) and node.name == "__init__")
        argument = next(arg for arg in init.args.kwonlyargs if arg.arg == "cov_policy")
        assert argument.annotation is not None
        self.assertEqual(ast.unparse(argument.annotation), "CovPolicy | None")


if __name__ == "__main__":
    unittest.main()
