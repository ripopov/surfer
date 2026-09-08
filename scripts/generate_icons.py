#!/usr/bin/env python3
"""Render the checked-in application assets from logo-small.svg (requires librsvg)."""
import argparse
from pathlib import Path
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'surfer/assets/logo-small.svg'


def render(size):
    return subprocess.run(
        ['rsvg-convert', '--width', str(size), '--height', str(size), str(SOURCE)],
        check=True, stdout=subprocess.PIPE,
    ).stdout


def favicon(images):
    # ICO directory followed by PNG frames; width/height 0 denotes 256 pixels.
    offset = 6 + 16 * len(images)
    directory = bytearray(struct.pack('<HHH', 0, 1, len(images)))
    for size, data in images:
        directory.extend(struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0,
                                     1, 32, len(data), offset))
        offset += len(data)
    return bytes(directory) + b''.join(data for _, data in images)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='fail if generated assets differ')
    args = parser.parse_args()
    sizes = (16, 24, 32, 48, 64, 128, 256, 350, 512)
    pngs = {size: render(size) for size in sizes}
    ico = favicon([(size, pngs[size]) for size in sizes if size <= 256])
    outputs = {
        'surfer/assets/com.gitlab.surferproject.surfer.png': pngs[256],
        'surfer/assets/logo.png': pngs[512],
        'surfer/assets/favicon.ico': ico,
        'surver/assets/favicon.ico': ico,
        'surfer-vscode/extension/icon.png': pngs[350],
    }
    stale = []
    for name, data in outputs.items():
        path = ROOT / name
        if args.check:
            if not path.exists() or path.read_bytes() != data:
                stale.append(name)
        else:
            path.write_bytes(data)
    if stale:
        print('Regenerate icons: python3 scripts/generate_icons.py', file=sys.stderr)
        print('\n'.join(stale), file=sys.stderr)
        return 1
    print('Icon assets match the SVG.' if args.check else 'Generated five icon assets.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
