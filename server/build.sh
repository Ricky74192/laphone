#!/bin/bash
# Build the on-phone input server: a static ARM64 binary (server/native,
# Rust) cross-compiled with the musl target, copied to server/build/
# laphone-input. The binary is committed so the mirror works out of the box
# (no per-machine toolchain needed), like scrcpy does with its server jar.
#
# Prereqs: rustup target add aarch64-unknown-linux-musl (rust-lld is bundled
# with the toolchain; no cross C compiler needed).
set -eu
cd "$(dirname "$0")/.."
cd server/native

RUSTFLAGS="-C linker=rust-lld" cargo build --release --target aarch64-unknown-linux-musl

mkdir -p ../build
cp target/aarch64-unknown-linux-musl/release/laphone-input ../build/laphone-input
ls -la ../build/laphone-input
echo "BUILD OK"
