#!/usr/bin/env bash
# A Muse headband on LSL through OpenMuse (installed in the muse-motor conda env of
# ~/Projects/muse-alpha-motor). Force-close the Muse phone app first: a band talks to one app.
#   scripts/muse.sh            list nearby headbands
#   scripts/muse.sh ADDRESS    stream one until Ctrl-C (EEG, motion, PPG, battery)
set -euo pipefail
openmuse=${OPENMUSE:-$HOME/miniforge3/envs/muse-motor/bin/OpenMuse}
if [[ $# -eq 0 ]]; then
	exec "$openmuse" find
fi
exec "$openmuse" stream --address "$1"
