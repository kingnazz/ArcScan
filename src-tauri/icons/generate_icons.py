#!/usr/bin/env python3
"""Generate ArcScan app icons (PNG + ICO) with no third-party dependencies.

Draws a radar motif on a dark rounded-square background to match the app's
premium dark theme, then writes the icon sizes referenced by tauri.conf.json.
Run from anywhere:  python3 src-tauri/icons/generate_icons.py
"""
import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))

ACCENT = (59, 130, 246)   # #3b82f6
ACCENT_SOFT = (96, 165, 250)
OK = (34, 197, 94)        # #22c55e
BG_TOP = (13, 16, 22)
BG_BOT = (20, 25, 33)


def lerp(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))


def over(dst, src, alpha):
    """Alpha-composite src (rgb) over dst (rgba) with the given 0..1 alpha."""
    dr, dg, db, da = dst
    out_a = alpha + da / 255 * (1 - alpha)
    if out_a <= 0:
        return (0, 0, 0, 0)
    r = (src[0] * alpha + dr * (da / 255) * (1 - alpha)) / out_a
    g = (src[1] * alpha + dg * (da / 255) * (1 - alpha)) / out_a
    b = (src[2] * alpha + db * (da / 255) * (1 - alpha)) / out_a
    return (int(round(r)), int(round(g)), int(round(b)), int(round(out_a * 255)))


def smooth(edge0, edge1, x):
    if edge0 == edge1:
        return 0.0 if x < edge0 else 1.0
    t = max(0.0, min(1.0, (x - edge0) / (edge1 - edge0)))
    return t * t * (3 - 2 * t)


def render(size):
    px = [[(0, 0, 0, 0) for _ in range(size)] for _ in range(size)]
    cx = cy = (size - 1) / 2.0
    radius = size * 0.22  # corner radius
    maxr = size * 0.40

    for y in range(size):
        for x in range(size):
            # Rounded-rect mask (signed distance to the rounded square).
            dx = abs(x - cx) - (size / 2 - radius)
            dy = abs(y - cy) - (size / 2 - radius)
            dist_corner = math.hypot(max(dx, 0), max(dy, 0)) - radius
            inside = min(max(dx, dy), 0) + dist_corner
            mask = 1.0 - smooth(-1.0, 1.0, inside)
            if mask <= 0:
                continue

            # Background vertical gradient.
            col = lerp(BG_TOP, BG_BOT, y / size)
            cell = (col[0], col[1], col[2], int(round(255 * mask)))

            # Radar geometry relative to centre.
            ddx = x - cx
            ddy = y - cy
            d = math.hypot(ddx, ddy)
            ang = math.atan2(ddy, ddx)  # -pi..pi

            # Concentric rings.
            for k, rr in enumerate((0.45, 0.72, 1.0)):
                ring_r = maxr * rr
                rw = size * 0.012 + 0.6
                edge = abs(d - ring_r)
                if edge < rw + 1:
                    a = (1.0 - smooth(rw - 0.5, rw + 0.8, edge)) * (0.55 - k * 0.1)
                    cell = over(cell, ACCENT, a * mask)

            # Sweep wedge (a soft fan trailing the leading edge).
            lead = -math.pi / 3  # leading edge angle
            rel = (lead - ang) % (2 * math.pi)
            span = math.pi * 0.42
            if d <= maxr * 1.02 and rel < span:
                fade = (1.0 - rel / span) * (1.0 - smooth(maxr * 0.9, maxr * 1.05, d))
                cell = over(cell, ACCENT_SOFT, 0.30 * fade * mask)
            # Bright leading edge line.
            if d <= maxr * 1.02:
                line = 1.0 - smooth(0.0, 0.06, abs(rel))
                cell = over(cell, ACCENT_SOFT, 0.9 * line * mask)

            # Centre hub.
            hub = size * 0.045
            if d < hub + 1.5:
                a = 1.0 - smooth(hub - 0.5, hub + 1.2, d)
                cell = over(cell, ACCENT_SOFT, a * mask)

            # A couple of detected "blips".
            for (bx, by, bcol) in (
                (maxr * 0.55, -maxr * 0.30, OK),
                (-maxr * 0.42, maxr * 0.50, ACCENT_SOFT),
            ):
                bd = math.hypot(ddx - bx, ddy - by)
                br = max(size * 0.018, 1.2)
                if bd < br + 2:
                    a = 1.0 - smooth(br - 0.4, br + 1.5, bd)
                    cell = over(cell, bcol, a * mask)

            px[y][x] = cell
    return px


def write_png(px, path):
    size = len(px)
    raw = bytearray()
    for row in px:
        raw.append(0)  # filter type 0
        for (r, g, b, a) in row:
            raw += bytes((r, g, b, a))
    comp = zlib.compress(bytes(raw), 9)

    def chunk(tag, data):
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", ihdr)
           + chunk(b"IDAT", comp)
           + chunk(b"IEND", b""))
    with open(path, "wb") as f:
        f.write(png)
    return png


def png_bytes(px):
    import io
    buf = io.BytesIO()
    size = len(px)
    raw = bytearray()
    for row in px:
        raw.append(0)
        for (r, g, b, a) in row:
            raw += bytes((r, g, b, a))
    comp = zlib.compress(bytes(raw), 9)

    def chunk(tag, data):
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    buf.write(b"\x89PNG\r\n\x1a\n")
    buf.write(chunk(b"IHDR", ihdr))
    buf.write(chunk(b"IDAT", comp))
    buf.write(chunk(b"IEND", b""))
    return buf.getvalue()


def write_ico(sizes, path):
    images = [(s, png_bytes(render(s))) for s in sizes]
    count = len(images)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    entries = b""
    blob = b""
    for (s, data) in images:
        w = 0 if s >= 256 else s
        h = 0 if s >= 256 else s
        entries += struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(data), offset)
        blob += data
        offset += len(data)
    with open(path, "wb") as f:
        f.write(header + entries + blob)


def main():
    cache = {}

    def get(size):
        if size not in cache:
            cache[size] = render(size)
        return cache[size]

    targets = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
        # Windows Store / installer extras (handy for `tauri build`).
        "Square150x150Logo.png": 150,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }
    for name, size in targets.items():
        write_png(get(size), os.path.join(HERE, name))
        print("wrote", name)

    write_ico([16, 32, 48, 64, 128, 256], os.path.join(HERE, "icon.ico"))
    print("wrote icon.ico")


if __name__ == "__main__":
    main()
