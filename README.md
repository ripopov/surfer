# Surfer

![Pipeline status](https://gitlab.com/surfer-project/surfer/badges/main/pipeline.svg)
![License](https://img.shields.io/gitlab/license/surfer-project%2Fsurfer)
![Code coverage](https://gitlab.com/surfer-project/surfer/badges/main/coverage.svg)
![Contributors](https://img.shields.io/gitlab/contributors/surfer-project%2Fsurfer)

A waveform viewer with a focus on a snappy usable interface, and extensibility.

![A screenshot of surfer](snapshots/render_readme_screenshot.png)

## Documentation

There are automatically generated [source documentation](https://docs.surfer-project.org/surfer/)
and an extremely early version of [user documentation](https://docs.surfer-project.org/book/).

## Development Discussion

Contributions are more than welcome! The easiest way is to just create an issue
to discuss a change, or just make merge request with your changes directly. If
you want more in-depth discussion we have bi-weekly meetings to coordinate
development, see [meeting notes](https://hackmd.io/k_C-F7CdS9KVSRJu08tKGg) for more details. You
can also hop in our [matrix channel](https://matrix.to/#/#surfer-project:matrix.org) for asynchronous discussions.

## Installation

If you just want to try things out, you can try surfer right in the web-browser
at <https://app.surfer-project.org/>. However, performance is a bit worse than
native, and some features are missing so for day-to-day use it is recommended
to install the native version.

The primary way to install surfer to build it from source which should work on
Linux, Mac and Windows. For convenience we also provide pre-built Linux and Windows
binaries.

If you want to include the [AccessKit](https://accesskit.dev/) integration from eframe,
add `--features accesskit` at the end of any `cargo build` or `cargo install` command.
This is enabled by default for the pre-built binaries.

See [Documentation](https://docs.surfer-project.org/book/#installation) for further instructions.

### WSL support

It is possible to run Surfer in WSL (Windows Subsystem for  Linux). However, there are several [issues](https://gitlab.com/surfer-project/surfer/-/issues/?label_name%5B%5D=platform%3A%20WSL)
reported that most likely are caused by the gui framework used (as in, Surfer cannot really affect it).
These are the suggested solutions if it does not work for you:

1. There is a script, `surfer.sh` in the repository, that can be used to start the Windows version from WSL, avoiding the second caveat mentioned below. Read the instructions in the script. This is the preferred way as the Windows version is faster.
2. Start the Surfer server `surver`, see below, in WSL and open the Windows version using the URL provided from running the previous command.
3. Start Surfer with the environment variable `WAYLAND_DISPLAY` cleared. For example, `WAYLAND_DISPLAY= surfer`.
4. Compile Surfer with a change in `Cargo.toml` as below (replace the line defining `eframe` version, using the current version if the line below has an older version).  Installing `libgtk-3-dev` and/or `zenity` may be required if errors remain (although most likely it is a dependency of that package that is really required).

```toml
  eframe = { version="0.31.1",  features = ["glow", "x11", "default_fonts"], default-features = false}
```

5. Install the VS Code [extension](https://marketplace.visualstudio.com/items?itemName=surfer-project.surfer). This will allow you to just write `code filename.vcd`  and
   start the Windows version of VS Code with the WASM-version of Surfer (if it opens the VCD as text, make sure that you trust the containing folder)

## Configuration

Many aspects of Surfer can be configured.
To learn more about configuration, have a look at our [wiki](https://gitlab.com/surfer-project/surfer/-/wikis/Configuration).

## Server Mode (Surver)

It is possible to run Surfer in client-server mode, where the server, `surver` is started on one computer with one or more waveform files and the waveform viewer is opened on another computer. This avoids having to copy the waveform files. To do this, run

```bash
surver waveform.vcd/fst/ghw
```

and follow the instructions.

It is also possible to start Surfer in server mode, although there are plans to drop this at some stage and only support `surver`

```bash
surfer server --file waveform.vcd/fst/ghw
```

## Development Information

Contributions to Surfer are very welcome! See the
[list of issues](https://gitlab.com/surfer-project/surfer/-/issues) if you do not have any
suggestions on your own. Some basic [development information](https://gitlab.com/surfer-project/surfer/-/wikis/home)
is available.

## Integration

Surfer is designed to be easy to integrate in other projects, especially web
applications. The easiest way to do so is to embed the viewer as an `<iframe>`,
either the publicly hosted `app.surfer-project.org` or a local copy.

A pre-compiled web-assembly build can be downloaded from
`https://gitlab.com/surfer-project/surfer/-/jobs/artifacts/main/download?job=pages_build`

To control the embedded waveform viewer, use the `postMessage` function on the
`iframe`. Information about the API for this can be found in
`surfer/assets/integration.js`. Note that these functions can only be run after Surfer has been
loaded. To ensure this, modify the HTML to replace `/*SURFER_SETUP_HOOKS*/` with JavaScript which
notifies your application that it has loaded, for example, the VSCode plugin
does this using the following snippet

```javascript
const load_notifier = `
    (function() {
        const vscode = acquireVsCodeApi();

        vscode.postMessage({
            command: 'loaded',
        })
    }())`
html = html.replaceAll("/*SURFER_SETUP_HOOKS*/", `${load_notifier}`)
```

It is also possible to embed surfer without an iframe. An example of this can
be found in `https://gitlab.com/surfer-project/orconf2024#`. However, note that
the API for messages is unstable.

## Project Status

Surfer is in a usable state. In fact, if
you are able to take advantage of the extensibility such as with the
[Spade](https://spade-lang.org) integration, you may even prefer it to the alternatives.

As an indication of the status of the project, here is an incomplete list of supported and planned features

- [x] Basic rendering of waves
- [x] Snappy zooming, panning and general interaction
- [x] Waveform file format support
  - [x] VCD loading
  - [x] FST loading
  - [x] GHW loading
- [x] Memory transaction file format support
  - [x] FTR loading
- [x] [Fuzzy completion based command line interface](misc/surfer_ui_trimmed.mp4)
- [x] Bit-vector translation
  - [x] Raw bits
  - [x] Hexadecimal values
  - [x] Unsigned values
  - [x] [Spade](https://spade-lang.org) values
  - [x] Signed values
  - [x] Octal values
  - [x] VHDL nine-valued std_ulogic support
  - [x] ASCII
  - [x] Fixed-point based on bit-index (`fixed_pkg`)
  - [x] Floating-point
    - [x] IEEE 754 binary128 (quad), use feature `f128`, using [f128](https://docs.rs/f128/) (requires gcc)
    - [x] IEEE 754 binary64 (double)
    - [x] IEEE 754 binary32 (single)
    - [x] IEEE 754 binary16 (half), using [half](https://docs.rs/half/)
    - [x] bfloat16, using [half](https://docs.rs/half/)
    - [x] 8-bit E5M2
    - [x] 8-bit E4M3
    - [ ] Based on bit-inded (`float_pkg`)
  - [x] Posit, using [softposit](https://docs.rs/softposit/)
    - [x] 32-bit, two exponent bits
    - [x] 16-bit, one exponent bit
    - [x] 8-bit, no exponent bit
    - [ ] Quire for 32-bit, two exponent bits
    - [x] Quire for 16-bit, one exponent bit
    - [x] Quire for 8-bit, no exponent bit
  - [x] Instruction decoding using [instruction-decoder](https://github.com/ics-jku/instruction-decoder)
    - [x] RV32
    - [x] RV64
    - [x] MIPS
    - [x] LoongArch64 (LA64)
    - [ ] Arm
  - [x] Compressed integers
    - [x] LEB128 (also for other bitwidths than 128)
  - [x] Number of ones
  - [x] Leading/trailing zeros/ones
  - [x] Identical MSBs
  - [ ] Custom translation via Python API
  - [x] Custom translation via WASM API
- [x] Dividers
- [x] Wave file reloading
- [x] Saving and loading selected waves
- [x] Cursors for measuring time
- [x] Mouse gesture control
- [x] Keyboard commands
  - [ ] Configurable keyboard commands
- [ ] [WAL](https://wal-lang.org) integration
- [x] VS Code [extension](https://marketplace.visualstudio.com/items?itemName=surfer-project.surfer)
- [x] Remote support
  - [x] Serving wave forms from remote server
  - [x] Multi-file support for remote server
  - [ ] Remote control of Surfer
- [x] Scripting
  - [x] Loading of commands on startup
  - [x] Loading of script file/commands in running instance
  - [ ] Logging of commands (generation of log file to be executed later)
- [x] Grouping of variables
- [ ] Computed variables, e.g., `a >= 100`
- [ ] Clock period time unit
- [x] Configurable color schemes
- [x] Analog drawing

## License

Surfer is licensed under the [EUPL-1.2 license](LICENSE-EUPL-1.2.txt).
