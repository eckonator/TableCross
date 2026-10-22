#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Gives a Mach-O program the same UUID in every build (one per architecture).

macOS recognizes an app for the local network permission by the UUID of its program. A new build
has a new UUID, so after every update macOS drops all local traffic of the app for a minute or two
until it has looked at it again. With a fixed UUID the updated app counts as the one already
allowed. Must run before signing, the signature covers the UUID.

Usage: scripts/fixed-uuid.py PROGRAM
"""

import struct
import sys
import uuid

LC_UUID = 0x1B
FAT_MAGIC = 0xCAFEBABE
MH_MAGIC_64 = 0xFEEDFACF


def fixed(cpu_type):
    return uuid.uuid5(uuid.NAMESPACE_DNS, f"tablecross.eckonator.de/cpu/{cpu_type}").bytes


def patch_slice(data, start):
    magic, cpu_type, _, _, ncmds, _ = struct.unpack_from("<IiiIII", data, start)
    if magic != MH_MAGIC_64:
        raise SystemExit(f"not a 64-bit Mach-O slice at {start:#x}")
    at = start + 32
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<II", data, at)
        if cmd == LC_UUID:
            data[at + 8 : at + 24] = fixed(cpu_type)
            return cpu_type
        at += size
    raise SystemExit(f"no UUID in the slice at {start:#x}")


def main():
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    path = sys.argv[1]
    with open(path, "rb") as f:
        data = bytearray(f.read())
    (magic,) = struct.unpack_from(">I", data, 0)
    if magic == FAT_MAGIC:
        (count,) = struct.unpack_from(">I", data, 4)
        slices = [struct.unpack_from(">iiIII", data, 8 + 20 * i)[2] for i in range(count)]
    else:
        slices = [0]
    cpus = [patch_slice(data, start) for start in slices]
    with open(path, "wb") as f:
        f.write(data)
    print(f"{path}: fixed UUID for CPU types {cpus}")


if __name__ == "__main__":
    main()
