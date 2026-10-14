"""Verify the approved source/assets and optionally a fresh export. Python stdlib only."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "manifest.json"
PNG_SIZES = {16, 24, 32, 48, 64, 128, 256, 512}
ICO_SIZES = {16, 20, 24, 32, 40, 48, 64, 256}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tracked_files():
    files = []
    for folder in ("assets", "document-icons", "renderer/src", "references"):
        files.extend(path for path in (ROOT / folder).rglob("*") if path.is_file())
    files.extend(ROOT / name for name in (
        "renderer/Cargo.toml", "renderer/Cargo.lock", "build-info.json"
    ))
    return sorted(files)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def check_png(data, size):
    require(data[:8] == b"\x89PNG\r\n\x1a\n", "PNG signature")
    offset, chunks, compressed = 8, [], bytearray()
    while offset < len(data):
        length = struct.unpack_from(">I", data, offset)[0]
        kind = data[offset + 4:offset + 8]
        body = data[offset + 8:offset + 8 + length]
        crc = struct.unpack_from(">I", data, offset + 8 + length)[0]
        require(zlib.crc32(kind + body) == crc, "PNG CRC")
        if kind == b"IHDR":
            width, height, bits, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", body)
            require((width, height) == size, f"PNG dimensions {(width, height)} != {size}")
            require((bits, color, compression, filtering, interlace) == (8, 6, 0, 0, 0), "Expected non-interlaced 8-bit RGBA")
        if kind == b"IDAT":
            compressed.extend(body)
        chunks.append(kind)
        offset += length + 12
    require(chunks[0] == b"IHDR" and chunks[-1] == b"IEND" and offset == len(data), "PNG chunk structure")
    raw = zlib.decompress(compressed)
    require(len(raw) == size[1] * (size[0] * 4 + 1), "PNG pixel data length")


def check_ico(path):
    data = path.read_bytes()
    reserved, kind, count = struct.unpack_from("<HHH", data)
    require((reserved, kind, count) == (0, 1, 8), f"{path.name}: ICO header/frame count")
    sizes = set()
    for index in range(count):
        width, height, colors, reserved, planes, bits, length, offset = struct.unpack_from("<BBBBHHII", data, 6 + index * 16)
        width, height = width or 256, height or 256
        require(width == height and (planes, bits) == (1, 32), "ICO frame dimensions/depth")
        require(offset >= 6 + count * 16 and offset + length <= len(data), "ICO frame bounds")
        frame = data[offset:offset + length]
        if width == 256:
            check_png(frame, (width, height))
        else:
            header, dib_width, dib_height, dib_planes, dib_bits = struct.unpack_from("<IiiHH", frame)
            require((header, dib_width, dib_height, dib_planes, dib_bits) == (40, width, height * 2, 1, 32), "ICO DIB header")
            expected_length = 40 + width * height * 4 + ((width + 31) // 32) * 4 * height
            require(length == expected_length, "ICO DIB byte length")
        sizes.add(width)
    require(sizes == ICO_SIZES, f"{path.name}: ICO sizes {sizes}")


def check_assets():
    assets = ROOT / "assets"
    apps = sorted(path.stem for path in assets.glob("*.ico"))
    require(bool(apps), "No app ICO files")
    expected = {"preview.png"}
    for app in apps:
        check_ico(assets / f"{app}.ico")
        expected.add(f"{app}.ico")
        for size in PNG_SIZES:
            name = f"{app}-{size}.png"
            check_png((assets / name).read_bytes(), (size, size))
            expected.add(name)
    require({path.name for path in assets.iterdir() if path.is_file()} == expected, "Unexpected or missing asset files")
    check_png((assets / "preview.png").read_bytes(), (len(apps) * 200 + 20, 452))
    return len(apps)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rendered", type=Path, help="Compare a freshly generated output folder")
    parser.add_argument("--existing-only", action="store_true", help="Check old app outputs only while adding an app; skip source/contact-sheet changes")
    parser.add_argument("--write-manifest", action="store_true", help="Explicitly replace the approved checksum baseline after review")
    args = parser.parse_args()
    require(not (args.write_manifest and (args.rendered or args.existing_only)), "Write the manifest separately from checking an export")
    require(not args.existing_only or args.rendered, "--existing-only requires --rendered")
    count = check_assets()
    current = {path.relative_to(ROOT).as_posix(): digest(path) for path in tracked_files()}
    if args.write_manifest:
        MANIFEST.write_text(json.dumps({"schema": 1, "algorithm": "sha256", "files": current}, indent=2) + "\n", encoding="utf-8")
        print(f"Recorded approved baseline: {len(current)} files, {count} apps. Review the manifest diff.")
        return
    approved = json.loads(MANIFEST.read_text(encoding="utf-8"))["files"]
    if not args.existing_only:
        require(current.keys() == approved.keys(), "Baseline file list changed; review added/removed files")
        for name, value in approved.items():
            require(current[name] == value, f"Approved file changed: {name}")
        print(f"Approved source/assets verified: {len(approved)} checksums; {count} apps, 8 ICO frames each.")
    if args.rendered:
        expected = {Path(name).name: value for name, value in approved.items() if name.startswith("assets/")}
        if args.existing_only:
            expected.pop("preview.png")
        else:
            require({path.name for path in args.rendered.iterdir() if path.is_file()} == expected.keys(), "Rendered export contains missing or unexpected files")
        for name, value in expected.items():
            path = args.rendered / name
            require(path.is_file(), f"Missing rendered asset: {name}")
            require(digest(path) == value, f"Rendered asset differs: {name}")
        print(f"Fresh export: {len(expected)} files match the approved bytes exactly.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, struct.error, zlib.error) as error:
        raise SystemExit(f"Verification failed: {error}") from error
