#!/usr/bin/env bash
# PC prototype: the loop bus, loop-sense, loop-record, the Stardust server in flatscreen mode, and
# loop-ring.
#   scripts/dev.sh           with the synthetic headband
#   scripts/dev.sh --muse    with a real headband (start scripts/muse.sh first), or any EEG on LSL
# The mouse pointer stands in for gaze. Left click, Space or Enter confirms; Backspace undoes; Home
# recalls the ring. With the synthetic headband, C clenches and B double-blinks. Keys reach Loop
# while the pointer is on one of its targets. Shift + right-drag looks around.
# Extra arguments go to the server (e.g. --spectator).
set -euo pipefail
cd "$(dirname "$0")/.."
sense=(--sim)
if [[ ${1:-} == --muse ]]; then
	sense=()
	shift
fi
cargo build -q
(cd upstream/server && cargo build -q)
pids=()
trap 'kill "${pids[@]}" 2>/dev/null; wait' EXIT
target/debug/loop-bus &
pids+=($!)
sleep 0.3
target/debug/loop-sense "${sense[@]}" &
pids+=($!)
target/debug/loop-record &
pids+=($!)
upstream/server/target/debug/stardust-xr-server -f "$@" &
pids+=($!)
sleep 4
target/debug/loop-ring
