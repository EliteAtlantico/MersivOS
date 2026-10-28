# Loop

A Humanistic-Intelligence mixed-reality environment for the Valve Steam Frame. It runs as a session on stock SteamOS, built on Stardust XR.

- Design spec: `docs/SPEC.md`
- Mockups: `design/renders/` (overview, look-ahead, glance-down, design-language sheet)
- Mockup scene (Blender 5.2): `design/blender/loop_mockup.blend`
- PC prototype: Rust crates in `crates/`; Stardust and liblsl are cloned into `upstream/` (not tracked)

| Crate | What it is |
|---|---|
| `loop-ring` | The ring, tiles, status arc, raise / return / pin (a Stardust XR client). Also fuses intent (gaze target + any confirm) and runs the adaptation policy. |
| `loop-sense` | Reads EEG over LSL: signal quality per channel, focus against your baseline, jaw-clench and double-blink confirm, inline calibration. `--sim` runs a synthetic Muse-class band. |
| `loop-record` | Mirrors bus events to an LSL marker stream ("Loop events"); records every LSL stream from this machine to `~/.local/share/loop/sessions/*.xdf` when asked. |
| `loop-bus` | The hub for Loop's events (JSON lines over a Unix socket). `loop-bus tap` prints them; `loop-bus send JSON` sends one. |
| `loop-lsl` | The parts of liblsl Loop uses, loaded at run time. |

## Run the PC prototype

One-time setup. `upstream/` is not in git: the Stardust server, asteroids (a path dependency of `loop-ring`) and liblsl 1.17.7 are cloned there at the commits Loop was built against:

```bash
git clone https://github.com/StardustXR/server upstream/server && git -C upstream/server checkout 75f8e3078726
git clone https://github.com/StardustXR/asteroids upstream/asteroids && git -C upstream/asteroids checkout 05aea7b04360
git clone --depth 1 --branch v1.17.7 https://github.com/sccn/liblsl upstream/liblsl
cmake -S upstream/liblsl -B upstream/liblsl/build -DCMAKE_BUILD_TYPE=Release && cmake --build upstream/liblsl/build -j
```

Then:

```bash
scripts/dev.sh           # synthetic headband
scripts/muse.sh          # list nearby Muse headbands (OpenMuse, from the muse-motor conda env)
scripts/muse.sh ADDRESS  # stream one on LSL, then in another terminal:
scripts/dev.sh --muse    # read the Muse (or any EEG stream on LSL)
```

In the Stardust window (flatscreen mode) the mouse pointer stands in for gaze:

| Input | Does |
|---|---|
| Point at a tile or word | Focus (white outline; cyan ring = listening for a clench or double blink) |
| Left click, Space or Enter | Confirm |
| Jaw clench, double blink | Confirm, when the band can hear it (the cyan ring shows) |
| Backspace | Undo |
| Home | Recall the ring |
| C / B | Make the synthetic band clench / double-blink |
| Shift + right-drag | Look around |

Keys reach Loop only while the pointer is on one of its targets (Stardust gives keyboard focus to whatever the pointer hits). On the Signals panel, **Record** starts a session recording (a coral "Recording" pill appears on the arc; confirm it to stop) and **Calibrate** learns your clench ("Clench twice, firmly").

## Tests

```bash
cargo test                                                         # unit tests; LSL round trip needs upstream/liblsl built
cargo test --release -p loop-sense stress -- --ignored --nocapture # detector numbers over 2 h of synthetic signal
```

Unattended screenshots (Hyprland; the window opens on a hidden workspace), driving the ring and the synthetic band with timed actions. Everything on the bus is logged to `OUT_DIR/bus.log`:

```bash
scripts/shots.sh /tmp/loop-shots "2:hover 1; 3:clench; 6:hover 2; 7:blink2; 9:record on" 4.5 8.5 11
```

`LOOP_SCRIPT` commands: `press N`, `pin N`, `return N`, `undo`, `hover N`, `move X Z`, `recall`, `reverse`, `key`, `clench`, `blink`, `blink2`, `focus X` / `focus drift`, `contact CH lost|good`, `record on|off`, `calibrate`.

Regenerate the mockups and the runtime assets:

```bash
cd design
blender -b --factory-startup --python scripts/scene.py -- --out renders/passes --save-blend $PWD/blender/loop_mockup.blend
~/miniforge3/bin/python scripts/composite.py renders/passes renders
~/miniforge3/bin/python scripts/sheet.py renders/passes/ahead_room.png renders/loop_design_language.png
blender -b --factory-startup --python scripts/assets.py          # tile/pill/glyph meshes → crates/loop-ring/res/loop
blender -b --factory-startup --python scripts/scene.py -- --out renders/passes --views sky --res 4096x2048
```

The passthrough panorama (`res/loop/passthrough.png`) is `renders/passes/sky_room.png` in grayscale with camera grain, rolled half a turn so its centre faces Stardust's forward.

Font: Atkinson Hyperlegible (Braille Institute, SIL OFL 1.1, `assets/fonts/OFL.txt`).
