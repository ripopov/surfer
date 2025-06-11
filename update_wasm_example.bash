#!/bin/bash

cd wasm_example_translator || exit

cargo build --release

cd ..

cp ./target/wasm32-unknown-unknown/release/wasm_example_translator.wasm examples
