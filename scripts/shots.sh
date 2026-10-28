#!/usr/bin/env bash
# Unattended screenshots of the PC prototype, with the synthetic headband, on a private bus.
# Hyprland only: the server window opens on a hidden workspace and is captured by toplevel id.
# Everything on the bus goes to OUT_DIR/bus.log; recordings go under OUT_DIR/data.
#   scripts/shots.sh OUT_DIR "3:hover 1; 6:clench" 2.5 5 8
set -euo pipefail
cd "$(dirname "$0")/.."
out=$(realpath -m "$1") script=$2
shift 2
mkdir -p "$out"
cargo build -q
rm -f /tmp/asteroids_config/org.loop.ring.ron
pkill -x loop-ring || true
pkill -x stardust-xr-ser || true
sleep 1
export LOOP_BUS=$out/bus.sock XDG_DATA_HOME=$out/data
pids=()
trap 'pkill -x loop-ring; pkill -x stardust-xr-ser; kill "${pids[@]}" 2>/dev/null; wait' EXIT
target/debug/loop-bus 2>"$out/hub.log" &
pids+=($!)
sleep 0.3
target/debug/loop-bus tap >"$out/bus.log" &
pids+=($!)
target/debug/loop-sense --sim 2>"$out/sense.log" &
pids+=($!)
target/debug/loop-record 2>"$out/record.log" &
pids+=($!)
# Opened on a hidden workspace through Hyprland, so the window never lands on the screen in use.
hyprctl -q dispatch exec "[workspace special:loop-shots silent] env RUST_LOG=warn $PWD/upstream/server/target/debug/stardust-xr-server -f >'$out/server.log' 2>&1"
sleep 6
LOOP_SCRIPT="$script" RUST_LOG=warn target/debug/loop-ring >"$out/ring.log" 2>&1 &
id=$(hyprctl clients -j | python3 -c "import json,sys; print(next(c['stableId'] for c in json.load(sys.stdin) if 'Stardust' in c['title']))")
prev=0
for t in "$@"; do
	sleep "$(python3 -c "print(max(0, $t - $prev))")"
	prev=$t
	grim -T "$id" "$out/at_$t.png"
done
