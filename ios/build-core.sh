#!/bin/sh
# Rust 코어 크레이트를 iOS용 정적 라이브러리로 빌드하고 Swift 바인딩과 XCFramework를 만듭니다. Xcode의 빌드 단계가 부릅니다.
#
# - 결과는 build/generated 아래에 두며 저장소에 올리지 않습니다.
# - 필요한 것: rustup 대상 aarch64-apple-ios, aarch64-apple-ios-sim
set -eu

here=$(cd "$(dirname "$0")" && pwd)
out="$here/build/generated"
export PATH="$HOME/.cargo/bin:$PATH"

# Xcode가 넘기는 iOS SDK 설정이 남아 있으면 cargo가 빌드 스크립트를 iOS용으로 링크하려다 실패합니다.
unset SDKROOT IPHONEOS_DEPLOYMENT_TARGET

cd "$here/.."

# 암호 연산과 SRTP가 debug 빌드에서는 너무 느리므로 앱의 빌드 종류와 관계없이 release로 빌드합니다.
cargo build -q --release --lib --target aarch64-apple-ios
cargo build -q --release --lib --target aarch64-apple-ios-sim

# 바인딩은 방금 만든 라이브러리의 메타데이터에서 만듭니다.
rm -rf "$out"
cargo run -q --example uniffi-bindgen -- \
    generate --library target/aarch64-apple-ios/release/libnaru.a -l swift -o "$out/swift" -n

mkdir -p "$out/headers"
mv "$out/swift/NaruFFI.h" "$out/headers/"
mv "$out/swift/NaruFFI.modulemap" "$out/headers/module.modulemap"

xcodebuild -create-xcframework \
    -library target/aarch64-apple-ios/release/libnaru.a -headers "$out/headers" \
    -library target/aarch64-apple-ios-sim/release/libnaru.a -headers "$out/headers" \
    -output "$out/Naru.xcframework" > /dev/null
