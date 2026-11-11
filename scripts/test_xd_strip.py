#!/usr/bin/env python3
"""Unit tests for xd_strip.py's property-states choice (#2472): python3 -m unittest discover -s scripts -p 'test_xd_strip.py'"""

import unittest
import xml.etree.ElementTree as ET

import xd_strip

NS = xd_strip.NS


def defs(alternatives, extra=""):
    body = "".join(alternatives)
    xml = (
        f'<Definitions xmlns="{NS[1:-1]}"><Choice name="0-BACnetPropertyStates" comment="prose">'
        f"<Choices>{body}</Choices></Choice>{extra}</Definitions>"
    )
    return ET.fromstring(xml)


class ChoiceTests(unittest.TestCase):
    def test_keeps_tag_name_and_datatype_only(self):
        root = defs(
            [
                '<Boolean name="boolean-value" contextTag="0" comment="more prose"/>',
                '<Enumerated name="sc-connection-state" contextTag="258" type="0-BACnetSCConnectionState"/>',
            ]
        )
        self.assertEqual(
            xd_strip.collect_choice(root),
            [(0, "boolean-value", "Boolean"), (258, "sc-connection-state", "Enumerated:0-BACnetSCConnectionState")],
        )

    def test_unknown_attribute_stops_the_script(self):
        root = defs(['<Boolean name="boolean-value" contextTag="0" surprise="x"/>'])
        with self.assertRaises(SystemExit):
            xd_strip.collect_choice(root)

    def test_repeated_tag_stops_the_script(self):
        root = defs(['<Boolean name="a" contextTag="0"/>', '<Boolean name="b" contextTag="0"/>'])
        with self.assertRaises(SystemExit):
            xd_strip.collect_choice(root)


if __name__ == "__main__":
    unittest.main()
