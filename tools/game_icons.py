"""Writes the game's own images the replay viewer (viewer/) draws, from your Civilization VI installation, to
viewer/public/data/icons/ (the images are the game's: they stay out of git).

  game_icons.py [--game <the game's folder>] [--workshop <the workshop's folder>]

It writes the yield icons drawn on the map (yields.png) and, as <ICON_NAME>.png of 64 pixels, the icons of the
districts, improvements, resources, units, buildings (wonders too) and projects, with icons.json listing them.

Two kinds of files hold the images.

A texture of its own is a file in BLPs/SHARED_DATA: a 0x30-byte header (at 0x20: 1, mip count, DXGI format,
width, height) and the mips, largest first, at the end of the file.

The icon atlases (the IconTextureAtlases of the game's Icons_*.xml) are in BLPs/UI/Icons.blp of the game, of each
DLC and of mods. Its header gives, at 0x10, where the data of its textures starts. Its index names each atlas
(a string: u32 capacity, u32 length, the characters) and keys records by the FNV-1a hash of the name:
- a texture: u64 offset, u64 size, u32 hash, and 0x28 bytes after the hash u16 DXGI format, u16 width, u16 height
  (the pages an atlas can be cut from, Page_0, Page_1, ...);
- an atlas on a page: u32 hash, 0, u32 flags, u32 page, u16 x, y, width, height, u32 hash of the lookup;
- an atlas of its own: the same with a page number past the pages, then u32 tile offset, u32 index offset (both in
  4-byte units), u16 tile side, u16 bytes per index, and its data where a record of u64 offset, u64 size precedes the
  hash: tiles of side x side RGBA pixels, each stored once, and an index of the tile at each place, row by row.
The pages read are RGBA (DXGI 28), largest mip first, row by row.
"""
import argparse
import io
import json
import re
import struct
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

import numpy as np
from PIL import Image

import civ6opt

OUT = civ6opt.ROOT / "viewer" / "public" / "data" / "icons"

# DXGI_FORMAT -> the DDS four-character code and bytes per 4x4 block
FORMATS = {71: (b"DXT1", 8), 72: (b"DXT1", 8), 77: (b"DXT5", 16), 78: (b"DXT5", 16)}

# the icons the viewer draws (by prefix): the map's and what cities produce
WANTED = ("ICON_DISTRICT_", "ICON_IMPROVEMENT_", "ICON_RESOURCE_", "ICON_UNIT_", "ICON_BUILDING_", "ICON_PROJECT_")
SIZE = 64


def steam_dir():
    if sys.platform == "win32":
        return Path(r"C:\Program Files (x86)\Steam")
    if sys.platform == "darwin":
        return Path.home() / "Library/Application Support/Steam"
    return Path("/mnt/c/Program Files (x86)/Steam")


def texture(path):
    """A texture of its own (BLPs/SHARED_DATA)."""
    b = path.read_bytes()
    if b[:6] != b"CIVBIG":
        sys.exit(f"{path}: not a texture of the game")
    _, mips, fmt, w, h = struct.unpack_from("<HHHHH", b, 0x20)
    if fmt not in FORMATS:
        sys.exit(f"{path}: texture format {fmt} is not read")
    code, block = FORMATS[fmt]
    sizes = [max(1, (w >> i) // 4) * max(1, (h >> i) // 4) * block for i in range(mips)]
    data = b[len(b) - sum(sizes):][:sizes[0]]
    pixel_format = struct.pack("<II4s5I", 32, 4, code, 0, 0, 0, 0, 0)
    header = struct.pack("<7I", 124, 0x1007, h, w, len(data), 0, 1) + bytes(44) + pixel_format + struct.pack("<5I", 0x1000, 0, 0, 0, 0)
    return Image.open(io.BytesIO(b"DDS " + header + data))


def fnv1a(s):
    h = 0x811C9DC5
    for c in s.encode():
        h = ((h ^ c) * 0x01000193) & 0xFFFFFFFF
    return h


class Package:
    """The atlases of one Icons.blp."""

    def __init__(self, path):
        self.f = path.open("rb")
        head = self.f.read(0x20)
        if head[:6] != b"CIVBLP":
            sys.exit(f"{path}: not a package of the game")
        self.base = struct.unpack_from("<I", head, 0x10)[0]
        self.index = self.f.read(self.base - 0x20)
        self.index = head + self.index
        names = set()
        for m in re.finditer(rb"[\x01-\x7f]\x00\x00\x00([\x01-\x7f])\x00\x00\x00([A-Za-z0-9_]{2,})\x00", self.index):
            if m.group(1)[0] == len(m.group(2)):
                names.add(m.group(2).decode())
        self.hits = {}
        for n in names:
            key, at, found = struct.pack("<I", fnv1a(n)), 0, []
            while (at := self.index.find(key, at)) >= 0:
                found.append(at)
                at += 1
            self.hits[n] = found
        self.pages = {}
        for n, found in self.hits.items():
            if re.fullmatch(r"Page_\d+", n):
                for i in found:
                    fmt, w, h = struct.unpack_from("<3H", self.index, i + 0x28)
                    off, size = struct.unpack_from("<2Q", self.index, i - 0x10)
                    if fmt == 28 and size >= w * h * 4:
                        self.pages[int(n[5:])] = (w, h, off)
        self.cache = {}

    def atlas(self, name):
        """The atlas `name` (an RGBA image), or None when this package does not have it."""
        if name in self.cache:
            return self.cache[name]
        img = None
        for i in self.hits.get(name, []):
            zero, flags, page = struct.unpack_from("<3I", self.index, i + 4)
            x, y, w, h = struct.unpack_from("<4H", self.index, i + 16)
            if zero or not 0 < w <= 8192 or not 0 < h <= 8192:
                continue
            if page in self.pages:
                pw, ph, off = self.pages[page]
                self.f.seek(self.base + off)
                full = np.frombuffer(self.f.read(pw * ph * 4), dtype=np.uint8).reshape(ph, pw, 4)
                img = Image.fromarray(full[y:y + h, x:x + w].copy(), "RGBA")
                break
            tiles_at, index_at, side, per_index = struct.unpack_from("<2I2H", self.index, i + 0x20)
            if side in (4, 8, 16) and per_index in (1, 2, 4) and w % side == 0 and h % side == 0:
                data = self.data(name)
                if data is None:
                    continue
                cols, rows = w // side, h // side
                dtype = {1: "u1", 2: "<u2", 4: "<u4"}[per_index]
                index = np.frombuffer(data[index_at * 4:index_at * 4 + cols * rows * per_index], dtype=dtype).astype(np.int64)
                tiles = np.frombuffer(data[tiles_at * 4:index_at * 4], dtype=np.uint8).reshape(-1, side, side, 4)
                full = tiles[index].reshape(rows, cols, side, side, 4).transpose(0, 2, 1, 3, 4).reshape(h, w, 4)
                img = Image.fromarray(full, "RGBA")
                break
        self.cache[name] = img
        return img

    def data(self, name):
        """The data of a texture of its own: the record of u64 offset, u64 size right before its hash."""
        for i in self.hits.get(name, []):
            off, size = struct.unpack_from("<2Q", self.index, i - 0x10)
            self.f.seek(0, 2)
            if 0 < size and self.base + off + size <= self.f.tell():
                self.f.seek(self.base + off)
                return self.f.read(size)
        return None


def icon_definitions(folders):
    """The atlases (name -> [(icon size, icons per row, file)]) and the icons (name -> (atlas, index)) of the
    Icons_*.xml files in the folders, later files overriding earlier ones."""
    atlases, icons = {}, {}
    for folder in folders:
        for f in sorted(folder.rglob("*.xml")):
            try:
                text = f.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            if "IconTextureAtlases" not in text and "IconDefinitions" not in text:
                continue
            try:
                root = ET.fromstring(text)
            except ET.ParseError:
                continue
            for table in root:
                for row in table:
                    a = row.attrib
                    if table.tag == "IconTextureAtlases" and "Filename" in a and "IconSize" in a:
                        atlases.setdefault(a["Name"], []).append((int(a["IconSize"]), int(a["IconsPerRow"]), a["Filename"]))
                    elif table.tag == "IconDefinitions" and "Atlas" in a and "Index" in a:
                        icons[a["Name"]] = (a["Atlas"], int(a["Index"]))
    return atlases, icons


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--game", type=Path, default=steam_dir() / "steamapps/common/Sid Meier's Civilization VI")
    ap.add_argument("--workshop", type=Path, default=steam_dir() / "steamapps/workshop/content/289070")
    a = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    texture(a.game / "Base/Platforms/Windows/BLPs/SHARED_DATA/TEXTURE_YieldOverlayAtlas").save(OUT / "yields.png")

    # the base game, then the DLCs (the expansions last, as the game loads them), then the mods
    dlcs = sorted((a.game / "DLC").iterdir(), key=lambda p: (p.name.startswith("Expansion"), p.name))
    roots = [a.game / "Base"] + dlcs + (sorted(a.workshop.iterdir()) if a.workshop.exists() else [])
    packages = [Package(p) for r in roots for p in sorted(r.glob("**/Platforms/Windows/BLPs/UI/Icons.blp"))]
    atlases, icons = icon_definitions([a.game / "Base/Assets", *[d for d in dlcs], *([a.workshop] if a.workshop.exists() else [])])

    written = {}
    for name, (atlas, index) in sorted(icons.items()):
        if not name.startswith(WANTED) or name.endswith("_FOW"):
            continue
        # the largest size of the atlas a package has
        for size, per_row, file in sorted(atlases.get(atlas, []), reverse=True):
            img = next((im for p in reversed(packages) if (im := p.atlas(Path(file).stem)) is not None), None)
            if img is None:
                continue
            x, y = (index % per_row) * size, (index // per_row) * size
            if y + size > img.height:
                continue
            img.crop((x, y, x + size, y + size)).resize((SIZE, SIZE), Image.LANCZOS).save(OUT / f"{name}.png")
            written[name] = f"{atlas} {index} ({file})"
            break
    (OUT / "icons.json").write_text(json.dumps(sorted(written), indent=0))
    missing = sorted(n for n in icons if n.startswith(WANTED) and not n.endswith("_FOW") and n not in written)
    print(f"wrote {len(written)} icons to {OUT}; not found: {len(missing)} {missing[:10]}")


if __name__ == "__main__":
    main()
