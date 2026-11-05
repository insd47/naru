# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [0.2.0](https://github.com/insd47/naru/compare/v0.1.1...v0.2.0) - 2026-11-05

### All

#### Fixed
- count clicks in the core so double clicks reach the mac ([#30](https://github.com/insd47/naru/pull/30))
- ask for a keyframe when hevc loses one before the first whole screen ([#32](https://github.com/insd47/naru/pull/32))
- tell a quick two-finger scroll from a pinch when fingers report one at a time ([#35](https://github.com/insd47/naru/pull/35))
- keep the session when the network answers a media packet with an icmp error ([#31](https://github.com/insd47/naru/pull/31))

### iPad

#### Fixed
- keep a trackpad pinch from cancelling the click under it ([#34](https://github.com/insd47/naru/pull/34))
## [0.1.1](https://github.com/insd47/naru/compare/v0.1.0...v0.1.1) - 2026-10-25

### All

#### Added
- blur the last picture while the mac's display resizes ([#25](https://github.com/insd47/naru/pull/25))
- toggle the mac's dock features as screen sharing does ([#18](https://github.com/insd47/naru/pull/18))

#### Fixed
- send the media offer when the mac asks for it ([#26](https://github.com/insd47/naru/pull/26))
- leave the unpublished core out of its third-party notices ([#28](https://github.com/insd47/naru/pull/28))

### iPad

#### Fixed
- size the first display below a title bar that stays ([#23](https://github.com/insd47/naru/pull/23))

### Android

#### Added
- add the session title bar ([#19](https://github.com/insd47/naru/pull/19))

#### Changed
- shrink the release build with r8 ([#17](https://github.com/insd47/naru/pull/17))

## [0.1.0](https://github.com/insd47/naru/releases/tag/v0.1.0) - 2026-10-24

Initial release.
