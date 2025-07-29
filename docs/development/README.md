# Development Information

Anyone is welcome to contribute to Surfer. As Surfer is licensed under
[EUPL 1.2](https://interoperable-europe.ec.europa.eu/collection/eupl/eupl-text-eupl-12)
it is assumed that your contribution will also follow that license.

Once you find something to contribute, either that feature that you are missing or one of the [issues](https://gitlab.com/surfer-project/surfer/-/issues), the pattern follows a regular git-like contribution:

1. [Fork](https://docs.gitlab.com/user/project/repository/forking_workflow/) and [clone](https://docs.gitlab.com/topics/git/clone/) the repository
2. Setup [pre-commit](https://pre-commit.com/)
3. Create a [branch](https://docs.gitlab.com/topics/git/branch/) (other than `main`)
4. Edit code
5. [Commit](https://docs.gitlab.com/topics/git/commit/) code with a [sensible commit message](https://cbea.ms/git-commit/)
6. [Push](https://docs.gitlab.com/topics/git/commit/#send-changes-to-gitlab) branch
7. Create a [merge request](https://docs.gitlab.com/user/project/merge_requests/creating_merge_requests/)
8. Wait for the change to be merged, including fixing suggestions from reviewers
9. Enjoy the new feature!

## Pre-Commit

Surfer uses the [pre-commit](https://pre-commit.com/) framework to do some basic
checking when committing new code locally. By using this, the risk of CI errors
is reduced.

This is a Python package, so the instructions below assumes that you have a working Python in your console.

1. Install pre-commit:

```bash
pip install pre-commit
```

2. In the Surfer source-code directory:

```bash
pre-commit install
```

The first time you commit, there will be things installed, so the time taken can be
long. However, the next time no installation is required.

Also note that if something fails, like `cargo fmt` has to reformat the code, you will
have to commit again as the pre-commit hook only formatted the code, not committed the
formatted code.

Note that the spelling check does not alter the code, but only points out errors.
Hence, these must be manually corrected before committing again.

## Tests

To run the tests locally, do

``` bash
cargo test
```

When possible, it is nice to have a test of the added code.
As Surfer is graphical to a large extent, we primarily rely on image tests
located in `libsurfer/src/tests/snapshots.sh`. These tests send a suitable
set of messages and then takes a snapshot of the screen content which is
the ground truth. Easiest way it to copy a suitable test and change the messages.

After running the coverage test in the CI, either a red or a green vertical line will
be present in the code view of the merge request to see which code was executed.
Ideally, all new code should be tested, but it is currently not realistic to have that
as a strict requirement.

### Update Image Tests

If you change something that affects rendering or adds new image tests, you will have to update the test images:

1. Run `cargo test`

2. Run `./accept_snapshots.bash`

3. Add and commit new images

Or if you for whatever reason only want to update some of the images (if you're incrementally fixing things), copy `snapshots/<test>.new.png` to `snapshots/<test>.png` (and remove `snapshots/<test>.diff.png`).

Note that the test images are compressed using oxipng as part of the pre-commit hook.

### Using egui Test Framework

When Surfer started with the graphical testing, egui did not have any testing facilities.
Now it does, and it would be much beneficial to use [egui_kittest](https://github.com/emilk/egui/tree/main/crates/egui_kittest).
This would allow not just sending messages but to actually click on things etc,
which allows both much higher test coverage and, more importantly,
certainty that Surfer works as expected.

If you prefer to write the tests using `egui_kittest` that is much appreciated and
clearly not a problem. More a step in the right direction.

## Long Compilation Times

Compiling Surfer takes a long time which can be annoying during development. To make compilation faster, you can change `lto` to `false` and `opt-level` to a `0` or `1` towards the end of `Cargo.toml` in the root directory. This will speed up compilation at the expense of slightly larger and slower binaries (which is probably OK during development anyway).

## Adding Configurations

The preferred pattern for a configuration value that can be set both in the
program and in the config file is to add an `Option`-value in the `UserState`
enum and then query the value first there, and, if not set by the user, take
it from the config. This has two benefits:

1. If set by the user in the application, it will saved in the state file.
2. If not set by the user, any changes in the config will be reflected when loading a state.

To obtain this, a function similar to the following can be added to `libsurfer/state_util.rs`

``` rust
    #[inline]
    pub fn show_default_timeline(&self) -> bool {
        self.user
            .show_default_timeline
            .unwrap_or_else(|| self.user.config.layout.show_default_timeline())
    }
```

and then this method is used everywhere to access the value.
This also requires adding a public method in `config.rs`, in this case
`show_default_timeline()`, that simply returns the corresponding config
value (which should not be public to avoid overwriting etc).

## Performance Measurement

If you want to measure performance, surfer has some features to help out.
By running the command `show_performance`, it will show a graph with the total
frame time, as well as the time taken to run various parts of the program.

The bulk of rendering is done in `signal_canvas::generate_draw_commands`.
The results of `generate_draw_commands` are cached by default, and only
recomputed when the viewport changes.
This makes performance measurement harder, but there is a switch to turn off the cache.
You can either click the "Continuous redraw" checkbox in the performance window,
or run `show_performance redraw` to turn off the cache.

### Command files

If you are debugging performance issues in a specific situation, you can use command
files to automate the setup of surfer.
For example, if you want to automatically add waves and turn on performance
measurements, you can create `performance.sucl and add command prompt arguments that reproduce the issue to it, then run

```bash
surfer <wave file> -c performance.sucl
```

For example, to check performance with many displayed waves, `performance.sucl`
may look like this:

```text
show_performance redraw
module_add testbench.top.uut
module_add testbench.top.uut
```

### Optimizations

Remember to run in release mode to get accurate performance measurements.

```bash
cargo run --bin surfer --release
```

### Flamegraphs

Flamegraphs can be generated using [cargo-flamegraph](https://github.com/flamegraph-rs/flamegraph)

```bash
CARGO_PROFILE_RELEASE_DEBUG=true ca flamegraph -- examples/picorv32.vcd -c performance.sucl
```
