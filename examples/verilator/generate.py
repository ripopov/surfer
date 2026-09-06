#!/usr/bin/env python3
"""Regenerate small, real Verilator recordings in both VTR/VDB and FST."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
VTR = HERE.parents[3]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--verilator', type=Path, default=VTR / 'bench/build/verilator/install/bin/verilator')
    parser.add_argument('--build-dir', type=Path, default=VTR / 'bench/build/surfer-examples')
    args = parser.parse_args()
    # Link the static library only, so the simulators do not depend on libvtr.so at run time.
    lib = args.build_dir.resolve() / 'lib'
    lib.mkdir(parents=True, exist_ok=True)
    shutil.copy2(VTR / 'target/release/libvtr.a', lib / 'libvtr.a')
    env = dict(os.environ, VTR_INCLUDE=str(VTR / 'crates/vtr-capi/include'), VTR_LIBDIR=str(lib))
    for name in ('pipeline', 'operators', 'features'):
        for mode in ('vtr', 'fst'):
            obj = args.build_dir.resolve() / name / mode
            obj.mkdir(parents=True, exist_ok=True)
            flags = []
            if mode == 'fst':
                for flag, query in (('-CFLAGS', '--cflags'), ('-LDFLAGS', '--libs')):
                    value = subprocess.check_output(['pkg-config', query, 'liblz4'], text=True).strip()
                    if value:
                        flags.extend([flag, value])
            with (obj / 'build.log').open('w') as log:
                subprocess.run([str(args.verilator.resolve()), '--cc', '--exe', '--build', '-j', '2',
                                '--top-module', 'top', '--prefix', 'Vtop', '--Mdir', str(obj),
                                f'--trace-{mode}', '-Wno-fatal', '-CFLAGS', '-DVDB_' + name.upper(),
                                *flags, *(['+incdir+include', '+define+FEAT_SAT_LIMIT=200'] if name == 'features' else []),
                                name + '.sv', 'main.cpp', '-o', 'sim'],
                               cwd=HERE, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            subprocess.run([str(obj / 'sim'), str(HERE / f'{name}.{mode}')], cwd=HERE, check=True)
            if mode == 'vtr':
                (HERE / f'{name}.vdb.json').replace(HERE / f'{name}.vdb')
            print(f'{name}.{mode}: simulated', flush=True)


if __name__ == '__main__':
    main()
