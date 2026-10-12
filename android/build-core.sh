#!/bin/sh
# Builds the Rust core crate for Android and generates its Kotlin bindings. Gradle's preBuild calls this.
#
# - Output goes under app/build/generated and is not committed.
# - Requires the rustup target aarch64-linux-android, cargo-ndk, and the Android NDK
set -eu

here=$(cd "$(dirname "$0")" && pwd)
out="$here/app/build/generated"
export PATH="$HOME/.cargo/bin:$PATH"

cd "$here/.."

# Crypto and SRTP are too slow in debug builds, so this always builds release regardless of the app's build type.
cargo ndk -t arm64-v8a -P 29 -o "$out/jniLibs" build --release

# Bindings are generated from the metadata of the library just built.
rm -rf "$out/uniffi"
cargo run -q --example uniffi-bindgen -- \
    generate "$out/jniLibs/arm64-v8a/libnaru.so" -l kotlin -o "$out/uniffi" -n
