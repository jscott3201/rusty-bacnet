"""Installed-native DCC local policy, not source authentication or hardware proof."""
import asyncio
import inspect
import socket
import unittest
from typing import Any

from rusty_bacnet import BACnetServer


class DccConstructorTests(unittest.TestCase):
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
            BACnetServer(123, transport=transport, dcc_policy="require_password", dcc_password="x")
            for policy in ["deny_all", "legacy_permissive"]:
                for password in [None, "", "x" * 100]:
                    BACnetServer(123, transport=transport, dcc_policy=policy, dcc_password=password)


class DccNativeTests(unittest.IsolatedAsyncioTestCase):
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
                                        self.assertEqual(await server.comm_state(), mode)
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
            self.assertEqual(await server.comm_state(), 2)
            self.assertEqual((await self.exchange(server, sock, 2, 2, None, False, 0))[0], 0x20)
            async with asyncio.timeout(2):
                while await server.comm_state() != 0:
                    await asyncio.sleep(0)
        finally:
            await server.stop()
            sock.close()
