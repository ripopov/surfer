#!/bin/bash
set -e

SPADE_REV="b7d5dc6c18f1e58d872ede81ff7c8b8c23d41ffe"

d="$(mktemp -d)"
pushd "$d"
git clone https://gitlab.com/spade-lang/spade
cd spade
git checkout -d $SPADE_REV
cd swim_tests
swim test pipeline_ready_valid --testcases enabled_stages_behave_normally
popd
cp "$d/spade/swim_tests/build/state.ron" ./examples/spade_state.ron
cp "$d/spade/swim_tests/build/pipeline_ready_valid_enabled_stages_behave_normally/pipeline_ready_valid.vcd" ./examples/spade.vcd

rm -rf "$d"
