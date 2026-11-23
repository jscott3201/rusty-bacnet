"""Local server database errors precede any SC network connection attempt."""
import asyncio

from rusty_bacnet import BACnetServer
from test_sc_hub_mtls import MtlsFixture, SERVER_UUID


class ServerLocalValidationTests(MtlsFixture):
    async def test_invalid_local_database_does_not_dial_sc(self):
        for collision in ("pending_names", "device_name"):
            with self.subTest(collision=collision):
                accepted = []
                observed = asyncio.Event()

                def connected(_reader, writer):
                    accepted.append(writer)
                    observed.set()
                    # A mistaken dial terminates promptly; no TLS timeout oracle.
                    writer.close()

                listener = await asyncio.start_server(connected, "127.0.0.1", 0)
                address = listener.sockets[0].getsockname()
                node = BACnetServer(
                    893, "Selected Device", transport="sc",
                    sc_hub=f"wss://127.0.0.1:{address[1]}",
                    sc_vmac=b"\x02\0\0\0\0\2", sc_device_uuid=SERVER_UUID,
                    sc_ca_cert=self.path("site.pem"),
                    sc_client_cert=self.path("server.pem"),
                    sc_client_key=self.path("server.key"),
                )
                # A point builder refuses a name in use at the call (#2711);
                # a File object's builder leaves the clash to the database.
                if collision == "pending_names":
                    node.add_analog_input(0, "duplicate", 64, 72.5)
                    node.add_file(1, "duplicate")
                else:
                    node.add_file(0, "Selected Device")
                try:
                    caught = None
                    try:
                        await asyncio.wait_for(node.start(), 3)
                    except Exception as error:
                        caught = error
                    self.assertEqual(
                        len(accepted), 0,
                        f"local {collision} error attempted SC dial; returned {caught!r}",
                    )
                    self.assertIsInstance(caught, ValueError)
                    self.assertIn("duplicate object name", str(caught))
                    # This exact listener must observe a real connection. Since
                    # start has completed, no future can defer a hidden dial.
                    _reader, writer = await asyncio.open_connection(*address)
                    writer.close()
                    await writer.wait_closed()
                    await asyncio.wait_for(observed.wait(), 3)
                    self.assertEqual(len(accepted), 1)
                finally:
                    await node.stop()
                    listener.close()
                    await listener.wait_closed()
                    for writer in accepted:
                        writer.close()
                        await writer.wait_closed()
