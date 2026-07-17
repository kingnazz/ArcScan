#!/usr/bin/env python3
"""Generate the ArcScan application icon set with no external dependencies.

Renders a premium dark "radar arc" mark at high resolution (supersampled for
anti-aliasing), then writes the PNG sizes and a multi-image ICO that Tauri
bundles for Windows. Pure standard library: math + zlib + struct only.

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
BG_TOP = (14, 20, 32)
BG_BOTTOM = (8, 11, 18)
BRAND = (56, 212, 240)      # cyan arc
BRAND_DEEP = (10, 152, 184)
BLIP = (150, 240, 255)
WHITE = (255, 255, 255)


def lerp(a, b, t):
    return a + (b - a) * t


def mix(c1, c2, t):
    return tuple(lerp(c1[i], c2[i], t) for i in range(3))


def over(dst, src, a):
    """Alpha-composite src (rgb) with alpha a over dst (rgba floats)."""
    da = dst[3]
    out_a = a + da * (1 - a)
    if out_a <= 0:
        return (0.0, 0.0, 0.0, 0.0)
    out_rgb = tuple(
        (src[i] * a + dst[i] * da * (1 - a)) / out_a for i in range(3)
    )
    return (out_rgb[0], out_rgb[1], out_rgb[2], out_a)


def render(size):
    """Render the icon at the given pixel size, supersampled 4x."""
    ss = 4
    S = size * ss
    # buffer of (r,g,b,a) floats, premultiplied-free
    buf = [[(0.0, 0.0, 0.0, 0.0) for _ in range(S)] for _ in range(S)]

    cx = cy = S / 2.0
    radius = S * 0.5
    corner = S * 0.235  # rounded-square radius

    def rounded_alpha(x, y):
        # signed distance to rounded rect covering the whole canvas
        hw = radius
        dx = abs(x - cx) - (hw - corner)
        dy = abs(y - cy) - (hw - corner)
        if dx < 0 and dy < 0:
            d = max(dx, dy)
        else:
            d = math.hypot(max(dx, 0.0), max(dy, 0.0)) - corner
        # antialias over ~1.5 supersampled px
        return max(0.0, min(1.0, 0.5 - d / (1.5 * ss)))

    # radar geometry: origin near lower-left of the tile
    ox, oy = cx - S * 0.16, cy + S * 0.17
    rings = [0.30, 0.46, 0.62, 0.78]  # as fraction of S
    sweep_angle = math.radians(-38)   # pointing up-right
    sweep_width = math.radians(30)

    for y in range(S):
        for x in range(S):
            ra = rounded_alpha(x + 0.5, y + 0.5)
            if ra <= 0:
                continue
            # background gradient
            t = y / S
            bg = mix(BG_TOP, BG_BOTTOM, t)
            # subtle radial vignette lift toward center
            dist_c = math.hypot(x - cx, y - cy) / radius
            bg = mix(bg, (20, 30, 46), max(0.0, 0.35 - dist_c * 0.35))
            px = (bg[0], bg[1], bg[2], ra)

            dxo = x - ox
            dyo = y - oy
            dist = math.hypot(dxo, dyo)
            ang = math.atan2(-(dyo), dxo)  # screen y is down

            # concentric arc rings (quarter/three-quarter sweep)
            for rr in rings:
                rad = rr * S * 0.5
                edge = abs(dist - rad)
                line_w = 1.4 * ss
                if edge < line_w and dxo > -S * 0.02 and dyo < S * 0.02:
                    aa = 1.0 - edge / line_w
                    fade = 0.32 + 0.10 * (1 - rr)
                    px = over(px, BRAND_DEEP, aa * fade * ra)

            # sweep wedge (radar beam)
            da = ((sweep_angle - ang + math.pi) % (2 * math.pi)) - math.pi
            if -sweep_width < da < sweep_width * 0.35 and dist < S * 0.40:
                # brighter at leading edge
                lead = max(0.0, 1.0 - (sweep_angle - ang) / sweep_width) if da <= 0 else 0.4
                intensity = (0.55 * lead) * (1.0 - dist / (S * 0.42))
                if intensity > 0:
                    col = mix(BRAND_DEEP, BRAND, lead)
                    px = over(px, col, min(0.9, intensity) * ra)

            buf[y][x] = px

    # blips (detected hosts) as glowing dots
    blips = [(0.34, 0.30, 0.020), (0.60, 0.52, 0.016), (0.70, 0.30, 0.013)]
    for bx, by, br in blips:
        bxp, byp = bx * S, by * S
        brp = br * S
        for y in range(max(0, int(byp - brp * 4)), min(S, int(byp + brp * 4))):
            for x in range(max(0, int(bxp - brp * 4)), min(S, int(bxp + brp * 4))):
                d = math.hypot(x - bxp, y - byp)
                if d < brp * 4:
                    core = max(0.0, 1.0 - d / brp)
                    halo = max(0.0, 1.0 - d / (brp * 4)) ** 2
                    a = min(1.0, core * 1.0 + halo * 0.45)
                    if a > 0:
                        ra = buf[y][x][3]
                        if ra > 0:
                            col = mix(BRAND, WHITE, core * 0.6)
                            buf[y][x] = over(buf[y][x], col, a * ra)

    # downsample ss x ss -> size
    out = bytearray(size * size * 4)
    for y in range(size):
        for x in range(size):
            r = g = b = a = 0.0
            for j in range(ss):
                for i in range(ss):
                    px = buf[y * ss + j][x * ss + i]
                    r += px[0] * px[3]
                    g += px[1] * px[3]
                    b += px[2] * px[3]
                    a += px[3]
            n = ss * ss
            if a > 0:
                r, g, b = r / a, g / a, b / a
            a = a / n
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
        raw.append(0)  # filter type 0
        raw += rgba[y * size * 4:(y + 1) * size * 4]
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", ihdr)
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def png_bytes(size, rgba):
    import io
    tmp = os.path.join(OUT_DIR, f".__tmp_{size}.png")
    write_png(tmp, size, rgba)
    with open(tmp, "rb") as f:
        data = f.read()
    os.remove(tmp)
    return data


def write_icns(path, entries):
    """entries: list of (size, png_bytes). Writes a modern PNG-based .icns.

    Maps rendered sizes onto the icns element types macOS expects, including
    the @2x retina variants that share pixel data with the larger base sizes.
    """
    type_for_size = {
        16: [b"icp4"],
        32: [b"icp5", b"ic11"],   # 32 base + 16@2x
        64: [b"icp6", b"ic12"],   # 64 base + 32@2x
        128: [b"ic07"],
        256: [b"ic08", b"ic13"],  # 256 base + 128@2x
        512: [b"ic09", b"ic14"],  # 512 base + 256@2x
    }
    body = b""
    for size, data in entries:
        for typ in type_for_size.get(size, []):
            body += typ + struct.pack(">I", 8 + len(data)) + data
    with open(path, "wb") as f:
        f.write(b"icns" + struct.pack(">I", 8 + len(body)) + body)


def write_ico(path, entries):
    """entries: list of (size, png_bytes). Stored as PNG-in-ICO (Vista+)."""
    count = len(entries)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + count * 16
    dir_entries = b""
    image_data = b""
    for size, data in entries:
        w = 0 if size >= 256 else size
        h = 0 if size >= 256 else size
        dir_entries += struct.pack(
            "<BBBBHHII", w, h, 0, 0, 1, 32, len(data), offset
        )
        offset += len(data)
        image_data += data
    with open(path, "wb") as f:
        f.write(header + dir_entries + image_data)


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    os.makedirs(PUBLIC_DIR, exist_ok=True)

    # Master render at 512, plus dedicated renders for small sizes so the
    # arc geometry stays crisp.
    sizes = [16, 24, 32, 48, 64, 128, 256, 512]
    rendered = {}
    for s in sizes:
        print(f"rendering {s}x{s} ...")
        rendered[s] = render(s)

    # PNG outputs Tauri references
    write_png(os.path.join(OUT_DIR, "32x32.png"), 32, rendered[32])
    write_png(os.path.join(OUT_DIR, "128x128.png"), 128, rendered[128])
    write_png(os.path.join(OUT_DIR, "128x128@2x.png"), 256, rendered[256])
    write_png(os.path.join(OUT_DIR, "icon.png"), 512, rendered[512])
    # web/mock build favicon
    write_png(os.path.join(PUBLIC_DIR, "icon.png"), 256, rendered[256])

    # Windows ICO with the common sizes
    ico_entries = [(s, png_bytes(s, rendered[s])) for s in [16, 24, 32, 48, 64, 128, 256]]
    write_ico(os.path.join(OUT_DIR, "icon.ico"), ico_entries)

    # macOS ICNS (PNG-based, with retina variants)
    icns_entries = [(s, png_bytes(s, rendered[s])) for s in [16, 32, 64, 128, 256, 512]]
    write_icns(os.path.join(OUT_DIR, "icon.icns"), icns_entries)

    print("done -> src-tauri/icons/{32x32,128x128,128x128@2x,icon}.png, icon.ico, icon.icns")


if __name__ == "__main__":
    main()
