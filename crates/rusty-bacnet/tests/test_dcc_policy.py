"""Installed-native DCC local policy, not source authentication or hardware proof."""
import ast
import asyncio
import inspect
from pathlib import Path
import socket
import unittest
from typing import Any

import rusty_bacnet
from rusty_bacnet import BACnetClient, BACnetServer, EnableDisable, ReinitializedState


class DccConstructorTests(unittest.TestCase):
    def test_disable_rate_constructor_bounds_and_default(self):
        parameter = inspect.signature(BACnetServer).parameters["dcc_disable_rate_limit"]
        self.assertIsNone(parameter.default)
        self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
        for transport in ["bip", "ipv6", "sc", "mstp"]:
            for invalid in [(0, 20000), (65536, 20000), (3, 0), (3, 86400001),
                            (2**32 - 1, 20000), (3, 2**64 - 1)]:
                with self.assertRaisesRegex(ValueError, "DCC disable rate"):
                    BACnetServer(123, transport=transport, dcc_disable_rate_limit=invalid)
            for valid in [None, (3, 20000), (1, 1), (65535, 86400000)]:
                for policy in ["deny_all", "legacy_permissive", "require_password"]:
                    BACnetServer(123, transport=transport, dcc_policy=policy,
                                 dcc_password="required", dcc_disable_rate_limit=valid,
                                 sc_device_uuid=bytes.fromhex("8e62ac46d7084226913776a32b619315"),
                                 sc_ca_cert="ca.pem", sc_client_cert="cert.pem", sc_client_key="key.pem")
        for invalid in [True, "bad", (), (3,), (3, 20, 1), (-1, 20000), (3, -1),
                        (2**32, 20000), (3, 2**64), (3.5, 20000), (3, None)]:
            with self.assertRaises((TypeError, ValueError, OverflowError)):
                BACnetServer(123, dcc_disable_rate_limit=invalid)

    def test_source_restriction_validation_before_every_transport(self):
        parameter = inspect.signature(BACnetServer).parameters["dcc_source_restriction"]
        self.assertIsNone(parameter.default)
        self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
        for transport in ["bip", "ipv6", "sc", "mstp"]:
            for policy in ["deny_all", "legacy_permissive"]:
                for restriction in [[], [(None, b"x")]]:
                    with self.assertRaisesRegex(ValueError, "source restriction"):
                        BACnetServer(123, transport=transport, dcc_policy=policy,
                                     dcc_source_restriction=restriction)
            # An entry is 1 to 18 octets, BACnetAddress::MAX_MAC_LEN (#1157).
            for restriction in [[(None, b"")], [(None, b"x" * 19)], [(7, b"x" * 19)],
                                [(None, b"x" * 256)], [(0, b"x")],
                                [(65535, b"x")], [(7, b"")], [(None, b"x")] * 257]:
                with self.assertRaises(ValueError):
                    BACnetServer(123, transport=transport, dcc_policy="require_password",
                                 dcc_password="required", dcc_source_restriction=restriction)
            for restriction in [[], [(None, b"x")], [(None, b"x" * 18)],
                                [(65534, b"x" * 18)] * 256]:
                BACnetServer(123, transport=transport, dcc_policy="require_password",
                             dcc_password="required", dcc_source_restriction=restriction,
                             sc_device_uuid=bytes.fromhex("8e62ac46d7084226913776a32b619315"),
                             sc_ca_cert="ca.pem", sc_client_cert="cert.pem", sc_client_key="key.pem")
        for invalid in [1, "bad", [(None, "bad")], [(65536, b"x")], [(-1, b"x")]]:
            with self.assertRaises((TypeError, ValueError, OverflowError)):
                BACnetServer(123, dcc_policy="require_password", dcc_password="required",
                             dcc_source_restriction=invalid)

    def test_policy_validation_before_every_transport(self):
        parameter = inspect.signature(BACnetServer).parameters["dcc_policy"]
        self.assertEqual(parameter.default, "deny_all")
        self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
        for transport in ["bip", "ipv6", "sc", "mstp"]:
            for invalid in ["", "DenyAll", "DENY_ALL", "require-password", "unknown"]:
                with self.subTest(transport=transport, invalid=invalid):
                    with self.assertRaisesRegex(ValueError, "dcc_policy"):
                        BACnetServer(123, transport=transport, dcc_policy=invalid)
            for invalid in [None, 1, b"deny_all"]:
                with self.assertRaises(TypeError):
                    BACnetServer(123, transport=transport, dcc_policy=invalid)
            for password in [None, ""]:
                with self.assertRaisesRegex(ValueError, "nonempty dcc_password"):
                    BACnetServer(123, transport=transport, dcc_policy="require_password",
                                 dcc_password=password)
            BACnetServer(123, transport=transport, dcc_policy="require_password", dcc_password="x",
                         sc_device_uuid=bytes.fromhex("8e62ac46d7084226913776a32b619315"),
                         sc_ca_cert="ca.pem", sc_client_cert="cert.pem", sc_client_key="key.pem")
            for policy in ["deny_all", "legacy_permissive"]:
                for password in [None, "x" * 20]:
                    BACnetServer(123, transport=transport, dcc_policy=policy, dcc_password=password,
                                 sc_device_uuid=bytes.fromhex("8e62ac46d7084226913776a32b619315"),
                                 sc_ca_cert="ca.pem", sc_client_cert="cert.pem", sc_client_key="key.pem")

    def test_password_over_twenty_characters_refused_in_every_policy(self):
        # The limit counts characters, not octets (#2060); a longer password
        # could never be sent, whatever the policy.
        for policy in ["deny_all", "require_password", "legacy_permissive"]:
            with self.subTest(policy=policy):
                with self.assertRaisesRegex(ValueError, "dcc_password exceeds the 20-character limit"):
                    BACnetServer(123, dcc_policy=policy, dcc_password="é" * 21)
                BACnetServer(123, dcc_policy=policy, dcc_password="é" * 20)

    def test_empty_password_refused_in_every_policy(self):
        # #2094: a request's empty password decodes as none, so a configured
        # empty one could never match; None is how to configure no password.
        for policy in ["deny_all", "require_password", "legacy_permissive"]:
            with self.subTest(policy=policy):
                with self.assertRaisesRegex(ValueError, "empty dcc_password"):
                    BACnetServer(123, dcc_policy=policy, dcc_password="")

    def test_comm_state_stub_returns_enable_disable(self):
        stub_path = Path(rusty_bacnet.__file__).with_suffix(".pyi")
        tree = ast.parse(stub_path.read_text(encoding="utf-8"), filename=str(stub_path))
        methods = {method.name: method
                   for node in tree.body
                   if isinstance(node, ast.ClassDef) and node.name == "BACnetServer"
                   for method in node.body if isinstance(method, ast.FunctionDef)}
        self.assertIn("comm_state", methods)
        returns = methods["comm_state"].returns
        self.assertEqual(returns and ast.unparse(returns), "Awaitable[EnableDisable]")


class DccNativeTests(unittest.IsolatedAsyncioTestCase):
    async def test_disable_rate_global_native_lifetime_and_enable_exemption(self):
        for policy in ["legacy_permissive", "require_password"]:
            server = BACnetServer(123, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", dcc_policy=policy,
                                  dcc_password="required", dcc_disable_rate_limit=(3, 20000))
            sockets = [socket.socket(socket.AF_INET, socket.SOCK_DGRAM) for _ in range(2)]
            for sock in sockets:
                sock.bind(("127.0.0.1", 0))
                sock.setblocking(False)
            try:
                # Same Python wrapper, distinct native servers: restart resets budget.
                for _ in range(2):
                    await server.start()
                    for invoke in range(1, 7):
                        reply = await self.exchange(server, sockets[invoke % 2], invoke, 2,
                                                    "required", invoke % 2 == 0)
                        self.assertEqual(reply, bytes([0x20, invoke, 17]) if invoke <= 3 else
                                         bytes([0x50, invoke, 17, 0x91, 5, 0x91, 29]))
                    for invoke in range(7, 10):
                        reply = await self.exchange(server, sockets[0], invoke, 0, "required", False)
                        self.assertEqual(reply, bytes([0x20, invoke, 17]))
                    self.assertEqual(await server.comm_state(), EnableDisable.ENABLE)
                    reply = await self.exchange(server, sockets[1], 10, 2, "required", True)
                    self.assertEqual(reply, bytes([0x50, 10, 17, 0x91, 5, 0x91, 29]))
                    self.assertEqual(await server.comm_state(), EnableDisable.ENABLE)
                    self.assertEqual(await server.dcc_outcome_counters(), dict(
                        accepted_total=6, policy_denied_total=4, password_failure_total=0,
                        deprecated_denied_total=0, malformed_total=0))
                    self.assertEqual((await server.request_admission_counters())["recovery_admitted_total"], 3)
                    await server.stop()
            finally:
                await server.stop()
                for sock in sockets:
                    sock.close()

    async def test_disable_rate_password_deprecated_and_configurable_refill(self):
        server = BACnetServer(123, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1", dcc_policy="require_password",
                              dcc_password="required", dcc_disable_rate_limit=(1, 200))
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        sock.setblocking(False)
        try:
            await server.start()
            reply = await self.exchange(server, sock, 1, 2, "wrong", False)
            self.assertEqual(reply, bytes([0x50, 1, 17, 0x91, 4, 0x91, 26]))
            reply = await self.exchange(server, sock, 2, 1, "required", False)
            self.assertEqual(reply, bytes([0x50, 2, 17, 0x91, 5, 0x91, 29]))
            self.assertEqual((await self.exchange(server, sock, 3, 2, "required", False))[0], 0x20)
            await asyncio.sleep(0.25)
            self.assertEqual((await self.exchange(server, sock, 4, 2, "required", True))[0], 0x20)
        finally:
            await server.stop()
            sock.close()

    async def test_source_restriction_direct_routed_and_outcomes(self):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        sock.setblocking(False)
        direct = socket.inet_aton("127.0.0.1") + sock.getsockname()[1].to_bytes(2, "big")
        restrictions: list[list[tuple[int | None, bytes]] | None] = [
            None, [], [(None, direct)], [(7, b"\x2a")], [(8, b"\x2a")], [(7, b"\x2b")],
        ]
        try:
            for kind, restriction in enumerate(restrictions):
                server = BACnetServer(123, interface="127.0.0.1", port=0,
                                      broadcast_address="127.0.0.1", dcc_policy="require_password",
                                      dcc_password="required", dcc_source_restriction=restriction)
                try:
                    await server.start()
                    invoke = 0
                    expected = dict(accepted_total=0, policy_denied_total=0, password_failure_total=0,
                                    deprecated_denied_total=0, malformed_total=0)
                    for routed in [False, True]:
                        for mode in [2, 0, 1]:
                            for password in [None, "wrong", "required"]:
                                invoke += 1
                                before = await server.comm_state()
                                reply = await self.exchange(server, sock, invoke, mode, password, routed)
                                allowed = kind == 0 or (kind == 2 and not routed) or (kind == 3 and routed)
                                outcome = ("password_failure" if password != "required" else
                                           "deprecated_denied" if mode == 1 else
                                           "policy_denied" if not allowed else "accepted")
                                expected[outcome + "_total"] += 1
                                if outcome == "accepted":
                                    self.assertEqual(reply, bytes([0x20, invoke, 17]))
                                    self.assertEqual(await server.comm_state(),
                                                     EnableDisable.from_raw(mode))
                                else:
                                    bad = outcome == "password_failure"
                                    self.assertEqual(reply, bytes([0x50, invoke, 17, 0x91,
                                                                  4 if bad else 5, 0x91, 26 if bad else 29]))
                                    self.assertEqual(await server.comm_state(), before)
                                self.assertEqual(await server.dcc_outcome_counters(), expected)
                    self.assertEqual((await server.request_admission_counters())["recovery_admitted_total"], 6)
                finally:
                    await server.stop()
        finally:
            sock.close()

    async def test_comm_state_returns_enable_disable(self):
        server = BACnetServer(123, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1", dcc_policy="legacy_permissive")
        with self.assertRaisesRegex(RuntimeError, "^server not started$"):
            await server.comm_state()
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        sock.setblocking(False)
        try:
            await server.start()
            state = await server.comm_state()
            self.assertIsInstance(state, EnableDisable)
            self.assertEqual(state, EnableDisable.ENABLE)
            self.assertEqual((await self.exchange(server, sock, 1, 2, None, False))[0], 0x20)
            state = await server.comm_state()
            self.assertIsInstance(state, EnableDisable)
            self.assertEqual(state, EnableDisable.DISABLE_INITIATION)
            self.assertEqual(state.to_raw(), 2)
            self.assertEqual(hash(state), hash(EnableDisable.DISABLE_INITIATION))
            self.assertEqual((await self.exchange(server, sock, 2, 0, None, False))[0], 0x20)
            self.assertEqual(await server.comm_state(), EnableDisable.ENABLE)
        finally:
            await server.stop()
            sock.close()
        with self.assertRaisesRegex(RuntimeError, "^server not started$"):
            await server.comm_state()

    async def exchange(self, server, sock, invoke, mode, password, routed, duration=None):
        body = b"" if duration is None else bytes([0x09, duration])
        body += bytes([0x19, mode])
        if password is not None:
            content = b"\0" + password.encode()
            body += bytes([0x2d, len(content)]) + content
        # Routed source is an address, deliberately not an authenticated principal.
        header = b"\x01\x08\x00\x07\x01\x2a" if routed else b"\x01\x00"
        npdu = header + bytes([0, 5, invoke, 17]) + body
        wire = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
        ip, port = (await server.local_address()).rsplit(":", 1)
        loop = asyncio.get_running_loop()
        await loop.sock_sendto(sock, wire, (ip, int(port)))
        reply, _ = await asyncio.wait_for(loop.sock_recvfrom(sock, 2048), 2)
        self.assertEqual(reply[:2], b"\x81\x0a")
        self.assertEqual(int.from_bytes(reply[2:4], "big"), len(reply))
        offset = 6
        if reply[5] & 0x20:
            offset += 4 + reply[8]  # DNET, DLEN, DADR, hop count
        apdu = reply[offset:]
        self.assertEqual(apdu[1:3], bytes([invoke, 17]))
        async with asyncio.timeout(2):
            while (await server.request_admission_counters())["confirmed_active"]:
                await asyncio.sleep(0)
        return apdu

    async def test_default_and_explicit_modes_wire_precedence(self):
        for policy in [None, "require_password", "legacy_permissive"]:
            for configured in [None, "required"]:
                if policy == "require_password" and configured is None:
                    continue
                options: dict[str, Any] = {} if policy is None else {"dcc_policy": policy}
                server = BACnetServer(123, interface="127.0.0.1", port=0,
                                      broadcast_address="127.0.0.1", dcc_password=configured, **options)
                sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                sock.bind(("127.0.0.1", 0))
                sock.setblocking(False)
                try:
                    await server.start()
                    invoke = 0
                    for routed in [False, True]:
                        for mode in [2, 0, 1]:
                            for password in [None, "wrong", "required"]:
                                invoke += 1
                                before = await server.comm_state()
                                reply = await self.exchange(server, sock, invoke, mode, password, routed)
                                bad_password = configured is not None and password != configured
                                denied = bad_password or policy is None or mode == 1
                                with self.subTest(policy=policy, configured=configured, mode=mode,
                                                  password=password, routed=routed):
                                    if denied:
                                        # Independent Error encoding: class SECURITY=4/code PASSWORD_FAILURE=26;
                                        # class SERVICES=5/code SERVICE_REQUEST_DENIED=29.
                                        self.assertEqual(reply, bytes([0x50, invoke, 17, 0x91,
                                                                      4 if bad_password else 5, 0x91,
                                                                      26 if bad_password else 29]))
                                        self.assertEqual(await server.comm_state(), before)
                                    else:
                                        self.assertEqual(reply, bytes([0x20, invoke, 17]))
                                        self.assertEqual(await server.comm_state(),
                                                         EnableDisable.from_raw(mode))
                    counters = await server.request_admission_counters()
                    self.assertEqual(counters["recovery_admitted_total"], 6)
                finally:
                    await server.stop()
                    sock.close()

    async def test_native_zero_and_indefinite_legacy_timer_semantics(self):
        server = BACnetServer(123, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1", dcc_policy="legacy_permissive")
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        sock.setblocking(False)
        try:
            await server.start()
            self.assertEqual((await self.exchange(server, sock, 1, 2, None, False))[0], 0x20)
            await asyncio.sleep(0.02)
            self.assertEqual(await server.comm_state(), EnableDisable.DISABLE_INITIATION)
            self.assertEqual((await self.exchange(server, sock, 2, 2, None, False, 0))[0], 0x20)
            async with asyncio.timeout(2):
                while await server.comm_state() != EnableDisable.ENABLE:
                    await asyncio.sleep(0)
        finally:
            await server.stop()
            sock.close()

    async def test_client_refuses_an_over_long_password_before_sending(self):
        # #2085: 21 characters raise ValueError at the call and nothing reaches
        # the server; 20 two-octet characters go out and match.
        server = BACnetServer(123, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1", dcc_policy="require_password",
                              dcc_password="é" * 20)
        await server.start()
        try:
            address = await server.local_address()
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                with self.assertRaisesRegex(ValueError, "password exceeds the 20-character limit"):
                    client.device_communication_control(address, EnableDisable.DISABLE,
                                                        password="é" * 21)
                with self.assertRaisesRegex(ValueError, "password exceeds the 20-character limit"):
                    client.reinitialize_device(address, ReinitializedState.COLDSTART,
                                               password="é" * 21)
                # #2094: an empty password is refused too; None sends none.
                with self.assertRaisesRegex(ValueError, "empty password"):
                    client.device_communication_control(address, EnableDisable.DISABLE,
                                                        password="")
                with self.assertRaisesRegex(ValueError, "empty password"):
                    client.reinitialize_device(address, ReinitializedState.COLDSTART,
                                               password="")
                await client.device_communication_control(
                    address, EnableDisable.DISABLE_INITIATION, time_duration=1,
                    password="é" * 20)
            self.assertEqual(await server.comm_state(), EnableDisable.DISABLE_INITIATION)
            self.assertEqual(await server.dcc_outcome_counters(), dict(
                accepted_total=1, policy_denied_total=0, password_failure_total=0,
                deprecated_denied_total=0, malformed_total=0))
        finally:
            await server.stop()
