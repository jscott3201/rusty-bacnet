"""Installed-native SC reconnect options and constructor contract."""
import inspect
import unittest

from rusty_bacnet import BACnetClient, ScReconnect


class ScReconnectOptionsTests(unittest.TestCase):
    def test_keyword_only_defaults_and_frozen_values(self):
        signature = inspect.signature(ScReconnect)
        expected = dict(initial_delay_ms=10_000, max_delay_ms=600_000, max_retries=10)
        self.assertEqual(list(signature.parameters), list(expected))
        options = ScReconnect()
        for name, default in expected.items():
            with self.subTest(name=name):
                self.assertEqual(signature.parameters[name].kind,
                                 inspect.Parameter.KEYWORD_ONLY)
                self.assertEqual(signature.parameters[name].default, default)
                self.assertEqual(getattr(options, name), default)
                with self.assertRaises(AttributeError):
                    setattr(options, name, 99)
        with self.assertRaises(TypeError):
            ScReconnect(10_000)

    def test_semantic_validation_and_unsigned_conversion(self):
        for kwargs in [dict(initial_delay_ms=0), dict(max_delay_ms=0),
                       dict(initial_delay_ms=200, max_delay_ms=100),
                       dict(max_delay_ms=86_400_001)]:
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                ScReconnect(**kwargs)
        for name, bits in [("initial_delay_ms", 64), ("max_delay_ms", 64),
                           ("max_retries", 32)]:
            for value in [-1, 1 << bits]:
                with self.subTest(name=name, value=value), self.assertRaises(OverflowError):
                    ScReconnect(**{name: value})
        zero = ScReconnect(initial_delay_ms=1, max_delay_ms=1, max_retries=0)
        self.assertEqual(zero.max_retries, 0)
        self.assertEqual(repr(zero),
                         "ScReconnect(initial_delay_ms=1, max_delay_ms=1, max_retries=0)")
        # None is unlimited recovery, which needs a one-second initial delay.
        unlimited = ScReconnect(initial_delay_ms=1_000, max_delay_ms=1_000, max_retries=None)
        self.assertIsNone(unlimited.max_retries)
        self.assertEqual(
            repr(unlimited),
            "ScReconnect(initial_delay_ms=1000, max_delay_ms=1000, max_retries=None)")
        self.assertIsNone(ScReconnect(max_retries=None).max_retries)
        for initial in [1, 999]:
            with self.subTest(initial=initial), \
                    self.assertRaisesRegex(ValueError, "at least 1000 with unlimited"):
                ScReconnect(initial_delay_ms=initial, max_retries=None)
        with self.assertRaises(TypeError):
            ScReconnect(retry_forever=True)

    def test_client_keyword_is_appended_and_sc_only(self):
        parameters = inspect.signature(BACnetClient).parameters
        self.assertEqual(list(parameters)[-8:], ["reconnect", "sc_npdu_per_origin_limit",
                                                 "sc_failover_hub", "sc_connect_timeout_ms",
                                                 "sc_control_rate_limit", "router",
                                                 "write_policy", "resolve_scope"])
        self.assertEqual(parameters["reconnect"].kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertIsNone(parameters["reconnect"].default)
        for transport in ["bip", "ipv6", "mstp", "unknown"]:
            with self.subTest(transport=transport), self.assertRaisesRegex(ValueError, "reconnect"):
                BACnetClient(transport=transport, reconnect=ScReconnect())
        BACnetClient(reconnect=None)
        with self.assertRaises(TypeError):
            BACnetClient(reconnect={})
