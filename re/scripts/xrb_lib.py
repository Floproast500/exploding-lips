"""Minimal XRB section locator following the loader order (ReadCompiledOrb).

File order after the 300-byte descriptor table:
  Texs (+ per-tex data), Pics, Points, Paths, Regions, Walls, Objects, ObjectStates,
  Globals6x64 (0x180), ViewTable (0x800), Views, Auxs, Sounds,
  Win60 (hdr[0x40]*0x3c), Texts (+ per-text data), Windows, Overlays, ActionNodes,
  Trailer (hdr[0x24] bytes), Tail (0x7c bytes).
"""
import struct

SECTIONS = ["Texs", "Pics", "Points", "Paths", "Regions", "Walls", "Objects", "ObjectStates",
            "Views", "Auxs", "Sounds", "Texts", "Windows", "Overlays", "ActionNodes"]


class Xrb:
    def __init__(self, path):
        self.path = path
        self.data = open(path, "rb").read()
        self.hdr = self.data[:0x68]
        self.body = self.data[0x68:]
        b = self.body
        self.desc = {n: struct.unpack_from("<5I", b, i * 20) for i, n in enumerate(SECTIONS)}
        self.off = {}
        off = 300

        def sec(name):
            nonlocal off
            self.off[name] = off
            off += self.desc[name][2] * self.desc[name][3]
        sec("Texs")
        self.off["TexData"] = off
        for k in range(self.desc["Texs"][1]):
            off += struct.unpack_from("<I", b, self.off["Texs"] + k * 100 + 0x50)[0]
        for n in ["Pics", "Points", "Paths", "Regions", "Walls", "Objects", "ObjectStates"]:
            sec(n)
        self.off["Globals6x64"] = off; off += 0x180
        self.off["ViewTable"] = off; off += 0x800
        for n in ["Views", "Auxs", "Sounds"]:
            sec(n)
        self.off["Win60"] = off; off += self.h32(0x40) * 0x3c
        sec("Texts")
        self.off["TextData"] = off
        for k in range(self.desc["Texts"][1]):
            r = self.off["Texts"] + k * 108
            if struct.unpack_from("<i", b, r + 0x3c)[0] > 0:
                off += struct.unpack_from("<i", b, r + 0x50)[0] * 4 + struct.unpack_from("<i", b, r + 0x40)[0]
        for n in ["Windows", "Overlays", "ActionNodes"]:
            sec(n)
        self.off["Trailer"] = off; off += self.h32(0x24)
        self.off["Tail"] = off; off += 0x7c
        self.end = off

    def h32(self, o):
        return struct.unpack_from("<i", self.hdr, o)[0]

    def count(self, name):
        return self.desc[name][1]

    def rec(self, name, i):
        rs = self.desc[name][3]
        o = self.off[name] + i * rs
        return self.body[o:o + rs]

    def recs(self, name):
        return [self.rec(name, i) for i in range(self.count(name))]


def i32(r, o): return struct.unpack_from("<i", r, o)[0]
def u32(r, o): return struct.unpack_from("<I", r, o)[0]
def i16(r, o): return struct.unpack_from("<h", r, o)[0]
def u16(r, o): return struct.unpack_from("<H", r, o)[0]
def fx(r, o): return i32(r, o) / 65536.0
def name(r, o=4): return r[o:o + 8].split(b"\0")[0].decode("latin1")
