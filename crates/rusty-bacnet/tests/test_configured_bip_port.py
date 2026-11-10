"""Configured snapshot API and independently specified loopback property results."""
import inspect
import unittest

from rusty_bacnet import (
    BACnetClient, BACnetServer, BipEndpoint, BacnetProtocolError,
    ObjectIdentifier, ObjectType, PropertyIdentifier as P, PropertyValue as V,
)


class ConfiguredBipPortTests(unittest.IsolatedAsyncioTestCase):
    def test_constructor_validation_and_greenfield_api(self):
        server = BACnetServer(2867, interface="127.0.0.1", port=0)
        self.assertFalse(hasattr(server, "add_network_port"))
        signature = inspect.signature(server.add_bip_network_port)
        self.assertEqual(signature.parameters["ip_address"].kind, inspect.Parameter.KEYWORD_ONLY)
        for instance in (0, 256, 4194303):
            with self.subTest(instance=instance), self.assertRaises((ValueError, BacnetProtocolError)):
                server.add_bip_network_port(instance, "bad")
        for kwargs in ({"apdu_length": 49}, {"network_number": 65535}, {"dns_servers": []},
                       {"dns_servers": ["::1"]}, {"ip_address": "1.2.3"},
                       {"subnet_mask": "::"}, {"default_gateway": "bad"}):
            with self.subTest(kwargs=kwargs), self.assertRaises((ValueError, BacnetProtocolError)):
                server.add_bip_network_port(1, "bad", **kwargs)
        # Configured local ID and ephemeral UDP are distinct; 51 is not a Device62 size.
        self.assertIsNone(server.add_bip_network_port(255, "valid", udp_port=0, apdu_length=51))
        for instance in (0, 256):
            with self.subTest(endpoint_instance=instance), self.assertRaises((ValueError, BacnetProtocolError)):
                BipEndpoint(2867, interface="127.0.0.1", port=0, network_port_instance=instance)

    async def test_real_bip_reads_snapshot_and_refuses_activation_writes(self):
        server = BACnetServer(2867, interface="127.0.0.1", port=0)
        server.add_bip_network_port(1, "configured", ip_address="192.0.2.10", udp_port=0,
                                   network_number=42, apdu_length=51,
                                   dns_servers=["192.0.2.53", "198.51.100.53"])
        oid = ObjectIdentifier(ObjectType.NETWORK_PORT, 1)
        await server.start()
        try:
            async with BACnetClient(interface="127.0.0.1", port=0) as client:
                address = await server.local_address()
                for prop, expected in (
                    (P.NETWORK_TYPE, V.enumerated(5)), (P.PROTOCOL_LEVEL, V.enumerated(2)),
                    (P.NETWORK_NUMBER, V.unsigned(42)), (P.NETWORK_NUMBER_QUALITY, V.enumerated(3)),
                    (P.APDU_LENGTH, V.unsigned(51)), (P.BACNET_IP_MODE, V.enumerated(0)),
                    (P.IP_ADDRESS, V.octet_string(bytes([192, 0, 2, 10]))),
                    (P.BACNET_IP_UDP_PORT, V.unsigned(0)),
                    (P.MAC_ADDRESS, V.octet_string(bytes([192, 0, 2, 10, 0, 0]))),
                    (P.IP_SUBNET_MASK, V.octet_string(bytes(4))),
                    (P.CHANGES_PENDING, V.boolean(False)),
                ):
                    with self.subTest(property=prop):
                        self.assertEqual(await client.read_property(address, oid, prop), expected)
                        with self.assertRaises(BacnetProtocolError):
                            await client.write_property(address, oid, prop, expected)
                        self.assertEqual(await client.read_property(address, oid, prop), expected)
                for index, expected in ((0, V.unsigned(2)),
                                        (1, V.octet_string(bytes([192, 0, 2, 53]))),
                                        (2, V.octet_string(bytes([198, 51, 100, 53])))):
                    self.assertEqual(await client.read_property(address, oid, P.IP_DNS_SERVER, array_index=index), expected)
                for prop, index in ((P.IP_DNS_SERVER, 3), (P.MAX_APDU_LENGTH_ACCEPTED, None),
                                    (P.COMMAND, None)):
                    with self.assertRaises(BacnetProtocolError):
                        await client.read_property(address, oid, prop, array_index=index)
                # The real socket never rewrites the configured port-zero snapshot.
                self.assertFalse(address.endswith(":0"))
        finally:
            await server.stop()
