"""Name clashes in an SC server's database are refused at the builder call
(#2748), so no start reaches a database error, let alone a dial. The refused
dial that keeps the registrations is covered by test_sc_hub_mtls
(``test_a_refused_dial_keeps_the_registrations_for_the_next_start``)."""
from rusty_bacnet import BACnetServer
from test_sc_hub_mtls import MtlsFixture, SERVER_UUID


class ServerLocalValidationTests(MtlsFixture):
    def node(self):
        return BACnetServer(
            893, "Selected Device", transport="sc",
            sc_hub="wss://127.0.0.1:1",
            sc_vmac=b"\x02\0\0\0\0\2", sc_device_uuid=SERVER_UUID,
            sc_ca_cert=self.path("site.pem"),
            sc_client_cert=self.path("server.pem"),
            sc_client_key=self.path("server.key"),
        )

    def test_a_name_clash_is_refused_at_the_call(self):
        node = self.node()
        node.add_analog_input(0, "duplicate", 64, 72.5)
        with self.assertRaisesRegex(ValueError, "duplicate object name: duplicate"):
            node.add_file(1, "duplicate")
        self.assertEqual(node._pending_registration_count(), 1)

    def test_the_devices_name_is_refused_at_the_call(self):
        node = self.node()
        with self.assertRaisesRegex(ValueError, "duplicate object name: Selected Device"):
            node.add_file(0, "Selected Device")
        self.assertEqual(node._pending_registration_count(), 0)
