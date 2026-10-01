"""Draw icon.ico: teal rounded square with a white sound wave (five rounded bars). Stdlib only.

    python desktop/icons/draw_icon.py
"""
import struct
import zlib
from pathlib import Path

TEAL, WHITE, CLEAR = (31, 111, 104, 255), (255, 255, 255, 255), (0, 0, 0, 0)
BARS = [(-0.26, 0.10), (-0.13, 0.22), (0.0, 0.30), (0.13, 0.18), (0.26, 0.08)]  # (x, half-height) / size


def pixel(x: float, y: float) -> tuple:
    """x, y in -0.5..0.5."""
    r, half = 0.22, 0.5  # corner radius, half side
    qx, qy = max(abs(x) - (half - r), 0), max(abs(y) - (half - r), 0)
    if qx * qx + qy * qy > r * r:
        return CLEAR
    for bx, bh in BARS:  # bars with round caps, width 0.07
        dx, dy = abs(x - bx), max(abs(y) - bh, 0)
        if dx * dx + dy * dy < 0.035 ** 2:
            return WHITE
    return TEAL


def png(size: int) -> bytes:
    raw = b"".join(
        b"\0" + b"".join(bytes(pixel((i + 0.5) / size - 0.5, (j + 0.5) / size - 0.5)) for i in range(size))
        for j in range(size))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


sizes = [16, 24, 32, 48, 256]
images = [png(s) for s in sizes]
offset = 6 + 16 * len(sizes)
directory = b""
for s, img in zip(sizes, images):
    directory += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(img), offset)
    offset += len(img)
out = Path(__file__).with_name("icon.ico")
out.write_bytes(struct.pack("<HHH", 0, 1, len(sizes)) + directory + b"".join(images))
print("wrote", out)
