# Agent Instructions

Read these before changing anything:

- [CONTRIBUTING.md](CONTRIBUTING.md): layout, setup, local checks, testing against a Mac, and style.
- [.docs/architecture.md](.docs/architecture.md): how the core and the apps divide the work.

On top of those:

- Never push or publish anything without the owner's go-ahead.
- Run `probe` only with `--handshake-only`. Signed-in sessions against a real Mac are run by the owner.
- Never log or print passwords, keys, or clipboard contents.
- Keep `.plan/` and screen captures out of commits.
- Write everything committed in English. Commit subjects are `type: subject`, lowercase and imperative.
