# CHI NoC packet trace

Open `chi_noc.vtr` in Surfer and add `chi_noc.RNF0.tx` and
`chi_noc.HNF0.tx` to the waveform canvas. Zoom to 0–260 ns for a readable
four-packet burst; zoom to 400–640 ns to inspect the 64-packet burst.
Each rectangle is one packet containing exactly one flit, not an entire
coherence request and its responses. Hollow dots on each bar mark its timed router events. Hover a dot for its
name, timestamp and attributes; click it to select the packet. Selecting a
packet also exposes the full event list in transaction details.

The deterministic 3×2 mesh has one controller at each router:

| Router coordinates | x=0 | x=1 | x=2 |
|---|---|---|---|
| y=0 | RNF0 / router_0 | HNF0 / router_1 | RNI0 / router_2 |
| y=1 | RNF1 / router_3 | HNF1 / router_4 | RNI1 / router_5 |

Each controller owns a `tx` stream with opcode generators. RN-F sources
emit REQ/ReadShared and RSP/SnpResp packets; HN-F sources emit SNP/SnpShared
and DAT/CompData packets; RN-I sources emit REQ/ReadNoSnp packets.
The channel roles and packet/flit terminology follow Arm's
[Introducing AMBA CHI](https://documentation-service.arm.com/static/682600710aae2a5d8f044978).

This is synthetic traffic for transaction visualization, not a complete CHI
coherence or link-credit simulation. Opcodes illustrate independent outgoing
packets; they are not paired into protocol exchanges. Data payloads, credits,
arbitration, snoop state and protocol dependencies are not modeled. The mesh,
40 ns router residence time and traffic schedule are fixture choices, not CHI
requirements. There is no assumption that physical links accept 64 flits at once:
the overlap includes packets resident in the modeled routers.

Routes move along X and then Y. A timestamped event identifies every visited
router, including injection and destination routers, with `router` and `hop`
attributes. Each packet carries `channel`, `opcode`, `SrcID`, `TgtID`, a
controller-local `TxnID`, `flits=1` and a synthetic cache-line address.
Addresses are illustrative metadata, including on response/data packets,
not a declaration of encoded CHI flit fields. Router names, coordinates and
routing describe the generated topology; no colors or canvas layout enter VTR.

Every controller emits bursts of 4, 64 and 8 packets, for 76 packets per
controller and 456 total. The bursts do not overlap. The middle burst starts
one packet per ns and all 64 lifetimes overlap, so the peak is exactly 64 in
each controller. TxnID slots are reused only after the preceding burst ends.

From the Surfer repository root, reproduce and verify with:

```sh
cargo run -p libsurfer --example generate_chi_noc
cargo test -p libsurfer --example generate_chi_noc
cargo test -p libsurfer --lib vtr_chi_noc_packet_streams
```

The generator reads the resulting file back and validates packet counts,
single-flit attributes, every timed hop and per-stream peak concurrency.
The example test validates the committed artifact; the PNG regression in
`libsurfer/src/tests/snapshot.rs` renders two complete streams over the sparse
burst. The full 64-packet burst remains in the same file for stress browsing.
The generator uses existing VTR hierarchy, transactions, attributes and events;
no format or public API changes are required.
