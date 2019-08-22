#!/usr/bin/env bash

cd fuzz
env RUSTFLAGS='-C opt-level=0' cargo afl build || exit 1
cargo afl fuzz -i dict -o output target/debug/$1
