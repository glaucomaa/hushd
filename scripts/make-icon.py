#!/usr/bin/env python3
import math
import struct
import zlib
from pathlib import Path

SIZE = 1024


def rounded_rect(x, y, left, top, right, bottom, radius):
    cx = max(left + radius, min(x, right - radius))
    cy = max(top + radius, min(y, bottom - radius))
    return radius - math.hypot(x - cx, y - cy)


def coverage(distance):
    return max(0.0, min(1.0, distance + 0.5))


def pixel(x, y):
    background = rounded_rect(x, y, 32, 32, 992, 992, 224)
    if background <= 0:
        return 0, 0, 0, 0
    shade = int(48 - 25 * y / SIZE)
    color = [shade + 4, shade + 5, shade + 6]
    alpha = coverage(background)

    distance = math.hypot(x - 512, y - 540)
    band = min(distance - 242, 310 - distance)
    if y <= 570 and band > -1:
        amount = coverage(band)
        color = [int(c * (1 - amount) + 242 * amount) for c in color]

    for left, right in ((224, 358), (666, 800)):
        cup = rounded_rect(x, y, left, 500, right, 800, 58)
        if cup > -1:
            amount = coverage(cup)
            color = [int(c * (1 - amount) + 242 * amount) for c in color]
        inset = rounded_rect(x, y, left + 34, 566, right - 34, 734, 28)
        if inset > -1:
            amount = coverage(inset)
            color = [int(c * (1 - amount) + 38 * amount) for c in color]

    return *color, int(255 * alpha)


def png_bytes(width, height, pixels):
    raw = b"".join(b"\0" + bytes(pixels[y * width * 4:(y + 1) * width * 4]) for y in range(height))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def downsample(source, source_size, size):
    ratio = source_size // size
    result = bytearray(size * size * 4)
    for y in range(size):
        for x in range(size):
            sums = [0, 0, 0, 0]
            for dy in range(ratio):
                for dx in range(ratio):
                    offset = ((y * ratio + dy) * source_size + x * ratio + dx) * 4
                    for channel in range(4):
                        sums[channel] += source[offset + channel]
            offset = (y * size + x) * 4
            count = ratio * ratio
            result[offset:offset + 4] = bytes(round(value / count) for value in sums)
    return result


def main():
    root = Path(__file__).resolve().parent.parent
    target = root / "target"
    target.mkdir(exist_ok=True)
    pixels = bytearray(SIZE * SIZE * 4)
    for y in range(SIZE):
        for x in range(SIZE):
            offset = (y * SIZE + x) * 4
            pixels[offset:offset + 4] = bytes(pixel(x + 0.5, y + 0.5))
    (target / "m4-icon.png").write_bytes(png_bytes(SIZE, SIZE, pixels))

    iconset = target / "m4.iconset"
    iconset.mkdir(exist_ok=True)
    for size in (16, 32, 64, 128, 256, 512, 1024):
        resized = pixels if size == SIZE else downsample(pixels, SIZE, size)
        data = png_bytes(size, size, resized)
        (iconset / f"icon_{size}x{size}.png").write_bytes(data)
        if size <= 512:
            (iconset / f"icon_{size // 2}x{size // 2}@2x.png").write_bytes(data)


if __name__ == "__main__":
    main()
