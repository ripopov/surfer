# Repository Guidelines

## Project Structure & Module Organization
The workspace ties together the GUI (`surfer/`), shared core (`libsurfer/`), headless remote server (`surver/`), mdBook docs (`translator-docs/`, `docs/`), plus helpers like `wasm_example_translator/` and `examples/`. Most feature logic (rendering, waveform parsing, translators) resides under `libsurfer/src/`. GUI assets, the Trunk WASM build, and cache files are in `surfer/assets/`, while baseline screenshots live in `snapshots/`. Defaults live in `default_config.toml`, `default_theme.toml`, and `themes/`.

## Build, Test, and Development Commands
- `cargo build --workspace --all-features` ensures every crate and feature flag still compiles.
- `cargo run -p surfer --bin surfer -- <wave.vcd>` exercise the desktop UI; add `--features accesskit` for AccessKit builds.
- `cargo run -p surver -- --file <wave.vcd>` exercises the remote/server binary headlessly.
- `cargo test --workspace` covers unit, network (`wcp`), and snapshot scaffolding.
- `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings` enforces formatting and lint gates used by CI/pre-commit.
- `pre-commit run --all-files` executes the repo hooks (fmt, spell-check, oxipng) when you cannot rely on automatic installation.

## Coding Style & Naming Conventions
Sources rely on `rustfmt` (4-space indentation, trailing commas) guided by the repo `rustfmt.toml`, so never hand-tune formatting. Modules/files stay in `snake_case`, public types remain `UpperCamelCase`, and CLI flags or features match their crate names (see `surfer/Cargo.toml`). Keep logic in small functions inside the relevant `libsurfer` submodule and guard optional behavior with the existing Cargo features so WASM, desktop, and surver builds remain aligned. Always run `fmt`/`clippy`; CI rejects warnings.

## Testing Guidelines
Tests live in `libsurfer/src/tests/` (remote, WCP TCP, and snapshot suites). Visual regressions render through `snapshot::render_and_compare_inner` and compare against PNGs in `snapshots/`; after a deliberate UI change run `cargo test -p libsurfer snapshot:: && ./accept_snapshots.bash` and commit the updated `.png` files plus removed `.diff.png`. Name new tests by scenario (`analog_zoom_handles`) and mention scripted steps inside the helper so future contributors can replay it. Attach screenshots or GIFs to merge requests for UI-heavy changes to supplement automated diffs.

## Commit & Pull Request Guidelines
Commits follow the existing imperative style (“Implement analog signal visualization”, “Fix build”) and should touch a single concern. Reference issues in commit trailers or the merge request description, summarize verification steps (commands, data files), and flag any follow-ups. Pull requests should explain user impact, mention whether snapshots or assets changed, and attach visuals when modifying rendering. Before requesting review, finish the `fmt`, `clippy`, `test`, and `pre-commit` suite locally to keep CI green.
