#!/usr/bin/env bash
set -m

case "$1" in
*boundary*)
  DICT=dict/boundary
  ;;
*header*)
  DICT=dict/headers
  ;;
*whole*)
  DICT=dict/whole
  ;;
*string*)
  DICT=dict/read-to-string
  ;;
*)
  echo unknown fuzzing target $1
  exit 1
  ;;
esac


cd fuzz
env RUSTFLAGS='-C opt-level=0' cargo afl build || exit 1
OUTPUT="output-$1"

rm -rf $OUTPUT
mkdir $OUTPUT

if [[ -n "$PARALLEL_FUZZ" ]]; then
  # trap Ctrl-C and others and pass to all child jobs
  trap "trap - SIGTERM && kill -- -$$" SIGINT SIGTERM EXIT
  # 4 comment lines preceed the output minus one extra for the master
  NUM_CPUS=`expr $(lscpu -p | wc -l) - 4`
  echo detected num cpus: $NUM_CPUS
  cargo afl fuzz -i $DICT -o $OUTPUT -M "$1_master" target/debug/$1 &
  for ((i=0;i<$NUM_CPUS-1;i++)); do
    NAME="$1_slave_$i"
    cargo afl fuzz -i $DICT -o $OUTPUT -S "$1_slave_$i" target/debug/$1 > "$OUTPUT/slave-$i" &
  done
  fg %1
else
  cargo afl fuzz -i $DICT -o $OUTPUT target/debug/$1
fi
