#!/bin/sh
# Rust 코어 크레이트를 iOS용 정적 라이브러리로 빌드하고 Swift 바인딩을 만듭니다. Xcode의 빌드 단계가 부릅니다.
#
# - Xcode가 넘기는 PLATFORM_NAME(iphoneos, iphonesimulator)의 라이브러리만 빌드합니다. 없으면 둘 다 빌드합니다.
# - 결과는 build/generated 아래에 두며 저장소에 올리지 않습니다. 프로젝트는 그 경로의 라이브러리와 헤더를 씁니다.
# - 필요한 것: rustup 대상 aarch64-apple-ios, aarch64-apple-ios-sim
set -eu

here=$(cd "$(dirname "$0")" && pwd)
out="$here/build/generated"
platforms=${PLATFORM_NAME:-"iphoneos iphonesimulator"}
export PATH="$HOME/.cargo/bin:$PATH"

# Xcode가 넘기는 iOS SDK 설정이 남아 있으면 cargo가 빌드 스크립트를 iOS용으로 링크하려다 실패합니다.
unset SDKROOT IPHONEOS_DEPLOYMENT_TARGET

cd "$here/.."

# 암호 연산과 SRTP가 debug 빌드에서는 너무 느리므로 앱의 빌드 종류와 관계없이 release로 빌드합니다.
for platform in $platforms; do
    case $platform in
        iphoneos) target=aarch64-apple-ios ;;
        iphonesimulator) target=aarch64-apple-ios-sim ;;
        *) echo "지원하지 않는 플랫폼입니다: $platform" >&2; exit 1 ;;
    esac

    cargo build -q --release --lib --target "$target"
    mkdir -p "$out/$platform"
    cp "target/$target/release/libnaru.a" "$out/$platform/"
    library="target/$target/release/libnaru.a"
done

# 바인딩은 방금 만든 라이브러리의 메타데이터에서 만듭니다.
rm -rf "$out/swift" "$out/headers"
cargo run -q --example uniffi-bindgen -- generate --library "$library" -l swift -o "$out/swift" -n
mkdir -p "$out/headers"
mv "$out/swift/NaruFFI.h" "$out/headers/"
mv "$out/swift/NaruFFI.modulemap" "$out/headers/module.modulemap"
