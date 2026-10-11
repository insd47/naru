#!/bin/sh
# Rust 프로토콜 크레이트를 Android용으로 빌드하고 Kotlin 바인딩을 만듭니다. Gradle의 preBuild가 부릅니다.
#
# - 결과는 app/build/generated 아래에 두며 저장소에 올리지 않습니다.
# - 필요한 것: rustup 대상 aarch64-linux-android, cargo-ndk, Android NDK
set -eu

here=$(cd "$(dirname "$0")" && pwd)
out="$here/app/build/generated"
export PATH="$HOME/.cargo/bin:$PATH"

cd "$here/../protocol"

# 암호 연산과 SRTP가 debug 빌드에서는 너무 느리므로 앱의 빌드 종류와 관계없이 release로 빌드합니다.
cargo ndk -t arm64-v8a -P 29 -o "$out/jniLibs" build --release

# 바인딩은 방금 만든 라이브러리의 메타데이터에서 만듭니다.
rm -rf "$out/uniffi"
cargo run -q --features cli --bin uniffi-bindgen -- \
    generate "$out/jniLibs/arm64-v8a/libprotocol.so" -l kotlin -o "$out/uniffi" -n
