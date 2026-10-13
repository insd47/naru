#!/bin/sh
# Regenerates the core's third-party notices from Cargo.lock: plain text for the Android and Windows apps, and the
# iPad's Settings acknowledgements. With --check, fails if the committed files are out of date instead of writing them.
#
# Needs the pinned cargo-about: cargo install cargo-about --locked --version 0.9.2 --features cli
set -eu
cd "$(dirname "$0")/.."

text=licenses/notices/core.txt
plist=ios/Naru/Settings.bundle/Acknowledgements.plist

if [ "${1:-}" = --check ]; then
    out=$(mktemp -d)
    trap 'rm -rf "$out"' EXIT
else
    out=
fi

for pair in "text.hbs $text" "plist.hbs $plist"; do
    set -- $pair
    target=${out:+$out/}$2
    mkdir -p "$(dirname "$target")"
    # Some crates' license files use CRLF; git stores LF, so the output does too.
    cargo about generate --frozen --fail -c licenses/about.toml "licenses/$1" | tr -d '\r' > "$target"

    if [ -n "$out" ] && ! cmp -s "$target" "$2"; then
        echo "$2 is out of date; run licenses/generate.sh" >&2
        exit 1
    fi
done
