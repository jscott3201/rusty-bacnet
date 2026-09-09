"""Recovery settings and native loopback behavior, not hardware availability proof."""
import asyncio
import socket
import unittest
from typing import Any

from rusty_bacnet import BACnetServer


class RecoveryConstructorTests(unittest.TestCase):
    def test_ranges_and_tiny_global_migration_before_all_transports(self):
        for transport in ["bip", "ipv6", "sc", "mstp"]:
            for global_limit, reserve in [(64, 64), (64, 65), (1, 4), (4, 4)]:
                with self.subTest(transport=transport, global_limit=global_limit, reserve=reserve):
                    with self.assertRaisesRegex(ValueError, "confirmed_recovery_reserve"):
                        BACnetServer(123, transport=transport, max_confirmed_in_flight=global_limit,
                                     confirmed_recovery_reserve=reserve)
            for reserve in [0, 4]:
                for value, error in [(0, ValueError), (-1, OverflowError), (1 << 200, OverflowError)]:
                    with self.subTest(transport=transport, reserve=reserve, value=value):
                        with self.assertRaises(error):
                            BACnetServer(123, transport=transport, confirmed_recovery_reserve=reserve,
                                         max_recovery_in_flight_per_peer=value)
            for reserve in [-1, 1 << 200]:
                with self.assertRaises(OverflowError):
                    BACnetServer(123, transport=transport, confirmed_recovery_reserve=reserve)
            for name in ["confirmed_recovery_reserve", "max_recovery_in_flight_per_peer"]:
                with self.assertRaises(TypeError):
                    invalid: dict[str, Any] = {name: 1.5}
                    BACnetServer(123, transport=transport, **invalid)
            BACnetServer(123, transport=transport, max_confirmed_in_flight=1, confirmed_recovery_reserve=0)
            BACnetServer(123, transport=transport, max_confirmed_in_flight=2, confirmed_recovery_reserve=1)
            # Independent quotas accept recovery peer > ordinary peer; Rust
            # barrier tests prove simultaneous 1+3 holding, not this constructor.
            BACnetServer(123, transport=transport, max_confirmed_in_flight=8,
                         confirmed_recovery_reserve=3, max_confirmed_in_flight_per_peer=1,
                         max_recovery_in_flight_per_peer=3)


class RecoveryNativeTests(unittest.IsolatedAsyncioTestCase):
    async def test_wire_classification_password_and_zero_reserve_policy(self):
        # Sequential wire checks establish installed native policy propagation.
        # Rust held-transport barriers prove 60/4 and same-peer 16+1 saturation;
        # sequential UDP replies cannot prove simultaneous independent quotas.
        for reserve in [0, 1]:
            server = BACnetServer(123, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", dcc_password="required",
                                  dcc_policy="require_password",
                                  max_confirmed_in_flight=2, confirmed_recovery_reserve=reserve,
                                  max_recovery_in_flight_per_peer=1)
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.bind(("127.0.0.1", 0))
            sock.setblocking(False)
            loop = asyncio.get_running_loop()
            try:
                await server.start()
                ip, port = (await server.local_address()).rsplit(":", 1)

                async def exchange(invoke, mode, password=None):
                    body = bytes([0x19, mode])
                    if password is not None:
                        content = b"\0" + password.encode()
                        body += bytes([0x2d, len(content)]) + content
                    npdu = b"\x01\x00" + bytes([0, 5, invoke, 17]) + body
                    wire = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
                    await loop.sock_sendto(sock, wire, (ip, int(port)))
                    reply, _ = await asyncio.wait_for(loop.sock_recvfrom(sock, 2048), 2)
                    self.assertEqual(reply[7], invoke)
                    async with asyncio.timeout(2):
                        while True:
                            counters = await server.request_admission_counters()
                            if counters["confirmed_active"] == counters["abort_active"] == 0:
                                break
                            await asyncio.sleep(0)
                    self.assertEqual(counters["recovery_active"], 0)
                    return reply[6:], counters

                # Existing DISABLE rejection is unchanged; use supported
                # DISABLE_INITIATION for the native recovery transition.
                reply, counters = await exchange(1, 2, "required")
                self.assertEqual(reply[0] >> 4, 2)
                self.assertEqual(await server.comm_state(), 2)
                self.assertEqual(counters["recovery_admitted_total"], 0)
                for invoke, password in [(2, None), (3, "wrong")]:
                    reply, counters = await exchange(invoke, 0, password)
                    self.assertEqual(reply[0] >> 4, 5)  # authoritative handler Error
                    self.assertEqual(await server.comm_state(), 2)
                reply, counters = await exchange(4, 0, "required")
                self.assertEqual(reply[0] >> 4, 2)
                self.assertEqual(await server.comm_state(), 0)
                self.assertEqual(counters["confirmed_admitted_total"], 4)
                self.assertEqual(counters["recovery_admitted_total"], 3 if reserve else 0)
                self.assertEqual(counters["confirmed_overloaded_total"], 0)
                self.assertEqual(counters["recovery_overloaded_total"], 0)
            finally:
                await server.stop()
                sock.close()
