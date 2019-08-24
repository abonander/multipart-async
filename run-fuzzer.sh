#!/usr/bin/env bash

case $1 in
*boundary*)
  DICT=dict/boundary
  ;;
*header*)
  DICT=dict/headers
  ;;
*)
  echo unknown fuzzing target $1
  exit 1
  ;;
esac

cd fuzz
env RUSTFLAGS='-C opt-level=0' cargo afl build || exit 1
cargo afl fuzz -i $DICT -o output target/debug/$1
