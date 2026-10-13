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

## The principle

The core stands for what the Mac receives. Its input and output are the same on every platform, and it exposes nothing
specific to one platform unless that is the natural shape of the data. Making a platform's input match what the Mac
expects is that platform's job, done the way that platform normally does it.

For keys, the Mac receives the character to type: the agent looks it up in the Mac's current layout and presses that
key. The Mac stays on the user's native-language input method, so a Korean Mac must receive `ㄱ`, not `r`, for its
input method to compose. Each app therefore sends what the user means to type, and the core sends it as is.

## The boundary

An app opens a session with `Session::connect(config, listener)` and gets events on `SessionListener::on_event`.

- `connect` returns at once. Every session ends with exactly one `Closed` event, whether it failed, the user closed it,
  or the Mac went away.
- Events arrive in order on a thread dedicated to that session. A slow listener slows the session down instead of
  letting frames pile up in memory, so listeners hand heavy work to another thread.
- Video arrives as a `Decoder` config followed by `Frame` access units. The app recreates its decoder on every new
  config, and calls `request_keyframe` whenever it drops frames or rebuilds the decoder.
- Pointer coordinates are in the pixels of the latest `Layout`. Keys are X11 keysyms of what the Mac should type, and
  a release repeats the keysym of its press. The Mac does not repeat held keys, so key repeat is the app's to send.
- Scrolls follow a Mac trackpad's phases: `MayBegin` when fingers rest, `Began` with the first movement, `Changed`,
  and `Ended` when they lift, or `Cancelled` if they never moved. Momentum is the app's: after `Ended`, it sends
  `momentum` frames as its own system decelerates the scroll, and ends them before any new input. Mouse wheels have
  neither phases nor momentum.
- `Touchpad` turns fingers on a screen into trackpad gestures (cursor moves, clicks, scroll, magnify). It knows nothing
  of the session: the app moves its own cursor, clamped to the screen, and sends the gestures like any other pointer
  and scroll.
- `character(latin, language, source)` is a pure function over the Mac's keyboard layouts: what a Latin key types in a
  language, given the Mac's input source from the latest `Source` event. It follows the Mac's secure input (Latin) and
  its own layout of that language (Sebeolsik, Kana typing), and returns nothing for languages typed in Latin.
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
- `input` turns pointer, scroll, momentum, and magnify input into protocol events. Keys go out unchanged.

Modules other than `session` and `ffi` do no I/O. They take bytes and time as arguments and return values, which keeps
them testable without a network. Every length read from the server is bounded before use, and malformed input returns
an error instead of panicking.

## Keys on each platform

Every app sends, per key, what the user means to type. Shortcuts (Command, Control, or Option held) send the Latin key
itself. What differs is what each system hands the app:

| | Gets | Sends |
|---|---|---|
| Android | the physical key's Latin character, and the IME's language | `character(latin, language)`, or the Latin character |
| Windows | the Windows layout's character, and a Korean/English mode tracked by the app | `character(char, language)`, or the character |
| iPad | the character iPadOS typed in its input language | `character(physical Latin, language)` for languages with a Mac layout, otherwise iPadOS's character |

iPadOS applies a hardware keyboard language switch only when its text system sees the next key, and that key's
`UIKey.characters` is still in the old language. The iPad app therefore passes typing keys to the text system first and
reads the input language afterwards, which is already the new one.

## Momentum on each platform

No platform reports momentum after a scroll ends; each system's own scrolling views decelerate it themselves. The apps
do what those views do, so a fling feels the way it does elsewhere on the device:

| | Velocity | Deceleration | Stops on |
|---|---|---|---|
| iPad | `UIPanGestureRecognizer.velocity(in:)` | `UIScrollView.DecelerationRate.normal`, per display frame | any touch, scroll, wheel, or pinch |
| Android | `VelocityTracker` on the swipe's touches | `OverScroller` with the system friction, per `Choreographer` frame | the touchpad's fling-stop signal, any touch, wheel, or click |
| Windows | `InteractionTracker` inertia for the touchpad, `GestureRecognizer` inertia for touch | the system's | a new interaction |

## Keyboard layout tables

`src/keyboard/tables.rs` is generated on a Mac from the system keyboard layouts:

```sh
swift layouts.swift > src/keyboard/tables.rs
```

Regenerate it when adding a language. The table only covers layouts that cannot type Latin letters; Latin layouts send
characters as they are.
