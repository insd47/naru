#!/bin/sh
# Builds the Rust core crate as an iOS static library and generates the Swift bindings. Called from an Xcode build phase.
#
# - Builds only the library for the PLATFORM_NAME Xcode passes (iphoneos, iphonesimulator), or both when it is unset.
# - Output goes under build/generated and is not committed. The project uses the library and headers from that path.
# - Requires the rustup targets aarch64-apple-ios and aarch64-apple-ios-sim
set -eu

here=$(cd "$(dirname "$0")" && pwd)
out="$here/build/generated"
platforms=${PLATFORM_NAME:-"iphoneos iphonesimulator"}
export PATH="$HOME/.cargo/bin:$PATH"

# If the iOS SDK settings Xcode passes are left in place, cargo tries to link build scripts for iOS and fails.
unset SDKROOT IPHONEOS_DEPLOYMENT_TARGET

cd "$here/.."

# Cryptography and SRTP are too slow in debug builds, so this always builds release regardless of the app's configuration.
for platform in $platforms; do
    case $platform in
        iphoneos) target=aarch64-apple-ios ;;
        iphonesimulator) target=aarch64-apple-ios-sim ;;
        *) echo "unsupported platform: $platform" >&2; exit 1 ;;
    esac

    cargo build -q --release --lib --target "$target"
    mkdir -p "$out/$platform"
    cp "target/$target/release/libnaru.a" "$out/$platform/"
    library="target/$target/release/libnaru.a"
done

# The bindings are generated from the metadata of the library just built.
rm -rf "$out/swift" "$out/headers"
cargo run -q --example uniffi-bindgen -- generate --library "$library" -l swift -o "$out/swift" -n
mkdir -p "$out/headers"
mv "$out/swift/NaruFFI.h" "$out/headers/"
mv "$out/swift/NaruFFI.modulemap" "$out/headers/module.modulemap"
