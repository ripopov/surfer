#!/bin/sh

set -eo pipefail

cd surfer/surfer
git submodule update --init --recursive
trunk build --release
cd ../..
mkdir -p extension/surfer
cp surfer/surfer/dist/manifest.json \
  surfer/surfer/dist/index.html \
  surfer/surfer/dist/surfer.js \
  surfer/surfer/dist/surfer_bg.wasm \
  surfer/surfer/dist/sw.js \
  surfer/surfer/dist/integration.js \
  extension/surfer

python3 prepare.py
