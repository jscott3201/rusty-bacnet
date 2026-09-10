"""Constructor identity contract against the installed native extension."""
import inspect
import socket
import unittest

from rusty_bacnet import BACnetClient, BACnetServer


class NodeIdentityConstructorTests(unittest.TestCase):
    def test_sc_missing_uuid_is_a_constructor_value_error(self):
        for api, args in [(BACnetClient, ()), (BACnetServer, (3000,))]:
            with self.subTest(api=api.__name__), self.assertRaisesRegex(
                    ValueError, "sc_device_uuid is required"):
                api(*args, transport="sc", sc_ca_cert="missing-ca.pem",
                    sc_client_cert="missing-cert.pem", sc_client_key="missing-key.pem")

    def test_sc_uuid_validation_precedes_file_and_socket_io(self):
        omitted = object()
        for api, args in [(BACnetClient, ()), (BACnetServer, (3000,))]:
            for value, message in [(omitted, "is required"), (None, "is required"),
                                   (b"", "exactly 16 bytes"), (b"a" * 15, "exactly 16 bytes"),
                                   (b"a" * 17, "exactly 16 bytes"), (bytes(16), "all zero")]:
                with self.subTest(api=api.__name__, value=value), socket.socket() as listener:
                    listener.bind(("127.0.0.1", 0))
                    listener.listen()
                    listener.setblocking(False)
                    options = {} if value is omitted else {"sc_device_uuid": value}
                    with self.assertRaisesRegex(ValueError, f"sc_device_uuid .*{message}"):
                        api(*args, transport="sc", sc_hub=f"wss://127.0.0.1:{listener.getsockname()[1]}",
                            sc_vmac=b"\x02\0\0\0\0\2", sc_ca_cert="missing-ca.pem",
                            sc_client_cert="missing-cert.pem", sc_client_key="missing-key.pem",
                            **options)
                    with self.assertRaises(BlockingIOError):
                        listener.accept()
                    # Positive control for the same accept queue.
                    listener.settimeout(3)
                    with socket.create_connection(listener.getsockname(), timeout=3):
                        accepted, _ = listener.accept()
                        accepted.close()

    def test_uuid_keyword_only_and_non_sc_defaults(self):
        for api, args in [(BACnetClient, ()), (BACnetServer, (3000,))]:
            params = inspect.signature(api).parameters
            self.assertEqual(params["sc_device_uuid"].kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertIsNone(params["sc_device_uuid"].default)
            api(*args)
            for transport in ("bip", "ipv6", "mstp"):
                for value in (None, b"", bytes(16), b"too short"):
                    api(*args, transport=transport, sc_device_uuid=value)
            # No version/variant policy was added: nonzero is the whole-array test.
            for value in (b"\x01" + bytes(15), bytes(15) + b"\x01",
                          bytes.fromhex("8e62ac46d7084226913776a32b619315")):
                api(*args, transport="sc", sc_ca_cert="ca.pem", sc_client_cert="cert.pem",
                    sc_client_key="key.pem", sc_device_uuid=value)

    def test_credentials_still_precede_uuid_validation(self):
        for api, args in [(BACnetClient, ()), (BACnetServer, (3000,))]:
            with self.assertRaisesRegex(ValueError, "sc_ca_cert"):
                api(*args, transport="sc", sc_device_uuid=bytes(16))
            with self.assertRaisesRegex(ValueError, "sc_client_cert"):
                api(*args, transport="sc", sc_ca_cert="ca.pem", sc_device_uuid=bytes(16))
            with self.assertRaisesRegex(ValueError, "sc_client_key"):
                api(*args, transport="sc", sc_ca_cert="ca.pem", sc_client_cert="cert.pem",
                    sc_device_uuid=bytes(16))
