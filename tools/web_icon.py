"""Draw the website's icons: a purple 8 on black, in the site's colours
(the dark background and the accent of the web client), without image libraries.

    python3 tools/web_icon.py

writes into crates/arrus-web/static/: icon-180.png (iPhone home screen),
icon-192.png and icon-512.png (Android / "install app", full-bleed so they also
work as maskable icons), favicon-32.png and favicon.ico (tabs and bookmarks).
"""

import struct
import zlib
from pathlib import Path

BG = (0x0B, 0x0B, 0x0C)  # theme DEFAULT_BG
FG = (0x92, 0x66, 0xF8)  # theme DEFAULT_ACCENT
SS = 4  # supersampling per axis

# The 8: two rings, the lower one a little wider, overlapping by one stroke.
# Sizes as a share of the icon; it fits inside the maskable safe zone (a circle
# of 40 % radius around the centre).
STROKE = 0.085
R_TOP, R_BOTTOM = 0.135, 0.165


def rings(glyph: float) -> list[tuple[float, float, float, float]]:
    """(cx, cy, r_outer, r_inner) per ring, in units of the icon size."""
    rt, rb, w = R_TOP * glyph, R_BOTTOM * glyph, STROKE * glyph
    height = 2 * rt + 2 * rb - w
    top = 0.5 - height / 2
    return [(0.5, top + rt, rt, rt - w), (0.5, top + height - rb, rb, rb - w)]


def draw(size: int, glyph: float = 1.0) -> bytes:
    """RGB rows (PNG filter byte first)."""
    rs = [(cx * size, cy * size, ro * size, ri * size) for cx, cy, ro, ri in rings(glyph)]
    rows = []
    for y in range(size):
        row = bytearray([0])
        for x in range(size):
            hits = 0
            for sy in range(SS):
                for sx in range(SS):
                    px, py = x + (sx + 0.5) / SS, y + (sy + 0.5) / SS
                    for cx, cy, ro, ri in rs:
                        d2 = (px - cx) ** 2 + (py - cy) ** 2
                        if ri * ri <= d2 <= ro * ro:
                            hits += 1
                            break
            a = hits / (SS * SS)
            row += bytes(round(BG[i] + (FG[i] - BG[i]) * a) for i in range(3))
        rows.append(bytes(row))
    return b"".join(rows)


def png_bytes(size: int, data: bytes) -> bytes:
    def chunk(kind: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)  # opaque RGB
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(data, 9)) + chunk(b"IEND", b"")


def ico_bytes(pngs: list[tuple[int, bytes]]) -> bytes:
    """An .ico holding PNG images (every current browser reads these)."""
    head = struct.pack("<HHH", 0, 1, len(pngs))
    offset = 6 + 16 * len(pngs)
    entries, blobs = b"", b""
    for size, blob in pngs:
        entries += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(blob), offset + len(blobs))
        blobs += blob
    return head + entries + blobs


def main() -> None:
    out = Path(__file__).resolve().parent.parent / "crates" / "arrus-web" / "static"
    made = {}
    for size in (16, 32, 48, 180, 192, 512):
        # Tiny sizes: a bigger 8, the stroke stays visible.
        made[size] = png_bytes(size, draw(size, 1.35 if size <= 48 else 1.15))
    for size in (180, 192, 512):
        (out / f"icon-{size}.png").write_bytes(made[size])
    (out / "favicon-32.png").write_bytes(made[32])
    (out / "favicon.ico").write_bytes(ico_bytes([(16, made[16]), (32, made[32]), (48, made[48])]))


if __name__ == "__main__":
    main()
