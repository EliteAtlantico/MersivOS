# Loop — design spec

Draft v0.1 · 2026-10-23 · status: for review (renders in `design/renders/`)

Loop is a mixed-reality operating environment for the Valve Steam Frame. It replaces the headset's own interface with one built on Steve Mann's Humanistic Intelligence: the wearer and the computer form a single feedback loop, the real world stays the base layer, and the system never takes more of your attention than you hand it. It is built for daily wear with brain and body sensing (EEG over LSL, consumer bands such as Muse, the Frame's own eye tracking), and it runs Android VR APKs.

---

## 1. Decisions so far

| Area | Decision |
|---|---|
| Hardware | No Steam Frame yet. Develop on the PC first (Monado / Stardust flatscreen), then bring up on the device. |
| OS layer | A replacement session on **stock SteamOS**. Valve's kernel, drivers, tracking, updates and Android layer stay underneath. To the wearer it is a whole new OS. |
| Philosophy | Mann's **Humanistic Intelligence** (HI), taken literally as system guarantees (§2). |
| Audience | Anyone, eventually. Full accessibility, safety and onboarding are in scope. |
| EEG hardware | **Anything over LSL** plus **consumer bands (Muse etc.)**. |
| EEG roles | Passive state sensing, active control (BCI), error-potential undo, research logging. |
| Other sensors | The Frame's **built-in eye tracking** (gaze, pupil, blink). |
| Data | **On device; export only by the wearer's choice.** |
| Input | Gaze to point. Confirm with **EEG**, **controller** or **voice**. |
| Passthrough | **Grayscale-first, adapts to color** when a color camera module (e.g. Arcturus) is attached. |
| Placement | **Body-anchored ring** at waist height. It **stays put until recalled**. **Look + confirm raises** an item to reading position. Items can be **pinned** to the room on purpose. |
| Status | A **status arc** on the front of the ring. When you walk away from the ring, **the arc alone follows you** at waist height. |
| Mediated reality | Augmented perception, labels and memory aids, sousveillance capture. Not diminished reality. |
| VR APKs | **App takes over, Loop stays as a thin layer.** |
| Steam games | **Not in scope.** Use Valve's interface for gaming. |
| v1 apps | **Research console**, **read / write / browse**. |
| AI | **On-headset model only.** |
| Visual style | **High-contrast tiles**: solid near-black surfaces, bright text. |
| Color | **Color means something**, always paired with a shape. |
| Notifications | **Sound and controller buzz only**, with no visual popup of any kind. |
| Accessibility | Low vision, limited mobility, Deaf / hard of hearing, cognitive and sensory load. All four are required for v1. |
| Decisions | **Undo over asking.** Only irreversible or privacy actions ask, and they ask inline on the item. |
| BCI confirm | **Adapts to the headset** and is calibrated per wearer. |
| Stack | **Stardust XR** (Rust, Bevy, OpenXR) as the display server, with Loop built as Stardust clients and services. |
| Adaptations | **Shown on the arc and reversible**: every automatic change leaves a mark saying what changed and why. |
| Voice | **Push-to-talk.** Nothing listens in the background. |
| Name | **Loop.** |

---

## 2. Humanistic Intelligence as OS guarantees

Mann describes HI by six signal-flow properties. Loop turns each one into a rule that every component must keep.

| HI property | Loop rule |
|---|---|
| **Unmonopolizing** | Passthrough is always the base layer. Loop's content sits at waist height, below the line of sight. A full takeover happens only when the wearer starts an immersive APK. |
| **Unrestrictive** | You can walk, talk and look around while using Loop. Nothing is head-locked except the thin safety layer during takeover. The ring stays where you leave it instead of chasing your head. |
| **Observable** | Sensor state, recording state and every automatic adaptation are visible on the status arc. Loop never changes something without leaving a mark. |
| **Controllable** | Every automatic adaptation can be reversed with one confirm. Actions are undoable. The microphone is push-to-talk. Export is always the wearer's own action. |
| **Attentive** | Loop senses attention, workload and fatigue (EEG, gaze, pupil, blink) and uses them to time and pace itself. It does not use them to judge the wearer. |
| **Communicative** | The wearer has several ways to express intent (gaze, EEG, controller, voice). Loop answers through the arc, spatial sound and controller haptics. |

Two further rules follow from these:

- **No popups, no floating windows.** Content lives in exactly three kinds of place: on the ring, raised to reading position, or pinned to the room.
- **Neural data belongs to the wearer.** It is recorded only when visible, stored only on the headset, and exported only by the wearer's explicit action (§10).

---

## 3. Hardware baseline

Facts that shape the design (sources at the end):

- **Display & compute:** 2160×2160 LCD per eye, pancake lenses, Snapdragon 8 Gen 3, 16 GB LPDDR5X, Wi-Fi 7.
- **Passthrough:** four grayscale tracking cameras, two of which provide passthrough, with IR illuminators for dark rooms. The Arcturus module on the front expansion port adds two color sensors.
- **Eye tracking:** built in (Valve uses it for foveated streaming).
- **Hand tracking:** treated as **not available**. The most specific source says controllers are required, though sources conflict.
- **Android:** SteamOS on the Frame runs APKs that don't need Google Play Services.
- **OS base:** SteamOS on aarch64. Valve and Collabora have published Holo Core, an aarch64 Arch base for the Frame.

### Must verify on the device (ordered by risk)

1. **Passthrough for third-party OpenXR apps.** Loop is useless without it. Check for the `XR_ENVIRONMENT_BLEND_MODE_ALPHA_BLEND` blend mode, `XR_FB_passthrough`, or a Valve-specific path.
2. **Replacing Steam's interface with a custom session** on the Frame, and how a session returns to Valve's own.
3. **Eye-tracking access:** gaze through `XR_EXT_eye_gaze_interaction` is likely. Pupil size and blink may need Valve's API, or may not be exposed at all.
4. **Persistence during immersive APKs:** can Loop's thin layer draw over another OpenXR app (an overlay API)?
5. **2D APK windows:** are Android 2D apps exposed as Wayland surfaces Loop can place, or only through Valve's interface?
6. **Camera frames** for augmented perception and labels. Raw passthrough frames may be restricted.
7. **Bluetooth LE** access from the session (to talk to the Muse directly).
8. **Stardust/Bevy performance** on the Adreno GPU at 2160² per eye.

Each item has a fallback in §13.

---

## 4. Architecture

```
 ┌──────────────────────────── Steam Frame · stock SteamOS (aarch64) ───────────────────────────┐
 │  Valve: kernel · drivers · tracking · OpenXR runtime · passthrough · Android (APK) layer       │
 │                                                                                               │
 │  ┌─────────────────────────── Loop session ──────────────────────────────────────────────┐   │
 │  │                                                                                        │   │
 │  │  stardust-xr-server (Bevy, OpenXR; GPLv2)  ◄── Wayland ── flatland (2D Linux apps)      │   │
 │  │        ▲  Stardust protocol                                                            │   │
 │  │        │                                                                               │   │
 │  │  loop-ring ─ ring, tiles, status arc, raise/return, pins, thin takeover layer           │   │
 │  │  loop-apps ─ Notes · Reader · Web · Signals · Apps · Settings                          │   │
 │  │        ▲ intents / adaptations                                                         │   │
 │  │        │                                    loop bus (Unix socket, typed events)       │   │
 │  │  loop-intent ─ gaze target + confirm source → select / undo                            │   │
 │  │  loop-adapt  ─ adaptation policy → arc marks (each one reversible)                     │   │
 │  │  loop-sense  ─ LSL in · Muse (BLE) · eye tracking → features, signal quality           │   │
 │  │  loop-voice  ─ push-to-talk speech recognition (on device)                             │   │
 │  │  loop-mind   ─ on-device language model (llama.cpp)                                    │   │
 │  │  loop-record ─ XDF session recording · sousveillance capture · export                  │   │
 │  └────────────────────────────────────────────────────────────────────────────────────────┘   │
 └───────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Display server:** Stardust XR's server, used as-is where possible. Loop changes behavior through clients, not a fork. A fork only happens for something clients cannot do, such as suppressing the default free-floating panel behavior. All of Loop's own code is Rust, built on Stardust's `core`, `molecules` and `asteroids` (declarative UI) client libraries.
- **2D apps:** Stardust's `flatland` hosts Wayland apps. Loop puts their surfaces into tiles instead of floating panels. X11 apps run through `xwayland-satellite`.
- **Sensor streams** stay on LSL internally (one clock, recordable as XDF). Discrete events (selections, adaptations, undo) go over the **loop bus**, a Unix socket carrying typed messages. Events are also mirrored to an LSL marker stream so recordings capture them.
- **As built (M2):** the bus is a hub (`loop-bus`) that repeats JSON lines between services. `loop-sense` publishes its features back onto LSL as "Loop features", and `loop-record` mirrors bus events onto "Loop events". `loop-intent` and `loop-adapt` are modules of `loop-ring` for now, since the ring is their only consumer. They split out when a second one appears (voice, apps).
- **The PC-first rig** is the same stack on x86_64: Stardust in flatscreen mode or on Monado's simulated headset. A **fake passthrough** comes from a webcam or recorded video, converted to grayscale with camera grain. A **synthetic EEG** stream on LSL is adapted from `~/Projects/muse-alpha-motor` (its `sim` source and `attention.py`).

---

## 5. Spatial model

### The ring

- A horizontal ring around the wearer at **waist height**: about 1.05 m for a standing adult, with a radius of about 0.7 m. Height and radius are set per wearer at first run, and there is a seated profile.
- It is drawn as a **hairline**. Tiles sit on the front ~110° of it, tilted to face the eyes. The rest of the ring stays empty and clear.
- It is **world-anchored and stays put**. Walking away leaves it where it is. **Recall** brings it back around you: controller menu button, push-to-talk "ring", or the BCI recall command. Recall is the one action available everywhere.
- **The status arc follows you.** Once you are more than ~1.5 m from the ring, the arc detaches and travels with you at waist height. It uses the same lazy follow as a heading, so quick glances don't move it. It rejoins the ring when you return or recall.
- Tiles are about 0.20 × 0.12 m. Viewed from the eyes they sit about 40° below the horizon: one glance down, with no head turn.

### Raising

- **Look + confirm** raises a tile to **reading position**: about 0.7 m ahead and 10–15° below the line of sight. It is **anchored to the world at that spot**, not to your head.
- Only **one item is raised at a time**. Raising another returns the first to its slot.
- Its ring slot stays as a ghost outline labelled "raised", so you can see where it came from.
- A raised item's actions are words along its bottom edge (*Return · Pin · Undo*), never a menu that pops up.

### Pins

- From reading position, **Pin** puts the item onto the surface you're looking at. Its ring slot shows "on the wall", or wherever it went.
- Without depth sensing it pins at your gaze point, at the distance where your eyes converge. If a depth or color module is present, it snaps to the detected surface.
- Looking at a pinned item and confirming **Return** takes it back to its slot.

### Takeover (immersive APKs)

- The app gets the whole view. Loop keeps only: hold-system-button to exit, a recording mark if recording is on, the EEG connection mark, and passthrough breaking through when you near an obstacle (from Valve's boundary).
- On exit, the ring is exactly as you left it.

---

## 6. Input and intent

**Pointing is your gaze.** A tile you're looking at gets a white outline. There are no hover popups and no tooltips.

**Gaze dwell** (look and hold to select) exists only as an accessibility option, off by default (§12).

**Confirm** comes from any connected source. `loop-intent` fuses them into one select event:

| Source | How it confirms |
|---|---|
| Controller | Trigger. Grip held = push-to-talk. Menu = recall the ring. |
| Keyboard (PC rig) | Space or Enter confirms, Backspace undoes, Home recalls. Stands in for the controller. |
| Voice | Push-to-talk, then a short command ("open", "return", "pin", "undo", "ring"). Dictation inside Notes. |
| EEG, full-head rig (occipital sensors, over LSL) | **Code-modulated visual targets** (c-VEP): the focused tile shimmers with a low-contrast pattern. Opt-in, and off in low-stimulus mode. |
| EEG, forehead band (Muse-class) | **A deliberate jaw clench or double blink**, picked up by the frontal/temporal sensors. This is muscle and eye activity, not brain activity, and Loop says so plainly. As built: a clench is muscle-band (>30 Hz, mains notched) power on TP9/TP10 held above 5× rest for 0.15–1.5 s. A double blink is two same-polarity frontal deflections 0.12–0.8 s apart. |

- When EEG confirm is armed on a focused tile, a **cyan ring mark** appears on that tile: "listening for your confirm".
- **Calibration** happens inline on a single tile ("look here, clench twice"), with no wizard. It is re-run when signal quality drops.
- **Error-potential undo** works only where the montage covers the frontocentral sensors (FCz/Cz). A detected error signal right after a selection triggers **undo**. Whether that happens automatically or as an offer is a setting. It is not available on Muse-class bands: undo stays on the controller and voice.
- **Photosensitivity:** flicker-based paradigms are opt-in, low contrast, confined to the focused tile, kept below photosensitive-epilepsy flash thresholds (WCAG 2.3.1 general/red flash limits), and off whenever low-stimulus mode is on. Onboarding asks about photosensitivity before they can be enabled.

---

## 7. Sensing (`loop-sense`)

- **Inputs:** any LSL stream (EEG, ECG, EDA and so on, recorded even if Loop doesn't interpret it); Muse via OpenMuse's BLE protocol directly from the headset (the handshake already works in `muse-alpha-motor`); Frame eye tracking.
- **Features:**
  - attention / workload index: relative alpha against the wearer's baseline (the method in `attention.py`), plus theta/alpha and pupil size where available
  - fatigue: blink rate and PERCLOS
  - signal quality per channel
- **Passive adaptations** (`loop-adapt`) apply only to Loop's own behavior:
  - hold notification sounds during sustained focus and release them at the next natural break
  - reduce the ring's visual density
  - suggest a break, as a mark on the arc, never a prompt

  Every adaptation shows on the arc as a **lilac diamond** with a short reason ("Sounds held · focus"). Confirming it reverses the adaptation.
- **Research logging:** all streams, plus head pose, gaze and Loop's own events, go to a single XDF file per session. The optional scene video sits beside it with a shared clock. By default only streams from the headset itself are recorded, never other machines' streams on the same network.

---

## 8. Mediated reality

| Power | What it does | Depends on |
|---|---|---|
| Augmented perception | Edge enhancement, low-light boost using the IR illuminators, magnifying a region, contrast boost for low vision. | Camera frame access (verify #6); otherwise only what the runtime's passthrough settings allow. |
| Labels & memory aids | Wearer-authored labels and reminders anchored to places and objects, plus on-device recognition to re-find them. | Spatial anchors (likely available); recognition needs camera frames. |
| Sousveillance capture | First-person video and audio recording that the wearer controls. | Camera frames, or the runtime's capture API. |

When recording is on, the arc always shows a **coral dot** "Recording", and a distinct tone plays at start and stop. Captures follow the same data policy as EEG (§10).

---

## 9. Notifications and decisions

- **Notifications are sound and controller buzz only.** A short, quiet tone, spatialized from where the ring is. The content waits inside the tile that owns it, which shows a count. You read it when you choose to. Nothing appears in your view.
- **Urgency** is a different tone, never a visual.
- **Decisions are undo, not asking.** Loop acts immediately and makes the action undoable (controller, voice, or an error signal).
- **Irreversible and privacy actions** (deleting a recording, exporting data, granting an app the camera) ask **inline on the tile**: one line of text with two worded choices, plus an **amber triangle** "needs you". Nothing is done until you answer.

---

## 10. Privacy and data

- Raw EEG, eye and camera data are stored only on the headset, under the wearer's account.
- **Export is always the wearer's action.** It's done from the Signals or Settings tile, it shows what will leave and where it goes, and it is confirmed inline (§9).
- Settings shows a **data inventory**: what is recorded, how much, since when. Anything can be deleted (inline confirm, because it can't be undone).
- The microphone is open only during push-to-talk.
- Apps get sensor access only by an inline grant on their tile. Sensor access is never granted by default.
- The on-device language model reads sensor-derived data only if the wearer grants it.

---

## 11. Design language

Minimal, quiet, legible over a grayscale world.

**Surfaces.** Tiles are solid near-black (`#0B0B0C`, 96% opaque) with rounded corners. They have no shadows, no gradients and no glass effect.

**Text.** Atkinson Hyperlegible (Braille Institute, designed for low vision).

| Role | Color | Contrast on the tile surface |
|---|---|---|
| Primary | `#F2F2F2` | 17.6 : 1 |
| Secondary | `#C9C9C9` | 11.9 : 1 |

No text role is ever dark gray or dark red, including disabled and error states. Disabled text uses the secondary color plus a strike-through or a word ("off"), never dimming.

**Text size** is set in distance-independent millimetres (mm at 1 m). Starting values, to be tuned on the device: body x-height ≥ 12 dmm (≈0.7°), labels ≥ 16 dmm. Low-vision mode scales everything up to 2×.

**Color means something, and always comes with a shape:**

| Meaning | Color | Shape |
|---|---|---|
| Recording | coral `#FF7B6B` (7.8 : 1) | filled dot ● |
| Listening (BCI armed / push-to-talk open) | cyan `#6FE3FF` (13.2 : 1) | ring ◯ |
| Needs you | amber `#FFC857` (12.8 : 1) | triangle ▲ |
| Loop adapted something | lilac `#C9A7FF` (9.8 : 1) | diamond ◆ |

Everything else is white on black. In color-passthrough mode the palette stays the same; tiles gain a hairline white edge to separate them from colored backgrounds.

**Focus.** A white outline, 3 dmm, on the gazed tile. This is the only hover state.

**Motion.** Nothing moves unless you caused it. Raise and return take ≤ 200 ms, ease-out, with no bounce. Reduced-motion mode makes them instant.

**Sound.** A small set of soft tones, one per meaning, spatialized from the ring.

**Haptics.** Short controller buzzes paired with the tones.

---

## 12. Accessibility (all required for v1)

- **Low vision:** 2× scale, magnifier, passthrough contrast boost (§8), spoken descriptions of the focused tile on push-to-talk "read".
- **Limited mobility:** every action works with gaze + EEG/EMG confirm, gaze + voice, or **gaze dwell** (accessibility-only, adjustable dwell time, with a visible fill on the focus outline), with no controller. There is a seated profile (lower ring, smaller radius). Recall works from any of those inputs.
- **Deaf / hard of hearing:** live captions placed beside the person speaking (captions are content, so they sit as text near the speaker, not as a popup). Every notification tone has a matching controller buzz and a count on its tile.
- **Cognitive / sensory load:** a predictable layout (nothing moves on its own), plain-language labels, a low-stimulus mode (no flicker paradigms, quieter tones, fewer tiles), reduced motion.

---

## 13. Apps and compatibility

**v1 tiles:** Notes · Reader · Web · Signals (research console) · Apps (Android) · Settings.

- **Notes, Reader:** native Loop clients (asteroids UI).
- **Web:** a Wayland browser hosted through flatland, shown inside its tile when raised.
- **Signals:** live traces, signal quality, start/stop recording, event markers, LSL stream list, export.
- **Apps:** the Android library. 2D APKs open in a tile when the platform exposes their surfaces (verify #5); otherwise they hand off to Valve's view. Immersive APKs take over (§5).
- **Not in scope:** the Steam library.
- **AI (`loop-mind`):** llama.cpp with a ~3–4B model (Q4), on device, within a ~3 GB memory budget. Used for voice command parsing, note summaries, and labels.

### Fallbacks for the unverified items (§3)

| # | If unavailable | Fallback |
|---|---|---|
| 1 | Passthrough for third-party apps | Ask Valve. Interim: Loop runs as a VR desk until it's possible. |
| 2 | Custom session | Run Loop as an app launched from Valve's interface (full screen, auto-start). |
| 3 | Pupil / blink data | Workload from EEG only. Fatigue from blink artifacts in the EEG. |
| 4 | Overlay during takeover | Exiting an APK always returns to Loop. No thin layer while the APK runs. |
| 5 | 2D APK surfaces | Hand off to Valve's view for 2D APKs. |
| 6 | Camera frames | Augmented perception limited to what the runtime offers. Labels use anchors only. Capture uses the runtime's recorder. |
| 7 | Bluetooth LE | Muse through a phone relay over Wi-Fi, as in `muse-alpha-motor`. |

---

## 14. Roadmap

| Milestone | Scope |
|---|---|
| **M0 — Spec + renders** (done 2026-10-23) | This document and the mockups. Agree on the look before any code. |
| **M1 — PC shell** (first pass 2026-10-23) | Stardust server + `loop-ring` on Monado or flatscreen. Fake grayscale passthrough. Ring, tiles, raise/return, pins, status arc. Controller and keyboard confirm. *Done in flatscreen except a Monado run. Pointer hover is verified. Keyboard confirm is built (M2); a real click and a real keypress through the server have not been tested by hand yet.* |
| **M2 — Sensing + intent** (first pass 2026-10-25) | `loop-sense` over LSL. Muse live. Synthetic-EEG test harness. Adaptations with arc marks. Jaw-clench / double-blink confirm. XDF recording. *Done against the synthetic band through real LSL, verified end to end in Stardust and with pyxdf. On synthetic signal: 0 false confirms in 120 min of rest, 240/240 clenches and 239/240 double blinks detected. Not yet run against a real Muse; the thresholds need tuning on real data.* |
| **M3 — Apps** | Notes, Reader, Web (flatland), Signals, Settings. Push-to-talk voice. The on-device model. |
| **M4 — Device bring-up** | Steam Frame: verify §3 items 1–8, port to aarch64, tune text sizes and the ring geometry on real optics. |
| **M5 — Accessibility + onboarding** | First-run ring fitting, inline calibration, all four accessibility profiles tested with real users. |

---

## 15. Resolved after the renders (2026-10-23)

- Renders approved as the visual baseline. Build M1 on them.
- **Status away from the ring:** the arc follows you (§5).
- **Gaze dwell:** exists as an accessibility-only option, off by default (§6, §12).
- **c-VEP shimmer:** allowed as an opt-in on full-head rigs, with the photosensitivity rules in §6.

---

Sources: [Wikipedia: Steam Frame](https://en.wikipedia.org/wiki/Steam_Frame) · [vr.org spec sheet](https://vr.org/steam-frame-specs) · [UploadVR](https://uploadvr.com/valve-steam-frame-official-announcement-features-details) · [PC Gamer](https://www.pcgamer.com/uk/hardware/vr-hardware/steam-frame-specs-availability/) · [GamingOnLinux: Holo Core](https://gamingonlinux.com/2026/07/collabora-announce-a-preview-of-holo-core-an-aarch64-port-of-arch-linux-for-steam-frame) · [Stardust XR server](https://github.com/StardustXR/server) · [Stardust dev setup](https://stardustxr.org/docs/dev-setup/dev) · [Godot OpenXR passthrough notes](https://docs.godotengine.org/en/4.2/tutorials/xr/openxr_passthrough.html)
