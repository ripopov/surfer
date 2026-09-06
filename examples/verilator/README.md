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

`main.cpp` drives both simulations for timestamps 0 through 30. These are native
simulator recordings, not traces assembled by a test writer. Source filenames
are relative to this directory so navigation works after moving the checkout.
The VDB producer manifest also records the compiler's installed standard files;
those are not needed to navigate these example modules.

To regenerate inside the parent VTR checkout, first build its release C API and
the pinned Verilator integration, then run `python3 examples/verilator/generate.py`
from the Surfer directory. The script accepts `--verilator` and `--build-dir`.
FST builds require `pkg-config` and `liblz4`. Build products and logs go outside
the examples directory. The script preserves the simulator's shared VTR/VDB
identity and renames the runtime `.vdb.json` companion to `.vdb`.
