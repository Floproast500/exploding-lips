"""Render every loaded pic of a level onto one contact-sheet PNG using PALETTE.PAL."""
import struct, sys, zlib, os

def write_png(path, w, h, rgb):
    raw = b"".join(b"\0" + rgb[y * w * 3:(y + 1) * w * 3] for y in range(h))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))

def main(xrb, pal_path, out):
    pal6 = open(pal_path, "rb").read()[8:8 + 768]
    pal = bytes(c * 255 // 63 for c in pal6)
    d = open(xrb, "rb").read()[0x68:]
    xpg = open(xrb[:-4] + ".XPG", "rb").read()
    descs = [struct.unpack_from("<5I", d, i * 20) for i in range(15)]
    off = 300 + descs[0][2] * 100
    for i in range(descs[0][1]):
        off += struct.unpack_from("<I", d, 300 + i * 100 + 0x50)[0]
    pics = []
    for i in range(descs[1][1]):
        p = d[off + i * 40: off + (i + 1) * 40]
        w, h = struct.unpack_from("<HH", p, 0x0e)
        x = struct.unpack_from("<i", p, 0x24)[0]
        if x >= 0 and 0 < w <= 512 and 0 < h <= 512:
            pics.append((w, h, xpg[x:x + w * h]))
    cell, cols = 66, 24
    rows = (len(pics) + cols - 1) // cols
    W, H = cols * cell, rows * cell
    img = bytearray(W * H * 3)
    for n, (w, h, px) in enumerate(pics):
        ox, oy = (n % cols) * cell, (n // cols) * cell
        sx, sy = max(1, (w + 63) // 64), max(1, (h + 63) // 64)
        for y in range(0, min(h, 64 * sy), sy):
            for x in range(0, min(w, 64 * sx), sx):
                c = px[y * w + x]
                o = ((oy + y // sy) * W + ox + x // sx) * 3
                img[o:o + 3] = pal[c * 3:c * 3 + 3]
    write_png(out, W, H, bytes(img))
    print(out, len(pics), "pics")

main(*sys.argv[1:4])
