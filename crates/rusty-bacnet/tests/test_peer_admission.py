"""Installed-wheel peer quota propagation; deterministic saturation lives in Rust."""
import asyncio
import socket
import unittest

from rusty_bacnet import BACnetServer


class PeerAdmissionRuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_two_udp_sources_peer_rejection_and_recovery(self):
        server = BACnetServer(123, interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1",
                              max_confirmed_in_flight_per_peer=1,
                              max_unconfirmed_in_flight_per_peer=1)
        sockets = [socket.socket(socket.AF_INET, socket.SOCK_DGRAM) for _ in range(2)]
        for sock in sockets:
            sock.bind(("127.0.0.1", 0))
            sock.setblocking(False)
        self.assertNotEqual(sockets[0].getsockname(), sockets[1].getsockname())
        loop = asyncio.get_running_loop()
        try:
            await server.start()
            ip, port = (await server.local_address()).rsplit(":", 1)
            address = (ip, int(port))

            async def send(sock, apdu):
                npdu = b"\x01\x00" + apdu
                wire = b"\x81\x0a" + (4 + len(npdu)).to_bytes(2, "big") + npdu
                await loop.sock_sendto(sock, wire, address)

            async def quiescent():
                async with asyncio.timeout(3):
                    while True:
                        counters = await server.request_admission_counters()
                        if counters["confirmed_active"] == counters["abort_active"] == 0:
                            return counters
                        await asyncio.sleep(0)

            # Same logical peer, distinct requests. Do not assert exact UDP counts.
            counters = await server.request_admission_counters()
            for batch in range(8):
                body = b"\x0c\x02\x00\x00\x7b\x1e" + b"\x09\x4d" * (256 + batch) + b"\x1f"
                for invoke in range(128):
                    await send(sockets[0], bytes([0, 5, invoke, 14]) + body)
                counters = await server.request_admission_counters()
                self.assertLessEqual(counters["confirmed_active"], 1)
                if counters["confirmed_peer_overloaded_total"]:
                    break
            self.assertGreater(counters["confirmed_peer_overloaded_total"], 0)
            counters = await quiescent()
            self.assertEqual(counters["confirmed_global_overloaded_total"], 0)
            self.assertEqual(counters["confirmed_overloaded_total"], counters["confirmed_peer_overloaded_total"])
            self.assertEqual(counters["confirmed_overloaded_total"],
                             counters["abort_admitted_total"] + counters["confirmed_fallback_dropped_total"])
            # Both immediate UDP sources can execute normal requests afterward.
            # Simultaneous second-peer availability is covered by Rust barriers.
            for sock in sockets:
                while True:
                    try:
                        sock.recvfrom(65536)
                    except BlockingIOError:
                        break
                await send(sock, b"\x00\x05\xfa\x0c")
                async with asyncio.timeout(3):
                    while True:
                        wire, _ = await loop.sock_recvfrom(sock, 65536)
                        if len(wire) >= 8 and wire[7] == 250:
                            self.assertEqual(wire[6] >> 4, 5)  # handler Error, not overload Abort
                            break
                await quiescent()
        finally:
            await server.stop()
            for sock in sockets:
                sock.close()
