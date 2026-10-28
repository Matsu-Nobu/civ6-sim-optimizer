"""Writes the game's own images the replay viewer (viewer/) draws, from your Civilization VI installation, to
viewer/public/data/icons/ (the images are the game's: they stay out of git).

  game_icons.py [--game <the game's folder>]

Each texture is a file in the game's BLPs/SHARED_DATA folder: a 0x30-byte header (at 0x20: 1, mip count, DXGI format,
width, height) and then the mips, largest first, at the end of the file. Only BC1/BC3 (DXT1/DXT5) textures are
read; Pillow decodes them as a DDS file.
"""
import argparse
import io
import struct
import sys
from pathlib import Path

from PIL import Image

import civ6opt

OUT = civ6opt.ROOT / "viewer" / "public" / "data" / "icons"

# DXGI_FORMAT -> the DDS four-character code and bytes per 4x4 block
FORMATS = {71: (b"DXT1", 8), 72: (b"DXT1", 8), 77: (b"DXT5", 16), 78: (b"DXT5", 16)}

# the textures the viewer uses: the yield icons drawn on the map (food, production, gold, science, culture, faith in
# columns of 128 pixels; one to four of a yield and a large one for five or more in rows of 128 pixels)
TEXTURES = {"yields": "TEXTURE_YieldOverlayAtlas"}


def steam_dir():
    if sys.platform == "win32":
        return Path(r"C:\Program Files (x86)\Steam")
    if sys.platform == "darwin":
        return Path.home() / "Library/Application Support/Steam"
    return Path("/mnt/c/Program Files (x86)/Steam")


def texture(path):
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


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--game", type=Path, default=steam_dir() / "steamapps/common/Sid Meier's Civilization VI")
    a = ap.parse_args()
    shared = a.game / "Base/Platforms/Windows/BLPs/SHARED_DATA"
    OUT.mkdir(parents=True, exist_ok=True)
    for name, tex in TEXTURES.items():
        texture(shared / tex).save(OUT / f"{name}.png")
        print("wrote", OUT / f"{name}.png")


if __name__ == "__main__":
    main()
