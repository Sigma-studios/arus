"""Draw the arus app icons (the site's icon, crates/arrus-web/static/icon-512.png,
at any size) without image libraries.

    python3 tools/app_icon.py

writes app/assets/icon.png (1024 px, opaque: iOS, macOS, Windows, Linux) and
app/assets/icon-foreground.png (transparent, for Android's adaptive icon, whose
background is the same dark colour). Then, in app/:
`dart run flutter_launcher_icons`.
"""

import struct
import zlib
from pathlib import Path

BG = (14, 16, 21)
RING = (245, 246, 248)
DOT = (240, 62, 116)
# Radii as a share of the size, measured on icon-512.png.
RING_OUT, RING_IN, DOT_R = 159 / 512, 148 / 512, 138 / 512
SS = 4  # supersampling per axis


def draw(size: int, scale: float, opaque: bool) -> bytes:
    """RGBA rows; `scale` shrinks the circles (1 = as on the site)."""
    c = size / 2
    r_out, r_in, r_dot = (v * size * scale for v in (RING_OUT, RING_IN, DOT_R))
    rows = []
    for y in range(size):
        row = bytearray([0])  # filter: none
        for x in range(size):
            acc = [0.0, 0.0, 0.0, 0.0]
            for sy in range(SS):
                for sx in range(SS):
                    dx = x + (sx + 0.5) / SS - c
                    dy = y + (sy + 0.5) / SS - c
                    d = (dx * dx + dy * dy) ** 0.5
                    if d <= r_dot:
                        col, a = DOT, 1.0
                    elif r_in <= d <= r_out:
                        col, a = RING, 1.0
                    elif opaque:
                        col, a = BG, 1.0
                    else:
                        col, a = (0, 0, 0), 0.0
                    for i in range(3):
                        acc[i] += col[i] * a
                    acc[3] += a
            n = SS * SS
            alpha = acc[3] / n
            if alpha > 0:
                rgb = [round(acc[i] / acc[3]) for i in range(3)]
            else:
                rgb = [0, 0, 0]
            row += bytes(rgb + [round(alpha * 255)])
        rows.append(bytes(row))
    return b"".join(rows)


def png(path: Path, size: int, data: bytes, opaque: bool) -> None:
    if opaque:  # drop alpha: the App Store refuses icons with an alpha channel
        out = bytearray()
        stride = 1 + size * 4
        for y in range(size):
            line = data[y * stride : (y + 1) * stride]
            out.append(0)
            for x in range(size):
                out += line[1 + x * 4 : 1 + x * 4 + 3]
        data, color_type = bytes(out), 2
    else:
        color_type = 6

    def chunk(kind: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    ihdr = struct.pack(">IIBBBBB", size, size, 8, color_type, 0, 0, 0)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(data, 9)) + chunk(b"IEND", b""))


def main() -> None:
    out = Path(__file__).resolve().parent.parent / "app" / "assets"
    out.mkdir(exist_ok=True)
    size = 1024
    png(out / "icon.png", size, draw(size, 1.0, True), True)
    # Android shows the middle ~61 % of an adaptive icon's foreground: keep the
    # ring well inside it.
    png(out / "icon-foreground.png", size, draw(size, 0.85, False), False)


if __name__ == "__main__":
    main()
