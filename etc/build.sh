#!/usr/bin/env bash

set -exuo pipefail

cargo build --target aarch64-apple-darwin --release
cargo build --target x86_64-apple-darwin --release

lipo -create -output target/release/checklfs \
    target/aarch64-apple-darwin/release/checklfs \
    target/x86_64-apple-darwin/release/checklfs
