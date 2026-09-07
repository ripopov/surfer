# Verilator waveform and source examples

Open `pipeline.vtr` or `operators.vtr` in Surfer. Each has a same-stem `.vdb`
companion and the original SystemVerilog source beside it. The `.fst` files
record the same design with the same stimulus for waveform parity tests.

- **pipeline**: two parameterized register stages, mux selection, an enable
  hold, asynchronous reset, hierarchy, and signal aliases. At time 26 the
  output is 3; the first stage has accepted 9 while the second retains 3.
- **operators**: signed arithmetic, shifts, bit selections, enum constants,
  concatenation, an instance array, and hierarchical references. Inputs change
  away from clock edges, at time 12.
- **features**: a package with an enum, a packed struct and a function, an
  interface with modports, a parameterized counter whose generate branch is
  selected per instance (`top.g_lane[0].u` wraps, `top.g_lane[1].u` saturates
  at `FEAT_SAT_LIMIT`), a case statement, and `include/features_defs.svh`
  reached through `+incdir+include` with `+define+FEAT_SAT_LIMIT=200`. It is the
  design the source tile's highlighting and navigation tests use.

`main.cpp` drives all simulations for timestamps 0 through 30. These are native
simulator recordings, not traces assembled by a test writer. Source filenames
are relative to this directory so navigation works after moving the checkout.
The VDB producer manifest also records the compiler's installed standard files;
those are not needed to navigate these example modules. Its `elaboration` record
names the design files, include directory and defines the simulator used, and
its `source_index` section, written by `verilator_vdb_index` at verilation, is
what colors and resolves every token in the source tile.

To regenerate inside the parent VTR checkout, first build its release C API and
the pinned Verilator integration, then run `python3 examples/verilator/generate.py`
from the Surfer directory. The script accepts `--verilator` and `--build-dir`.
FST builds require `pkg-config` and `liblz4`. Build products and logs go outside
the examples directory. The script preserves the simulator's shared VTR/VDB
identity and renames the runtime `.vdb.json` companion to `.vdb`.
