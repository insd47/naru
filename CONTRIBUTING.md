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
| `windows/` | The Windows app (WinUI 3, Windows 11)                                      |
| `tools/`   | `layouts.swift`, which generates the Mac keyboard layout tables            |
| `licenses/`| Templates for the core's third-party notices, and the notices generated   |
| `docs/`    | Design notes for maintainers                                               |

[`docs/architecture.md`](docs/architecture.md) describes how the core is put together.

## Setup

The Rust toolchain is pinned in `rust-toolchain.toml`; `rustup` installs it on the first build.

- **iPad app**: Xcode 27 and `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`. Open `ios/Naru.xcodeproj`.
  A build phase runs `ios/build-core.sh`, which builds the core as a static library for the selected platform and
  generates the Swift bindings into `ios/build/generated`.
- **Android app**: Android Studio or the Android SDK with an NDK, `cargo install cargo-ndk`, and
  `rustup target add aarch64-linux-android`. Gradle's `preBuild` depends on tasks in `android/app/build.gradle.kts`
  that build `libnaru.so` and generate the Kotlin bindings into `android/app/build/generated`.
- **Windows app**: Visual Studio 2026 with the .NET desktop and WinUI application development workloads and the MSVC
  C++ build tools, and the C# binding generator:
  `cargo install uniffi-bindgen-cs --git https://github.com/NordSecurity/uniffi-bindgen-cs --tag v0.11.1+v0.31.0`.
  For ARM64 builds, also `rustup target add aarch64-pc-windows-msvc` and the ARM64 MSVC build tools, then build with
  `-p:Platform=ARM64`. Open
  `windows/Naru/Naru.csproj` or run `dotnet build` there. Targets in the project build the core, generate the C#
  bindings into `obj/`, and package the core as `naru_ffi.dll`, because `Naru.dll` is the app's own assembly.

All apps build the core in release mode even for debug builds, because the cryptography and SRTP are too slow
unoptimized. Generated bindings are never committed.

## Local checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

When `Cargo.lock` changes, regenerate the core's third-party notices and commit them with it. CI fails if they are out
of date or a dependency brings a license `about.toml` does not accept.

```sh
cargo install cargo-about --locked --version 0.9.2 --features cli
cargo about generate --frozen --fail licenses/text.hbs -o licenses/notices/core.txt
cargo about generate --frozen --fail licenses/plist.hbs -o ios/Naru/Settings.bundle/Acknowledgements.plist
```

The Windows app builds its `ThirdPartyNotices.txt` from these notices and the licenses of the packages it ships. A
package licensed only by an SPDX expression takes its text from `licenses/`, and the build fails when that text is
missing.

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
- Other clients can share the Mac with you: an HEVC session seems to allow no second HEVC session, while H.264
  sessions join alongside other sessions and get the existing virtual display's size. The Apple Screen Sharing app
  also holds UDP ports 5900 and 5901 while it is connected. Quit other clients before testing.

## Issues and pull requests

- Bug reports go through the issue form, with the diagnostics each app exports from Help or Settings. Security
  problems go through [SECURITY.md](SECURITY.md) instead.
- Open an issue before a large change, so the design can be agreed first.
- Keep a pull request to one concern, run the local checks, build every app you touched, and say how you tested it:
  offline only, or against which macOS version and app.
- Never attach screen captures, or logs and recordings that show personal data.

## Style

- Comments, docs, log and error messages, and commit messages are written in English.
- Each app logs with its platform's logger (`os.Logger`, `android.util.Log`, `Log` on Windows), and users export that
  log from Help or Settings for bug reports. Log what happened (stages, failures, codec, layout, focus, input language,
  the Mac input source selected), never what was typed: no passwords, keys, typed characters or key codes, or
  clipboard contents.
- Commit subjects follow [Conventional Commits](https://www.conventionalcommits.org/): `type: subject`, lowercase and
  imperative, one concern per commit.
- Comments explain what the code cannot show: a contract, a constraint, or the reason for a choice that looks wrong.
  They do not document the wire format; the code is the reference for that.
- Prefer matching Apple's Screen Sharing client over inventing behavior it does not have.
