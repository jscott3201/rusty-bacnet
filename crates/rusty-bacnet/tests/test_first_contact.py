"""A device named by its instance is placed only where it answered the
client's own question of which device it is (#2824): an I-Am, which anyone
can send, is only a claim. On loopback, behind a relay whose address
another device answers at."""

from __future__ import annotations

import unittest

from rusty_bacnet import BacnetLookupError
from test_router_application import cleanup
from test_watch import client, i_am, mac_of, plant, relay_to, table_holds, until

FORGED = 7302
"""The instance an I-Am from the relay's address claims."""

AT_RELAY = 7301
"""The device that answers at the relay's address."""


class FirstContact(unittest.IsolatedAsyncioTestCase):
    async def test_a_forged_claim_raises_a_lookup_error_naming_the_address_asked(self) -> None:
        server = await plant(AT_RELAY)
        self.addAsyncCleanup(cleanup, server.stop)
        relay = await relay_to(await server.local_address())
        self.addCleanup(relay.close)
        address = relay.address()
        async with client() as c:
            # The relay learns the client's address from the discovery,
            # then hands it an I-Am for FORGED from the relay's address.
            await c.discover(scope=address, wait=0.3)
            relay.answer(i_am(FORGED))
            await until(lambda: table_holds(c, {(FORGED, address)}), 10, "the forged row")
            with self.assertRaises(BacnetLookupError) as caught:
                await c.device(FORGED)
            error = caught.exception
            self.assertEqual((error.target, error.instance), (str(FORGED), FORGED))
            self.assertEqual(error.at, (0, mac_of(address)))
            self.assertEqual((error.answer, error.answer_instance),
                             ("another-device", AT_RELAY))
            self.assertIsNone(error.candidate)
            self.assertEqual(error.candidates, [])
            self.assertIn(f"isn't confirmed at {address}", str(error))
            self.assertIn(f"answered as device {AT_RELAY}", str(error))


if __name__ == "__main__":
    unittest.main()
