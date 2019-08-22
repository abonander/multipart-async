#!/usr/bin/env bash

cd fuzz
env RUSTFLAGS='-C opt-level=0' cargo afl build
cargo afl fuzz -i dict -o output target/debug/$1
