#!/usr/bin/env python3
"""Generate the ArcScan application icon set with no external dependencies.

Renders the ArcScan mark — a stylized gradient "A" crossed by a glowing cyan
comet swoosh with three trailing data-squares, on a dark rounded tile — at high
resolution (supersampled for anti-aliasing), then writes the PNG sizes, a
multi-image Windows ICO, and a macOS ICNS. Pure standard library: math + zlib +
struct only.

Run from the repo root:  python3 scripts/generate_icon.py
Outputs into src-tauri/icons/ and public/icon.png (used by the web/mock build).
"""
import math
import os
import struct
import zlib

OUT_DIR = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "icons")
PUBLIC_DIR = os.path.join(os.path.dirname(__file__), "..", "public")

# Palette (RGB 0-255)
BG_TOP = (16, 22, 34)
BG_BOTTOM = (5, 8, 14)
A_TOP = (74, 186, 255)      # bright azure (top of the A)
A_BOT = (28, 78, 214)       # deep blue (foot of the A)
COMET_TAIL = (54, 170, 226)
COMET_HEAD = (224, 246, 255)
GLOW = (56, 200, 255)
WHITE = (255, 255, 255)
WARM = (255, 120, 110)


def lerp(a, b, t):
    return a + (b - a) * t


def mix(c1, c2, t):
    t = max(0.0, min(1.0, t))
    return tuple(lerp(c1[i], c2[i], t) for i in range(3))


def over(dst, src, a):
    """Alpha-composite src (rgb) with alpha a over dst (rgba floats)."""
    a = max(0.0, min(1.0, a))
    da = dst[3]
    out_a = a + da * (1 - a)
    if out_a <= 0:
        return (0.0, 0.0, 0.0, 0.0)
    out_rgb = tuple((src[i] * a + dst[i] * da * (1 - a)) / out_a for i in range(3))
    return (out_rgb[0], out_rgb[1], out_rgb[2], out_a)


def point_in_tri(px, py, a, b, c):
    d1 = (px - b[0]) * (a[1] - b[1]) - (a[0] - b[0]) * (py - b[1])
    d2 = (px - c[0]) * (b[1] - c[1]) - (b[0] - c[0]) * (py - c[1])
    d3 = (px - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (py - a[1])
    has_neg = (d1 < 0) or (d2 < 0) or (d3 < 0)
    has_pos = (d1 > 0) or (d2 > 0) or (d3 > 0)
    return not (has_neg and has_pos)


def dist_to_seg(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    ll = dx * dx + dy * dy
    if ll <= 1e-9:
        return math.hypot(px - ax, py - ay), 0.0
    t = ((px - ax) * dx + (py - ay) * dy) / ll
    t = max(0.0, min(1.0, t))
    cx, cy = ax + t * dx, ay + t * dy
    return math.hypot(px - cx, py - cy), t


def bezier(p0, p1, p2, n):
    pts = []
    for i in range(n + 1):
        t = i / n
        mt = 1 - t
        x = mt * mt * p0[0] + 2 * mt * t * p1[0] + t * t * p2[0]
        y = mt * mt * p0[1] + 2 * mt * t * p1[1] + t * t * p2[1]
        pts.append((x, y))
    return pts


def render(size):
    ss = 4 if size >= 48 else 5
    S = size * ss
    buf = [[(0.0, 0.0, 0.0, 0.0) for _ in range(S)] for _ in range(S)]

    cx = cy = S / 2.0
    radius = S * 0.5
    corner = S * 0.235

    def rounded_alpha(x, y):
        hw = radius
        dx = abs(x - cx) - (hw - corner)
        dy = abs(y - cy) - (hw - corner)
        if dx < 0 and dy < 0:
            d = max(dx, dy)
        else:
            d = math.hypot(max(dx, 0.0), max(dy, 0.0)) - corner
        return max(0.0, min(1.0, 0.5 - d / (1.4 * ss)))

    # --- geometry in normalized [0,1] coords, scaled by S ---
    # Stylized "A": outer shell minus inner opening (open at the bottom).
    a_apex = (0.475 * S, 0.165 * S)
    a_out_l = (0.145 * S, 0.885 * S)
    a_out_r = (0.815 * S, 0.885 * S)
    i_apex = (0.475 * S, 0.375 * S)
    i_bot_l = (0.315 * S, 0.895 * S)
    i_bot_r = (0.645 * S, 0.895 * S)

    # Comet swoosh centerline (quadratic bezier) + head/orb.
    comet = bezier((0.265 * S, 0.775 * S), (0.55 * S, 0.5 * S), (0.805 * S, 0.395 * S), 90)
    orb = (0.805 * S, 0.395 * S)
    orb_r = 0.05 * S
    w_max = 0.06 * S  # comet half-thickness

    # Trailing data-squares (center, half-size, rotation).
    squares = [
        (0.70 * S, 0.30 * S, 0.022 * S, 0.30),
        (0.775 * S, 0.235 * S, 0.03 * S, 0.35),
        (0.85 * S, 0.165 * S, 0.04 * S, 0.4),
    ]

    def comet_profile(px, py):
        """Return (inside_dist, t, halfwidth) nearest point on comet stroke."""
        best = (1e9, 0.0)
        for i in range(len(comet) - 1):
            d, tt = dist_to_seg(px, py, comet[i][0], comet[i][1], comet[i + 1][0], comet[i + 1][1])
            seg_t = (i + tt) / (len(comet) - 1)
            if d < best[0]:
                best = (d, seg_t)
        d, t = best
        # taper: thin at tail, full mid, thin toward head
        hw = w_max * (math.sin(math.pi * min(1.0, t * 1.04)) ** 0.55)
        hw = max(hw, 0.006 * S)
        return d, t, hw

    def sq_alpha(px, py, sq):
        sx, sy, half, rot = sq
        ca, sa = math.cos(-rot), math.sin(-rot)
        dx, dy = px - sx, py - sy
        lx = dx * ca - dy * sa
        ly = dx * sa + dy * ca
        r = half * 0.32
        ax = abs(lx) - (half - r)
        ay = abs(ly) - (half - r)
        if ax < 0 and ay < 0:
            dd = max(ax, ay)
        else:
            dd = math.hypot(max(ax, 0.0), max(ay, 0.0)) - r
        return max(0.0, min(1.0, 0.5 - dd / (1.4 * ss)))

    for y in range(S):
        for x in range(S):
            xf, yf = x + 0.5, y + 0.5
            ra = rounded_alpha(xf, yf)
            if ra <= 0:
                continue

            # background gradient + radial glow near the orb
            t = yf / S
            bg = mix(BG_TOP, BG_BOTTOM, t)
            gd = math.hypot(xf - orb[0], yf - orb[1]) / (0.55 * S)
            bg = mix(bg, GLOW, max(0.0, 0.22 - gd * 0.22))
            px = (bg[0], bg[1], bg[2], ra)

            # --- the A glyph ---
            in_outer = point_in_tri(xf, yf, a_apex, a_out_l, a_out_r)
            in_inner = point_in_tri(xf, yf, i_apex, i_bot_l, i_bot_r)
            if in_outer and not in_inner:
                shade = mix(A_TOP, A_BOT, (yf / S - 0.16) / 0.73)
                # a touch of left-to-right brightening
                shade = mix(shade, WHITE, max(0.0, (xf / S - 0.5)) * 0.10)
                px = over(px, shade, 0.97 * ra)

            # comet distance profile
            cd, ct, chw = comet_profile(xf, yf)

            # dark shadow gap carved where the comet crosses the A
            gap = chw * 1.5
            if cd < gap:
                shadow_a = (1.0 - cd / gap) * 0.85
                px = over(px, (3, 5, 9), shadow_a * ra)

            # comet glow halo
            if cd < chw * 3.0:
                halo = max(0.0, 1.0 - cd / (chw * 3.0)) ** 2
                px = over(px, GLOW, halo * 0.5 * ra)

            # comet body
            if cd < chw:
                edge = min(1.0, (chw - cd) / (1.4 * ss))
                col = mix(COMET_TAIL, COMET_HEAD, ct ** 0.7)
                px = over(px, col, edge * ra)

            # trailing squares (glow then body)
            for sq in squares:
                sd = math.hypot(xf - sq[0], yf - sq[1])
                if sd < sq[2] * 3.5:
                    g = max(0.0, 1.0 - sd / (sq[2] * 3.5)) ** 2
                    px = over(px, GLOW, g * 0.35 * ra)
                sa = sq_alpha(xf, yf, sq)
                if sa > 0:
                    col = mix(GLOW, WHITE, 0.25)
                    px = over(px, col, sa * ra)

            # the orb: bright core + halo + tiny warm highlight
            od = math.hypot(xf - orb[0], yf - orb[1])
            if od < orb_r * 3.0:
                halo = max(0.0, 1.0 - od / (orb_r * 3.0)) ** 2
                px = over(px, GLOW, halo * 0.6 * ra)
            if od < orb_r:
                core = max(0.0, 1.0 - od / orb_r)
                px = over(px, mix(COMET_HEAD, WHITE, core), min(1.0, 0.7 + core) * ra)
            wd = math.hypot(xf - (orb[0] + orb_r * 0.25), yf - (orb[1] + orb_r * 0.1))
            if wd < orb_r * 0.35:
                px = over(px, WARM, (1.0 - wd / (orb_r * 0.35)) * 0.8 * ra)

            buf[y][x] = px

    # downsample ss x ss -> size
    out = bytearray(size * size * 4)
    n = ss * ss
    for y in range(size):
        for x in range(size):
            r = g = b = a = 0.0
            for j in range(ss):
                row = buf[y * ss + j]
                for i in range(ss):
                    p = row[x * ss + i]
                    r += p[0] * p[3]
                    g += p[1] * p[3]
                    b += p[2] * p[3]
                    a += p[3]
            if a > 0:
                r, g, b = r / a, g / a, b / a
            a /= n
            o = (y * size + x) * 4
            out[o] = int(max(0, min(255, r)))
            out[o + 1] = int(max(0, min(255, g)))
            out[o + 2] = int(max(0, min(255, b)))
            out[o + 3] = int(max(0, min(255, a * 255)))
    return bytes(out)


def write_png(path, size, rgba):
    def chunk(typ, data):
        c = struct.pack(">I", len(data)) + typ + data
        return c + struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF)

    raw = bytearray()
    for y in range(size):
        raw.append(0)
        raw += rgba[y * size * 4:(y + 1) * size * 4]
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", ihdr)
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def png_bytes(size, rgba):
    tmp = os.path.join(OUT_DIR, f".__tmp_{size}.png")
    write_png(tmp, size, rgba)
    with open(tmp, "rb") as f:
        data = f.read()
    os.remove(tmp)
    return data


def write_icns(path, entries):
    type_for_size = {
        16: [b"icp4"],
        32: [b"icp5", b"ic11"],
        64: [b"icp6", b"ic12"],
        128: [b"ic07"],
        256: [b"ic08", b"ic13"],
        512: [b"ic09", b"ic14"],
    }
    body = b""
    for size, data in entries:
        for typ in type_for_size.get(size, []):
            body += typ + struct.pack(">I", 8 + len(data)) + data
    with open(path, "wb") as f:
        f.write(b"icns" + struct.pack(">I", 8 + len(body)) + body)


def write_ico(path, entries):
    count = len(entries)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + count * 16
    dir_entries = b""
    image_data = b""
    for size, data in entries:
        w = 0 if size >= 256 else size
        h = 0 if size >= 256 else size
        dir_entries += struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
        image_data += data
    with open(path, "wb") as f:
        f.write(header + dir_entries + image_data)


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    os.makedirs(PUBLIC_DIR, exist_ok=True)

    sizes = [16, 24, 32, 48, 64, 128, 256, 512]
    rendered = {}
    for s in sizes:
        print(f"rendering {s}x{s} ...")
        rendered[s] = render(s)

    write_png(os.path.join(OUT_DIR, "32x32.png"), 32, rendered[32])
    write_png(os.path.join(OUT_DIR, "128x128.png"), 128, rendered[128])
    write_png(os.path.join(OUT_DIR, "128x128@2x.png"), 256, rendered[256])
    write_png(os.path.join(OUT_DIR, "icon.png"), 512, rendered[512])
    write_png(os.path.join(PUBLIC_DIR, "icon.png"), 256, rendered[256])

    ico_entries = [(s, png_bytes(s, rendered[s])) for s in [16, 24, 32, 48, 64, 128, 256]]
    write_ico(os.path.join(OUT_DIR, "icon.ico"), ico_entries)

    icns_entries = [(s, png_bytes(s, rendered[s])) for s in [16, 32, 64, 128, 256, 512]]
    write_icns(os.path.join(OUT_DIR, "icon.icns"), icns_entries)

    print("done -> icons/{32x32,128x128,128x128@2x,icon}.png, icon.ico, icon.icns")


if __name__ == "__main__":
    main()
