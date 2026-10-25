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
- Video arrives as a `Decoder` config followed by `Frame` access units. The config holds the parameter sets, the
  picture size, and whether the colors are Display P3, and the units never repeat the parameter sets. A Mac sends
  Display P3; an app whose decoder does not tag its pictures with the stream's colors converts them, or they look
  washed out. The app recreates its decoder on every new config, and
  calls `request_keyframe` once whenever it drops frames or rebuilds the decoder on its own. The session repeats the
  request until a keyframe arrives.
- HEVC comes in four horizontal bands, as it does to Apple's client. Each frame is a picture of one band, numbered from
  the top; the app stacks them, keeps a band's last picture until a new one replaces it, and shows the screen at each
  `Present`, which follows the last band of an instant. While any band's latest picture is lost or broken, no
  `Present` comes, so the screen never pairs bands from different moments. H.264 is a single band and has no `Present`.
- Frames, `Present`, and audio come at the Mac's own pace. When the network holds packets back and lets them through
  together, as Wi-Fi does every half second while AWDL is busy, the session delays everything by the same amount and
  sends them a frame apart, as Apple's client shows them a beat late instead of skipping. The delay drops once arrivals
  are on time again. The app shows and plays what it gets as it gets it.
- `Cursor` is the Mac's cursor shape. `CursorHidden` hides it, as the Mac does while typing or over playing video,
  until the next `Cursor`, which the session sends again once the pointer moves even when the shape did not change.
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

`session` drives one connection attempt. When media does not start within about ten seconds, the session ends with
`NoMedia`, as Apple's client gives up on a High Performance connection. The one retry is for HEVC that brought no video
at all: the session connects again from scratch with H.264.

Within an attempt the order is fixed: UDP sockets are opened first so the local firewall accepts the Mac's media, then
the TCP handshake, sign-in (`auth`), and the switch to the encrypted record layer (`record`). After that the `Engine`
runs one loop over three inputs: control messages from the Mac, UDP datagrams, and commands from the app.

- `negotiation` sends the media offer whenever the Mac asks for one: at the start, after a display configuration, and after its display wakes.
- `media` decrypts SRTP, picks the active group of video SSRCs, one per band (`media::Streams`), and reassembles access
  units in decoding order (`media::Group`). The Mac does not retransmit, so a gap waits only for late packets. H.264
  then keeps going without the lost unit and asks for a keyframe. For HEVC it reads each picture's reference set
  (`media::References`) to tell which pictures decode intact, as Apple's client does: it acknowledges (LTR) only intact
  pictures, which tells the Mac what it may predict from, and asks for a refresh about once a frame while packets go
  missing. Every picture still goes to the decoder, marked `AccessUnit::intact`, and the screen holds while any band
  is broken until the Mac answers with pictures predicted from acknowledged ones, so it recovers whole and without a
  keyframe.
- Every 50 ms `media` also sends the receiver feedback Apple's client sends (`RCTL`): what arrived, and the bandwidth
  it allows. The Mac raises the bitrate from its 20.8 Mbps floor only on this feedback, and keeps it at or below that
  bandwidth. `media::Bandwidth` estimates it from the trend of one-way delay and from heavy loss, as Apple's client
  appears to: the 60 Mbps cap on a clear path, near the bottleneck's rate within seconds when a queue builds, and
  unchanged by random loss or a fixed delay.
- The engine watches for stalled video and asks for a keyframe, or tells the app to rebuild its decoder, only while the
  Mac is actually sending.
- `input` turns pointer, scroll, momentum, and magnify input into protocol events. Keys go out unchanged.

Modules other than `session` and `ffi` do no I/O. They take bytes and time as arguments and return values, which keeps
them testable without a network. Every length read from the server is bounded before use, and malformed input returns
an error instead of panicking.

## Keys on each platform

Every app sends, per key, what the user means to type, with the physical key as a USB HID usage. Shortcuts (Command,
Control, or Option held) send the Latin key itself. What differs is what each system hands the app:

| | Gets | Sends |
|---|---|---|
| Android | the physical key's Latin character and Linux key code, and the IME's language | `character(latin, language)`, or the Latin character |
| Windows | the Windows layout's character and scan code, and a Korean/English mode tracked by the app | `character(char, language)`, or the character |
| iPad | the character iPadOS typed in its input language, and the key's HID usage | `character(physical Latin, language)` for languages with a Mac layout, otherwise iPadOS's character |

Android and Windows turn their key codes into HID usages with the standard tables; the core turns a usage into the
Mac's key code, and a modifier's from its keysym, since a PC's Alt goes as Command.

Each app also reports its input language with `set_input_language`, as soon as it changes, from the system's notice
where there is one, as on the iPad, where it comes a moment after the switch key, and before keys typed in it, which
catches a late notice. `set_sync_keyboard_language` follows Apple's "Sync Keyboard Language". With it on, the core asks
the Mac for the source of the language, once the Mac has reported its own, which picks the mode of a language
(Sebeolsik, Kana typing). It asks only for a source other than the last one, so a source the Mac changes to by itself,
as when another window takes focus, is left alone until the client's language changes, as Apple's client leaves it.
Turning the option off releases the source.

The agent applies a requested source only at the next key event, and when that event is a typed key it often reads the
old source's layout and keeps it. Apple's client applies the source at the release of its switch key instead. The core
does not depend on any switch key: it releases the agent's source, as Apple's client does when the option turns off,
asks for the new one, and applies it at once with a key that types nothing, released after the switch, while keys typed
meanwhile wait for the Mac to report the new source, a few milliseconds.

Switching also commits a composition the Mac's input method still holds; without the option, Korean typed before
switching to English stays composing until Space. Like Apple's client, keys go with their macOS key code while the
Mac is on the source of the client's language, so the agent presses the physical key when a character is missing from
the layout it holds. With the option on, the source last asked for counts as the Mac's, since keys wait for the switch;
with it off, the source the Mac reported does. On another source keys go without one, and the Mac types the character
the client chose. After a switch the agent sometimes still keeps the old source's layout, mostly right after connecting
or after another window took focus, and the Mac then types the key's Latin letter until a text field takes focus again,
the same as with Apple's client; without the key code it typed separate jamo.

iPadOS applies a hardware keyboard language switch only when its text system sees the next key, and that key's
`UIKey.characters` is still in the old language. The iPad app therefore passes typing keys to the text system first and
reads the input language afterwards, which is already the new one.

## Momentum on each platform

No platform reports momentum after a scroll ends; each system's own scrolling views decelerate it themselves. The apps
do what those views do, so a fling feels the way it does elsewhere on the device:

| | Velocity | Deceleration | Stops on |
|---|---|---|---|
| iPad | `UIPanGestureRecognizer.velocity(in:)` | `UIScrollView.DecelerationRate.normal`, per display frame | fingers resting on the trackpad, any touch, scroll, wheel, or pinch |
| Android | `VelocityTracker` on the swipe's touches | `OverScroller` with the system friction, per `Choreographer` frame | the touchpad's fling-stop signal, any touch, wheel, or click |
| Windows | `InteractionTracker` inertia for the touchpad, `GestureRecognizer` inertia for touch | the system's, per frame | any touch, touchpad contact, wheel, or click |

## Keyboard layout tables

`src/keyboard/tables.rs` is generated on a Mac from the system keyboard layouts:

```sh
swift tools/layouts.swift > src/keyboard/tables.rs
```

Regenerate it when adding a language. The table only covers layouts that cannot type Latin letters; Latin layouts send
characters as they are.
