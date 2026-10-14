"""Compare all frames in an executable's selected RT_GROUP_ICON against a reference ICO."""
import argparse
import io
from pathlib import Path
import struct


def extract(executable, group_id):
    import pefile
    with pefile.PE(str(executable), fast_load=True) as pe:
        pe.parse_data_directories(directories=[pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_RESOURCE"]])
        resources = {entry.id: entry for entry in pe.DIRECTORY_ENTRY_RESOURCE.entries}
        groups = resources[14].directory.entries
        group = groups[0] if group_id is None else next(item for item in groups if item.id == group_id)
        # Default language entry, matching the app builds used for this kit.
        item = group.directory.entries[0].data.struct
        data = pe.get_data(item.OffsetToData, item.Size)
        count = struct.unpack_from("<H", data, 4)[0]
        icons = {entry.id: entry for entry in resources[3].directory.entries}
        entries = []
        for index in range(count):
            descriptor = data[6 + index * 14:6 + (index + 1) * 14]
            icon_id = struct.unpack_from("<H", descriptor, 12)[0]
            resource = icons[icon_id].directory.entries[0].data.struct
            frame = pe.get_data(resource.OffsetToData, resource.Size)
            entries.append((descriptor[:8] + struct.pack("<I", len(frame)), frame))
        result = bytearray(data[:6])
        offset = 6 + 16 * count
        for descriptor, frame in entries:
            result.extend(descriptor + struct.pack("<I", offset))
            offset += len(frame)
        for _, frame in entries:
            result.extend(frame)
        return bytes(result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument("reference_ico", type=Path)
    parser.add_argument("--group-id", type=int, help="Numeric icon resource group; default is first group")
    args = parser.parse_args()
    from PIL import Image
    with Image.open(io.BytesIO(extract(args.executable, args.group_id))) as actual, Image.open(args.reference_ico) as reference:
        expected_sizes = reference.ico.sizes()
        if actual.ico.sizes() != expected_sizes:
            raise SystemExit(f"ICO sizes differ: {actual.ico.sizes()} != {expected_sizes}")
        for size in sorted(expected_sizes):
            a = actual.ico.getimage(size).convert("RGBA")
            b = reference.ico.getimage(size).convert("RGBA")
            if a.tobytes() != b.tobytes():
                raise SystemExit(f"Embedded icon differs at {size}")
        print(f"Verified {args.executable.name}: all {len(expected_sizes)} embedded frames match the reference pixels.")


if __name__ == "__main__":
    main()
