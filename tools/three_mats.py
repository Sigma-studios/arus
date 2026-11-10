# Draws crates/arrus-web/assets/three-mats.png (standard library only):
#   python3 tools/three_mats.py crates/arrus-web/assets/three-mats.png
# Three cheer mats running left to right (horizontal strips), stacked back to front.
import sys, zlib, struct
W, H, N = 1200, 600, 3
MAT = (0x4A, 0x4A, 0x50)
DARK = (0x42, 0x42, 0x48)
SEAM = (0x6C, 0x6C, 0x74)
def px(x, y):
    i = min(y * N // H, N - 1)
    top = i * H // N
    bot = (i + 1) * H // N
    if y - top < 2 or bot - 1 - y < 2 or x < 3 or x > W - 4:
        return SEAM
    return DARK if i % 2 else MAT
raw = b"".join(b"\0" + bytes(c for x in range(W) for c in px(x, y)) for y in range(H))
def chunk(t, d):
    return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0)) \
    + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
