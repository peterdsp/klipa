#!/usr/bin/env python3
"""Pack PNG files into a Windows .ico (PNG-compressed entries).

No third-party deps. Usage: png2ico.py out.ico in1.png in2.png ...
Each PNG is embedded as-is; Windows Vista+ reads PNG-compressed icons.
"""
import struct
import sys


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: png2ico.py out.ico in1.png [in2.png ...]", file=sys.stderr)
        return 2
    out, pngs = sys.argv[1], sys.argv[2:]
    images = []
    for p in pngs:
        with open(p, "rb") as f:
            data = f.read()
        # PNG IHDR width/height live at byte offset 16..24.
        w, h = struct.unpack(">II", data[16:24])
        images.append((w, h, data))

    n = len(images)
    header = struct.pack("<HHH", 0, 1, n)  # reserved, type=icon, count
    offset = 6 + n * 16
    entries, blobs = b"", b""
    for w, h, data in images:
        bw = 0 if w >= 256 else w
        bh = 0 if h >= 256 else h
        entries += struct.pack(
            "<BBBBHHII", bw, bh, 0, 0, 1, 32, len(data), offset
        )
        blobs += data
        offset += len(data)

    with open(out, "wb") as f:
        f.write(header + entries + blobs)
    print(f"wrote {out} ({n} sizes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
