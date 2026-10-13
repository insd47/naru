# Architecture

Naru splits the work at one line: the Rust core owns everything that is the same on every platform, and each app owns
what the platform does better. The core never decodes, renders, or touches platform input APIs; the apps never parse
protocol bytes.

```text
 app (Swift / Kotlin / C#)                    core (Rust)
┌───────────────────────┐   Config, input   ┌──────────────────────────────────────────┐
│ UI, hosts, keychain   │ ────────────────▶ │ ffi        UniFFI surface, event thread  │
│ hardware decoder      │                   │ session    connect, retry, engine loop   │
│ renderer, audio out   │ ◀──────────────── │ auth       RSA + SRP-6a sign-in          │
│ input capture         │   events          │ record     encrypted control channel     │
└───────────────────────┘                   │ message    control messages              │
                                            │ offer      media negotiation             │
                                            │ media      SRTP, RTP, access units       │
                                            └──────────────────────────────────────────┘
```

## The boundary

An app opens a session with `Session::connect(config, listener)` and gets events on `SessionListener::on_event`.

- `connect` returns at once. Every session ends with exactly one `Closed` event, whether it failed, the user closed it,
  or the Mac went away.
- Events arrive in order on a thread dedicated to that session. A slow listener slows the session down instead of
  letting frames pile up in memory, so listeners hand heavy work to another thread.
- Video arrives as a `Decoder` config followed by `Frame` access units. The app recreates its decoder on every new
  config, and calls `request_keyframe` whenever it drops frames or rebuilds the decoder.
- Pointer coordinates are in the pixels of the latest `Layout`. Keys are X11 keysyms; the core translates characters
  to the Mac's keyboard layout itself.
- The core runs its own small tokio runtime, so apps need no async runtime of their own.

## Inside a session

`session` drives one connection attempt at a time and retries from scratch when media never starts, up to three
attempts in total. When HEVC was requested and no video arrives, later attempts fall back to H.264.

Within an attempt the order is fixed: UDP sockets are opened first so the local firewall accepts the Mac's media, then
the TCP handshake, sign-in (`auth`), and the switch to the encrypted record layer (`record`). After that the `Engine`
runs one loop over three inputs: control messages from the Mac, UDP datagrams, and commands from the app.

- `negotiation` sends the media offer and resends it if the Mac does not answer.
- `media` decrypts SRTP, picks the active video stream (`media::Streams`), and reassembles access units
  (`media::Assembler`). Gaps are retransmitted when they can be, and otherwise trade latency for quality according
  to the `Priority` the user chose.
- The engine watches for stalled video and asks for a keyframe, or tells the app to rebuild its decoder, only while the
  Mac is actually sending.
- `input` turns app input into protocol events. Keyboard text goes through `keyboard`, which maps characters to the
  Mac's layout and handles Korean/English switching; finger touches go through a touchpad state machine that emulates
  a trackpad.

Modules other than `session` and `ffi` do no I/O. They take bytes and time as arguments and return values, which keeps
them testable without a network. Every length read from the server is bounded before use, and malformed input returns
an error instead of panicking.

## Keyboard layout tables

`src/session/keyboard/tables.rs` is generated on a Mac from the system keyboard layouts:

```sh
swift layouts.swift > src/session/keyboard/tables.rs
```

Regenerate it when adding a language. The table only covers layouts that cannot type Latin letters; Latin layouts send
characters as they are.
