"""Instance 4194303 is reserved: no object may have it, so no server or
endpoint can be built as that Device (#2171). Each constructor raises
ValueError, before anything binds or dials; 4194302 is still accepted."""

import unittest

from rusty_bacnet import BACnetServer, BipEndpoint, MstpEndpoint, ScEndpoint

RESERVED = 4_194_303
LAST = 4_194_302

SC = {
    "sc_hub": "wss://localhost:1",
    "sc_vmac": b"\x02\x00\x00\x00\x00\x01",
    "sc_ca_cert": "absent-ca.pem",
    "sc_client_cert": "absent-cert.pem",
    "sc_client_key": "absent-key.pem",
    "sc_device_uuid": bytes.fromhex("8e62ac46d7084226913776a32b619315"),
}

CONSTRUCTORS = {
    "BACnetServer": lambda instance: BACnetServer(
        device_instance=instance, device_name="D", interface="127.0.0.1", port=0
    ),
    "BipEndpoint": lambda instance: BipEndpoint(
        device_instance=instance, interface="127.0.0.1", port=0
    ),
    "ScEndpoint": lambda instance: ScEndpoint(device_instance=instance, **SC),
    "MstpEndpoint": lambda instance: MstpEndpoint(
        device_instance=instance, serial_port="/tmp/nonexistent"
    ),
}


class ReservedDeviceInstanceTests(unittest.TestCase):
    def test_every_constructor_refuses_the_reserved_instance(self) -> None:
        for name, build in CONSTRUCTORS.items():
            for instance in (RESERVED, RESERVED + 1):
                with self.subTest(name, instance=instance):
                    with self.assertRaisesRegex(ValueError, "instance number"):
                        build(instance)

    def test_every_constructor_takes_the_last_addressable_instance(self) -> None:
        for name, build in CONSTRUCTORS.items():
            with self.subTest(name):
                build(LAST)


class ReservedObjectInstanceTests(unittest.IsolatedAsyncioTestCase):
    async def test_start_refuses_an_object_at_the_reserved_instance(self) -> None:
        server = BACnetServer(
            device_instance=1234, device_name="D", interface="127.0.0.1", port=0
        )
        server.add_binary_value(RESERVED, "BV-reserved")
        with self.assertRaisesRegex(ValueError, "^object refused: .*4194303"):
            await server.start()


if __name__ == "__main__":
    unittest.main()
