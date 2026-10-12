# Contributing

Naru is one Rust core shared by native apps. The core speaks the protocol and hands the apps decoded-ready media and
events through [UniFFI](https://mozilla.github.io/uniffi-rs/); each app owns decoding, rendering, input capture, and its
own UI.

| Path       | What                                                                       |
|------------|----------------------------------------------------------------------------|
| `src/`     | The `naru` crate: connection, authentication, media, input, UniFFI surface |
| `examples` | `probe`, a diagnostic client, and `uniffi-bindgen`, the binding generator  |
| `tests/`   | An end-to-end test against a fake server on loopback                       |
| `ios/`     | The iPad app (SwiftUI, iPadOS 26)                                          |
| `android/` | The Android tablet app (Jetpack Compose, API 29+)                          |

[`.docs/architecture.md`](.docs/architecture.md) describes how the core is put together.

## Setup

The Rust toolchain is pinned in `rust-toolchain.toml`; `rustup` installs it on the first build.

- **iPad app**: Xcode 27 and `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`. Open `ios/Naru.xcodeproj`.
  A build phase runs `ios/build-core.sh`, which builds the core as a static library for the selected platform and
  generates the Swift bindings into `ios/build/generated`.
- **Android app**: Android Studio or the Android SDK with an NDK, `cargo install cargo-ndk`, and
  `rustup target add aarch64-linux-android`. Gradle's `preBuild` runs `android/build-core.sh`, which builds `libnaru.so`
  and generates the Kotlin bindings into `android/app/build/generated`.

Both scripts build the core in release mode even for debug apps, because the cryptography and SRTP are too slow
unoptimized. Generated bindings are never committed.

## Local checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

For app changes, also build the app you touched. The Android JVM tests run with
`./gradlew :app:testDebugUnitTest` from `android/`.

## Testing against a Mac

Unit tests and `tests/loopback.rs` run offline. Anything beyond them needs a real Mac with Screen Sharing enabled.

```sh
# Check the version, security types, and server key without signing in.
cargo run --example probe -- my-mac.local --handshake-only

# Sign in, receive media for 30 seconds, and save the video.
cargo run --example probe -- my-mac.local myname --display 1280x800@2 --seconds 30 --out capture.h264
```

> Captures contain whatever was on the Mac's screen. Keep them out of the repository; `.gitignore` excludes `*.h264`.

- `probe` reads the password from `NARU_PASSWORD` or asks for it. It never prints passwords, keys, or clipboard
  contents, and neither may any log line you add.
- Only one High Performance session can run per Mac, and the Apple Screen Sharing app holds UDP ports 5900 and 5901
  while it is connected. Quit other clients before testing.

## Style

- Comments, docs, log and error messages, and commit messages are written in English.
- Commit subjects follow [Conventional Commits](https://www.conventionalcommits.org/): `type: subject`, lowercase and
  imperative, one concern per commit.
- Comments explain what the code cannot show: a contract, a constraint, or the reason for a choice that looks wrong.
  They do not document the wire format; the code is the reference for that.
- Prefer matching Apple's Screen Sharing client over inventing behavior it does not have.
