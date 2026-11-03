#!/usr/bin/env python3
"""The MCP bundle's icon (#2324): writes icon.svg and its 512x512 icon.png.

    python3 scripts/release/mcpb/icon.py

One list of shapes draws both, so the PNG is the SVG's export: a rounded
square holding a vertical bus with three floors, each a hub and two linked
nodes. It is original and geometric, with no product, vendor or protocol
mark. The PNG is rendered here with the standard library alone (4x4
supersampling per pixel), so the export is reproducible without an SVG
renderer.
"""

import math
import struct
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
SIZE = 512
BACKGROUND = "#17324a"
LINE = "#e8f1f5"
NODE = "#3fb8af"

FLOORS = (160, 256, 352)
# Drawn in order, each over the ones before: ("rect", x, y, w, h, radius,
# colour), ("line", x1, y1, x2, y2, width, colour) with round caps, and
# ("circle", cx, cy, r, colour).
SHAPES = [("rect", 0, 0, SIZE, SIZE, 104, BACKGROUND),
          ("line", 256, 112, 256, 400, 24, LINE)]
for y in FLOORS:
    SHAPES.append(("line", 136, y, 376, y, 16, LINE))
for y in FLOORS:
    SHAPES += [("circle", 136, y, 34, NODE), ("circle", 376, y, 34, NODE), ("circle", 256, y, 24, LINE)]


def svg():
    """The SVG source."""
    parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{SIZE}" height="{SIZE}" '
             f'viewBox="0 0 {SIZE} {SIZE}">']
    for shape in SHAPES:
        kind = shape[0]
        if kind == "rect":
            _, x, y, w, h, r, colour = shape
            parts.append(f'  <rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{colour}"/>')
        elif kind == "line":
            _, x1, y1, x2, y2, width, colour = shape
            parts.append(f'  <line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{colour}" '
                         f'stroke-width="{width}" stroke-linecap="round"/>')
        else:
            _, cx, cy, r, colour = shape
            parts.append(f'  <circle cx="{cx}" cy="{cy}" r="{r}" fill="{colour}"/>')
    parts.append("</svg>")
    return "\n".join(parts) + "\n"


def _inside(shape, px, py):
    kind = shape[0]
    if kind == "rect":
        _, x, y, w, h, r, _ = shape
        qx = max(abs(px - (x + w / 2)) - (w / 2 - r), 0.0)
        qy = max(abs(py - (y + h / 2)) - (h / 2 - r), 0.0)
        return qx * qx + qy * qy <= r * r
    if kind == "line":
        _, x1, y1, x2, y2, width, _ = shape
        dx, dy = x2 - x1, y2 - y1
        t = max(0.0, min(1.0, ((px - x1) * dx + (py - y1) * dy) / (dx * dx + dy * dy)))
        return math.hypot(px - (x1 + t * dx), py - (y1 + t * dy)) <= width / 2
    _, cx, cy, r, _ = shape
    return (px - cx) ** 2 + (py - cy) ** 2 <= r * r


def _rgb(colour):
    return tuple(int(colour[i:i + 2], 16) for i in (1, 3, 5))


def png(samples=4):
    """The 512x512 RGBA PNG of the same shapes."""
    offsets = [(i + 0.5) / samples for i in range(samples)]
    shapes = [(shape, _rgb(shape[-1])) for shape in SHAPES]
    rows = bytearray()
    for y in range(SIZE):
        rows.append(0)  # filter: none
        for x in range(SIZE):
            r = g = b = a = 0
            for oy in offsets:
                for ox in offsets:
                    px, py = x + ox, y + oy
                    hit = None
                    for shape, rgb in shapes:
                        if _inside(shape, px, py):
                            hit = rgb
                    if hit:
                        r, g, b, a = r + hit[0], g + hit[1], b + hit[2], a + 1
            if a:
                rows += bytes((round(r / a), round(g / a), round(b / a), round(255 * a / samples ** 2)))
            else:
                rows += b"\0\0\0\0"

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    header = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(bytes(rows), 9)) + chunk(b"IEND", b""))


if __name__ == "__main__":
    (HERE / "icon.svg").write_text(svg(), encoding="utf-8")
    (HERE / "icon.png").write_bytes(png())
    print(f"wrote {HERE / 'icon.svg'} and {HERE / 'icon.png'}")
