//! loop-ring: the ring of tiles at waist height, the status arc, raising, returning and pinning
//! (docs/SPEC.md §5, §9, §11). On the PC it runs against a Stardust server in flatscreen mode, with
//! a grayscale panorama standing in for passthrough.
//!
//! It is also where intent is fused (loop-intent, SPEC §6): whatever the wearer is looking at is the
//! target, and a confirm from any source (pointer, keyboard, clench, double blink) acts on it. The
//! passive adaptation policy (loop-adapt, SPEC §7) lives in `sensing`. Both split into services of
//! their own once something other than the ring needs them.
mod hotspot;
mod look;
mod script;
mod sensing;

use glam::{Quat, Vec3};
use hotspot::Hotspot;
use look::*;
use loop_bus::{Bus, ConfirmSource, Event, SimCmd};
use sensing::Sensing;
use serde::{Deserialize, Serialize};
use stardust_xr_asteroids::{
	ClientState, Context, CustomElement, Element, Entity, Migrate, Reify, Tasker, Transformable, client,
	components::KeyboardHandler,
	elements::{Lines, Model, ModelPart, SkyTex, Spatial, StageSpace, Text},
	project_local_resources,
};
use stardust_xr_fusion::{
	client::{Client, DefaultHandler, FrameInfo},
	drawable::{XAlign, YAlign},
	fields::Shape,
	spatial::{Spatial as SpatialNode, SpatialExt, SpatialRef, Transform},
	tracked::{Tracked, TrackedExt},
	types::Color,
};
use stardust_xr_molecules::keyboard_handler::protocol::KeyEvent;
use stardust_xr_molecules::lines::{LineExt, circle, line_from_points, rounded_rectangle};
use std::{f32::consts::FRAC_PI_2, sync::Arc, time::Duration};

#[tokio::main(flavor = "current_thread")]
async fn main() {
	tracing_subscriber::fmt()
		.with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
		.init();
	client::run::<State>(&[&project_local_resources!("res")])
		.await
		.unwrap();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
	Notes,
	Reader,
	Web,
	Signals,
	Apps,
	Settings,
}
impl Kind {
	fn label(self) -> &'static str {
		match self {
			Kind::Notes => "Notes",
			Kind::Reader => "Reader",
			Kind::Web => "Web",
			Kind::Signals => "Signals",
			Kind::Apps => "Apps",
			Kind::Settings => "Settings",
		}
	}
	fn body(self) -> &'static str {
		match self {
			Kind::Notes => {
				"The ring stayed put when I walked to the desk.\nRecall from the kitchen felt instant.\nMuse lost AF7 twice. Re-seat it before the session.\nTry a lower ring for seated work, about 0.9 m.\nClench to confirm: 23 of 24 today."
			}
			Kind::Reader => {
				"The wearer and the machine form one loop.\nWhat the machine senses, the wearer can see.\nWhat it changes on its own, the wearer can undo.\nNothing asks for attention it was not given."
			}
			Kind::Web => "Pages open here through flatland (M3).",
			Kind::Signals => "",
			Kind::Apps => {
				"Android apps are listed here (M3).\nImmersive apps take over the view;\nLoop stays as a thin layer."
			}
			Kind::Settings => {
				"Data: on this device, export by your choice\nVoice: push-to-talk\nGaze dwell: off (accessibility option)\nLow-stimulus mode: off"
			}
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Place {
	Ring,
	Raised,
	Pinned(Pin),
}

/// A pin stays in the room: it keeps the ring's pose from the moment it was pinned, so recalling
/// the ring elsewhere leaves it where it was.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pin {
	slot: u8,
	anchor: [f32; 3],
	yaw: f32,
}
impl Pin {
	const ANGLES: [f32; 6] = [28.0, -28.0, 56.0, -56.0, 84.0, -84.0];
	fn local_pos(self) -> Vec3 {
		on_circle(2.4, Self::ANGLES[self.slot as usize % Self::ANGLES.len()], 1.62)
	}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
	kind: Kind,
	place: Place,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Focus {
	Tile(usize),
	Action(usize, Action),
	Arc(ArcAction),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
	Return,
	Pin,
	Undo,
	/// Start or stop recording the session (Signals only).
	Record,
	/// Learn the wearer's clench (Signals only).
	Calibrate,
}
impl Action {
	fn word(self, recording: bool) -> &'static str {
		match self {
			Action::Return => "Return",
			Action::Pin => "Pin",
			Action::Undo => "Undo",
			Action::Record if recording => "Stop",
			Action::Record => "Record",
			Action::Calibrate => "Calibrate",
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ArcAction {
	ReverseAdaptation,
	Recall,
	StopRecording,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)] // Triangle marks "needs you" once inline decisions land (M3)
enum Glyph {
	Dot,
	Ring,
	Triangle,
	Diamond,
}

#[derive(Debug)]
struct ArcFollow {
	floor: Vec3,
	yaw: f32,
	turn_started: Option<f32>,
}

/// The connection to the loop bus.
struct BusLink(Bus);
impl std::fmt::Debug for BusLink {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("BusLink")
	}
}

#[derive(Debug, Default)]
struct Runtime {
	time: f32,
	hovered: Option<Focus>,
	undo: Vec<Vec<Place>>,
	sensing: Sensing,
	bus: Option<BusLink>,
	head: Option<(Vec3, Quat)>,
	ring_placed: bool,
	follow: Option<ArcFollow>,
	battery: Option<u8>,
	battery_checked: f32,
	script: script::Script,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct State {
	items: Vec<Item>,
	ring_origin: [f32; 3],
	ring_yaw: f32,
	#[serde(skip)]
	rt: Runtime,
}
impl Default for State {
	fn default() -> Self {
		let place = |kind| match kind {
			Kind::Notes => Place::Raised,
			Kind::Signals => Place::Pinned(Pin {
				slot: 0,
				anchor: [0.0, -EYE_HEIGHT, 0.0],
				yaw: 0.0,
			}),
			_ => Place::Ring,
		};
		State {
			items: [Kind::Notes, Kind::Reader, Kind::Web, Kind::Signals, Kind::Apps, Kind::Settings]
				.into_iter()
				.map(|kind| Item {
					kind,
					place: place(kind),
				})
				.collect(),
			ring_origin: [0.0, -EYE_HEIGHT, 0.0],
			ring_yaw: 0.0,
			rt: Runtime::default(),
		}
	}
}
impl Migrate for State {
	type Old = Self;
}

impl ClientState for State {
	const APP_ID: &'static str = "org.loop.ring";

	fn on_start(&mut self, context: &Context, tasks: impl Tasker<Self>) {
		self.rt.script = script::Script::from_env();
		self.rt.bus = Some(BusLink(loop_bus::connect("loop-ring")));
		let client = context.stardust_client.clone();
		tasks.spawn(
			async move { head_probe(client).await.map(Arc::new) },
			|tasks, _state: &mut State, head| {
				if let Some(head) = head {
					poll_head(tasks, head);
				}
			},
		);
	}

	fn on_frame(&mut self, _context: &Context, info: &FrameInfo) {
		let dt = info.delta;
		self.rt.time += dt;
		let now = self.rt.time;
		let mut incoming = Vec::new();
		if let Some(BusLink(bus)) = &mut self.rt.bus {
			while let Ok(envelope) = bus.rx.try_recv() {
				incoming.push(envelope.event);
			}
		}
		for event in incoming {
			match event {
				Event::Confirm { source } => self.confirm(source),
				event => self.rt.sensing.on_event(&event, now),
			}
		}
		if let Some(change) = self.rt.sensing.step(now) {
			self.tell(change, None);
		}
		if now - self.rt.battery_checked > 10.0 || self.rt.battery_checked == 0.0 {
			self.rt.battery_checked = now.max(f32::EPSILON);
			self.rt.battery = read_battery();
		}
		for cmd in self.rt.script.due(now) {
			self.run(cmd);
		}
		self.update_follow(now);
	}
}

// ------------------------------------------------------------------ behaviour

impl State {
	fn hover(&mut self, focus: Focus, on: bool) {
		if on {
			self.rt.hovered = Some(focus);
		} else if self.rt.hovered == Some(focus) {
			self.rt.hovered = None;
		}
	}

	fn send(&self, event: Event) {
		if let Some(BusLink(bus)) = &self.rt.bus {
			let _ = bus.tx.send(event);
		}
	}

	/// Logs something the wearer did, or Loop did on its own (recorded as a marker by loop-record).
	fn tell(&self, what: impl Into<String>, source: Option<ConfirmSource>) {
		self.send(Event::Ui {
			what: what.into(),
			source,
		});
	}

	/// A confirm from any source acts on whatever the wearer is looking at. Body-signal confirms
	/// count only while the listening mark can show (SPEC §6).
	fn confirm(&mut self, source: ConfirmSource) {
		let Some(focus) = self.rt.hovered else {
			return;
		};
		if source.is_sensed() && !self.bci_armed() {
			return;
		}
		self.activate(focus, source);
	}

	fn activate(&mut self, focus: Focus, source: ConfirmSource) {
		let what = match focus {
			Focus::Tile(i) => self.press_tile(i),
			Focus::Action(i, action) => self.act(i, action),
			Focus::Arc(action) => self.arc_action(action),
		};
		self.tell(what, Some(source));
	}

	/// Keyboard stand-ins for the controller (SPEC §6): Space or Enter confirms, Backspace undoes,
	/// Home recalls the ring. C and B make the synthetic headband clench or double-blink. Keys reach
	/// Loop while the pointer is on something of Loop's.
	fn on_key(&mut self, key: KeyEvent) {
		if !key.pressed {
			return;
		}
		const ENTER: u32 = 28;
		const SPACE: u32 = 57;
		const KEYPAD_ENTER: u32 = 96;
		const BACKSPACE: u32 = 14;
		const HOME: u32 = 102;
		const C: u32 = 46;
		const B: u32 = 48;
		match key.keycode {
			ENTER | SPACE | KEYPAD_ENTER => self.confirm(ConfirmSource::Keyboard),
			BACKSPACE => {
				let what = self.undo();
				self.tell(what, Some(ConfirmSource::Keyboard));
			}
			HOME => {
				self.recall();
				self.tell("recall", Some(ConfirmSource::Keyboard));
			}
			C => self.send(Event::Sim(SimCmd::Clench)),
			B => self.send(Event::Sim(SimCmd::DoubleBlink)),
			_ => {}
		}
	}

	fn run(&mut self, cmd: script::Cmd) {
		use script::Cmd;
		let source = ConfirmSource::Script;
		match cmd {
			Cmd::Press(i) if i < self.items.len() => self.activate(Focus::Tile(i), source),
			Cmd::Pin(i) if i < self.items.len() => self.activate(Focus::Action(i, Action::Pin), source),
			Cmd::Return(i) if i < self.items.len() => {
				self.activate(Focus::Action(i, Action::Return), source)
			}
			Cmd::Undo => {
				let what = self.undo();
				self.tell(what, Some(source));
			}
			Cmd::Hover(i) => self.rt.hovered = i.map(Focus::Tile),
			Cmd::Move(dx, dz) => {
				self.ring_origin[0] += dx;
				self.ring_origin[2] += dz;
			}
			Cmd::Recall => {
				self.recall();
				self.tell("recall", Some(source));
			}
			Cmd::Reverse => self.activate(Focus::Arc(ArcAction::ReverseAdaptation), source),
			Cmd::Clench => self.send(Event::Sim(SimCmd::Clench)),
			Cmd::Blink => self.send(Event::Sim(SimCmd::Blink)),
			Cmd::DoubleBlink => self.send(Event::Sim(SimCmd::DoubleBlink)),
			Cmd::Focus(value) => self.send(Event::Sim(SimCmd::Focus { value })),
			Cmd::Contact(channel, good) => self.send(Event::Sim(SimCmd::Contact { channel, good })),
			Cmd::Key => self.confirm(ConfirmSource::Keyboard),
			Cmd::Record(on) => self.send(Event::Record { on }),
			Cmd::Calibrate => self.send(Event::Calibrate),
			_ => {}
		}
	}

	fn snapshot(&mut self) {
		self.rt.undo.push(self.items.iter().map(|i| i.place).collect());
		if self.rt.undo.len() > 32 {
			self.rt.undo.remove(0);
		}
	}

	fn press_tile(&mut self, i: usize) -> String {
		self.snapshot();
		let label = self.items[i].kind.label();
		match self.items[i].place {
			Place::Ring => {
				for item in &mut self.items {
					if item.place == Place::Raised {
						item.place = Place::Ring;
					}
				}
				self.items[i].place = Place::Raised;
				format!("raise {label}")
			}
			Place::Raised | Place::Pinned(_) => {
				self.items[i].place = Place::Ring;
				format!("return {label}")
			}
		}
	}

	fn act(&mut self, i: usize, action: Action) -> String {
		let label = self.items[i].kind.label();
		let what = match action {
			Action::Return => {
				self.snapshot();
				self.items[i].place = Place::Ring;
				format!("return {label}")
			}
			Action::Pin => {
				self.snapshot();
				let slot = (0u8..)
					.find(|s| {
						!self
							.items
							.iter()
							.any(|it| matches!(it.place, Place::Pinned(p) if p.slot == *s))
					})
					.unwrap();
				self.items[i].place = Place::Pinned(Pin {
					slot,
					anchor: self.ring_origin,
					yaw: self.ring_yaw,
				});
				format!("pin {label}")
			}
			Action::Undo => self.undo(),
			Action::Record => {
				let on = self.rt.sensing.recording.is_none();
				self.send(Event::Record { on });
				format!("record {}", if on { "on" } else { "off" })
			}
			Action::Calibrate => {
				self.send(Event::Calibrate);
				"calibrate clench".into()
			}
		};
		self.rt.hovered = None;
		what
	}

	fn undo(&mut self) -> String {
		if let Some(places) = self.rt.undo.pop() {
			for (item, place) in self.items.iter_mut().zip(places) {
				item.place = place;
			}
			"undo".into()
		} else {
			"undo: nothing to undo".into()
		}
	}

	fn arc_action(&mut self, action: ArcAction) -> String {
		let what = match action {
			ArcAction::ReverseAdaptation => {
				self.rt.sensing.reverse_adaptation(self.rt.time);
				"adaptation reversed"
			}
			ArcAction::Recall => {
				self.recall();
				"recall"
			}
			ArcAction::StopRecording => {
				self.send(Event::Record { on: false });
				"record off"
			}
		};
		self.rt.hovered = None;
		what.into()
	}

	/// Brings the ring around the wearer, facing where they face.
	fn recall(&mut self) {
		let Some((pos, rot)) = self.rt.head else {
			return;
		};
		self.ring_origin = [pos.x, pos.y - EYE_HEIGHT, pos.z];
		self.ring_yaw = yaw_of(rot);
		self.rt.follow = None;
	}

	fn on_head(&mut self, pose: (Vec3, Quat)) {
		self.rt.head = Some(pose);
		if !self.rt.ring_placed {
			self.rt.ring_placed = true;
			self.recall();
			for item in &mut self.items {
				if let Place::Pinned(pin) = &mut item.place {
					pin.anchor = self.ring_origin;
					pin.yaw = self.ring_yaw;
				}
			}
		}
	}

	/// The status arc leaves the ring once the wearer walks away, and follows with a lazy heading.
	fn update_follow(&mut self, now: f32) {
		let Some((pos, rot)) = self.rt.head else {
			return;
		};
		let origin = Vec3::from(self.ring_origin);
		let away = Vec3::new(pos.x - origin.x, 0.0, pos.z - origin.z).length() > ARC_FOLLOW_DISTANCE;
		if !away {
			self.rt.follow = None;
			return;
		}
		let floor = Vec3::new(pos.x, pos.y - EYE_HEIGHT, pos.z);
		let head_yaw = yaw_of(rot);
		let follow = self.rt.follow.get_or_insert(ArcFollow {
			floor,
			yaw: head_yaw,
			turn_started: None,
		});
		follow.floor = floor;
		let diff = angle_diff(head_yaw, follow.yaw).abs();
		if diff > 30f32.to_radians() {
			let started = *follow.turn_started.get_or_insert(now);
			if now - started > 1.5 {
				follow.yaw = head_yaw;
				follow.turn_started = None;
			}
		} else {
			follow.turn_started = None;
		}
	}

	fn bci_armed(&self) -> bool {
		self.rt.sensing.armed(self.rt.time)
	}
}

fn yaw_of(rot: Quat) -> f32 {
	let forward = rot * Vec3::NEG_Z;
	(-forward.x).atan2(-forward.z)
}

fn angle_diff(a: f32, b: f32) -> f32 {
	let d = (a - b).rem_euclid(std::f32::consts::TAU);
	if d > std::f32::consts::PI {
		d - std::f32::consts::TAU
	} else {
		d
	}
}

fn read_battery() -> Option<u8> {
	let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
	dir.filter_map(Result::ok)
		.filter(|e| e.file_name().to_string_lossy().starts_with("BAT"))
		.find_map(|e| std::fs::read_to_string(e.path().join("capacity")).ok())
		.and_then(|s| s.trim().parse().ok())
}

struct HeadProbe {
	probe: SpatialNode,
	stage: SpatialRef,
}

/// A spatial parented to the head, so its pose relative to the stage can be read back.
async fn head_probe(client: Arc<Client<DefaultHandler>>) -> Option<HeadProbe> {
	let hmd = Tracked::hmd_spatial().await.ok()?;
	let stage = Tracked::stage_spatial().await.ok()?;
	let (probe, _) = SpatialNode::new(&client, &hmd, Transform::IDENTITY).await.ok()?;
	Some(HeadProbe { probe, stage })
}

fn poll_head<T: Tasker<State>>(tasks: T, head: Arc<HeadProbe>) {
	let h = head.clone();
	tasks.clone().spawn(
		async move {
			tokio::time::sleep(Duration::from_millis(50)).await;
			let t = h.probe.get_relative_transform(h.stage.clone()).await.ok()?.ok()?;
			Some((Vec3::from(t.translation), Quat::from(t.rotation)))
		},
		move |tasks, state: &mut State, pose| {
			if let Some(pose) = pose {
				state.on_head(pose);
			}
			poll_head(tasks, head);
		},
	);
}

// ------------------------------------------------------------------ drawing

fn surface_model(name: &str, c: Color) -> impl Element<State> + use<> {
	Model::namespaced("loop", name)
		.part(
			ModelPart::new("surface")
				.mat_param("unlit", unlit())
				.mat_param("color", color(c)),
		)
		.build()
}

fn outline(w: f32, h: f32, r: f32, thickness: f32, c: Color) -> impl Element<State> {
	Lines::new([rounded_rectangle(w, h, r, 8).thickness(thickness).color(c)])
		.pos([0.0, 0.0, LIFT * 0.5])
		.build()
}

fn glyph(g: Glyph, c: Color, size: f32, pos: [f32; 3]) -> impl Element<State> {
	let mesh = match g {
		Glyph::Dot => Some("disc"),
		Glyph::Triangle => Some("triangle"),
		Glyph::Diamond => Some("diamond"),
		Glyph::Ring => None,
	};
	Spatial::default()
		.pos(pos)
		.build()
		.maybe_child(mesh.map(|m| {
			Model::namespaced("loop", m)
				.part(
					ModelPart::new("surface")
						.mat_param("unlit", unlit())
						.mat_param("color", color(c)),
				)
				.scl([size; 3])
				.build()
		}))
		.maybe_child((g == Glyph::Ring).then(|| {
			Lines::new([circle(32, 0.0, size * 0.4).thickness(size * 0.2).color(c)])
				.rot(Quat::from_rotation_x(FRAC_PI_2))
				.build()
		}))
}

fn words(body: impl ToString, height: f32, c: Color, bold: bool, pos: [f32; 3]) -> Text {
	Text::new(body)
		.character_height(height)
		.color(c)
		.font(if bold { font_bold() } else { font_regular() })
		.pos(pos)
}

impl State {
	fn tile(&self, i: usize, item: &Item) -> impl Element<State> {
		let angle = -45.0 + 18.0 * i as f32;
		let pos = on_circle(RING_RADIUS, angle, TILE_HEIGHT);
		let ghost = item.place != Place::Ring;
		let focused = self.rt.hovered == Some(Focus::Tile(i));
		let [w, h] = TILE_SIZE;
		let sub = match (item.place, item.kind) {
			(Place::Raised, _) => "Raised".to_string(),
			(Place::Pinned(_), _) => "On the wall".to_string(),
			(_, Kind::Notes) => "3 drafts".to_string(),
			(_, Kind::Reader) => "Ch. 4 · 62%".to_string(),
			(_, Kind::Web) => "2 pages".to_string(),
			(_, Kind::Signals) => match self.rt.sensing.quality(self.rt.time) {
				(_, 0) => "No EEG".to_string(),
				(good, total) => format!("{good} of {total} good"),
			},
			(_, Kind::Apps) => "12 Android".to_string(),
			(_, Kind::Settings) => "All on device".to_string(),
		};
		let (label_color, sub_color) = if ghost {
			(text2(), text2())
		} else {
			(text(), text2())
		};
		Spatial::default()
			.pos(pos)
			.rot(facing(pos, eye()))
			.build()
			.child(surface_model(
				if ghost { "tile_ghost" } else { "tile" },
				if ghost { ghost_surface() } else { surface() },
			))
			.maybe_child(ghost.then(|| outline(w, h, 0.014, 0.0022, ghost_line())))
			.maybe_child(focused.then(|| outline(w + 0.008, h + 0.008, 0.018, 0.0035, focus())))
			.child(
				words(item.kind.label(), 0.032, label_color, true, [-w / 2.0 + TILE_PAD, h / 2.0 - TILE_PAD, LIFT])
					.build(),
			)
			.child(
				words(sub, 0.021, sub_color, false, [-w / 2.0 + TILE_PAD, -h / 2.0 + TILE_PAD, LIFT])
					.align_y(YAlign::Bottom)
					.build(),
			)
			.maybe_child((focused && !ghost && self.bci_armed()).then(|| {
				glyph(
					Glyph::Ring,
					listening(),
					0.024,
					[w / 2.0 - TILE_PAD - 0.008, h / 2.0 - TILE_PAD - 0.008, LIFT],
				)
			}))
			.child(
				Hotspot::new(
					TILE_SIZE,
					move |s: &mut State, on| s.hover(Focus::Tile(i), on),
					move |s: &mut State| s.activate(Focus::Tile(i), ConfirmSource::Controller),
				)
				.pos([0.0, 0.0, LIFT * 2.0])
				.build(),
			)
	}

	/// A raised item (reading position) or a pinned one (on the wall); `k` scales type for distance.
	fn panel(&self, i: usize, raised: bool) -> impl Element<State> {
		let item = &self.items[i];
		let (pos, rot, [w, h], k, mesh) = if raised {
			let pos = Vec3::new(0.0, EYE_HEIGHT - 0.72 * 12f32.to_radians().tan(), -0.72);
			(pos, facing(pos, eye()), RAISED_SIZE, 1.0, "raised")
		} else {
			let pos = match item.place {
				Place::Pinned(pin) => pin.local_pos(),
				_ => Vec3::ZERO,
			};
			(pos, facing(pos, Vec3::new(0.0, 1.62, 0.0)), PINNED_SIZE, 1.8, "pinned")
		};
		let left = -w / 2.0 + 0.028 * k;
		let top = h / 2.0 - 0.026 * k;
		let now = self.rt.time;
		let sensing = &self.rt.sensing;
		let signals = sensing.signals(now);
		let recording = sensing.recording.is_some();
		let meta = match item.kind {
			Kind::Signals => match signals {
				Some(_) => {
					let (good, total) = sensing.quality(now);
					format!("{} · {good} of {total} good", sensing.label(now))
				}
				None => "Waiting for a headband".to_string(),
			},
			Kind::Notes => "Thursday · 4 entries".to_string(),
			_ => String::new(),
		};
		// What the machine senses, the wearer can see (SPEC §2): focus, or what it is still learning.
		// While calibrating, the line asks for the gesture next to the listening mark.
		let status: Option<(String, Color, bool)> = if item.kind != Kind::Signals {
			None
		} else if let Some((done, of)) = sensing.calibration {
			Some((format!("Clench twice, firmly · {done} of {of}"), listening(), true))
		} else {
			let rec = if recording { "recording" } else { "not recording" };
			signals.map(|s| match s.focus {
				Some(f) => (format!("Focus {:.0}% of your range · {rec}", f * 100.0), text2(), false),
				None => (format!("Learning your baseline {:.0}% · {rec}", s.baseline * 100.0), text2(), false),
			})
		};
		let actions: &[Action] = match (item.kind, raised) {
			(Kind::Signals, true) => &[Action::Return, Action::Pin, Action::Record, Action::Calibrate],
			(Kind::Signals, false) => &[Action::Return, Action::Record, Action::Calibrate],
			(_, true) => &[Action::Return, Action::Pin, Action::Undo],
			(_, false) => &[Action::Return],
		};
		let rows_top = top - 0.10 * k;
		let row_gap = (rows_top - (-h / 2.0 + 0.085 * k)) / 3.0;
		let row_y = move |row: usize| rows_top - row as f32 * row_gap;
		let channels: Vec<(String, Option<String>)> = signals
			.map(|s| {
				s.channels
					.iter()
					.take(4)
					.map(|c| (c.name.clone(), c.issue.clone()))
					.collect()
			})
			.unwrap_or_default();
		let traces = (item.kind == Kind::Signals).then(|| {
			let x0 = left + 0.09 * k;
			let x1 = w / 2.0 - 0.03 * k;
			// One Lines per channel: each stays under the 8 KiB IPC payload limit.
			Spatial::default().build().children(
				sensing
					.traces
					.iter()
					.zip(&channels)
					.enumerate()
					.filter(|(_, (_, (_, issue)))| issue.is_none())
					.map(|(row, (buf, _))| {
						let y = row_y(row);
						let n = buf.len().max(2);
						let step = (n / 120).max(1);
						let points: Vec<[f32; 3]> = buf
							.iter()
							.step_by(step)
							.enumerate()
							.map(|(j, v)| {
								let x = x0 + (x1 - x0) * (j * step) as f32 / (n - 1) as f32;
								// ±40 µV fills a row.
								[x, y + (v / 40.0).clamp(-1.8, 1.8) * 0.012 * k, LIFT]
							})
							.collect();
						Lines::new([line_from_points(points).thickness(0.0018 * k).color(text())]).build()
					}),
			)
		});
		let channel_labels = (item.kind == Kind::Signals).then(|| {
			Spatial::default().build().children(channels.iter().enumerate().map(|(row, (name, issue))| {
				let y = row_y(row);
				Spatial::default()
					.build()
					.child(
						words(name.clone(), 0.016 * k, text2(), false, [left, y, LIFT])
							.align_y(YAlign::Center)
							.build(),
					)
					.maybe_child(issue.as_ref().map(|issue| {
						words(issue.clone(), 0.016 * k, text2(), false, [left + 0.09 * k, y, LIFT])
							.align_y(YAlign::Center)
							.build()
					}))
			}))
		});
		let body = (item.kind != Kind::Signals).then(|| {
			words(item.kind.body(), 0.0205 * k, text(), false, [left, top - 0.06 * k, LIFT]).build()
		});
		let status = status.map(|(line, c, listening_mark)| {
			let y = top - 0.052 * k;
			let x = if listening_mark { left + 0.026 * k } else { left };
			Spatial::default()
				.build()
				.maybe_child(listening_mark.then(|| glyph(Glyph::Ring, c, 0.02 * k, [left + 0.01 * k, y, LIFT])))
				.child(words(line, 0.018 * k, c, false, [x, y, LIFT]).align_y(YAlign::Center).build())
		});
		let word_h = 0.022 * k;
		let mut x = left;
		let placed_actions: Vec<(Action, f32, f32)> = actions
			.iter()
			.map(|action| {
				let width = text_width(action.word(recording), word_h, true) + 0.02 * k;
				let at = x;
				x += width + 0.012 * k;
				(*action, at, width)
			})
			.collect();
		Spatial::default()
			.pos(pos)
			.rot(rot)
			.build()
			.child(surface_model(mesh, surface()))
			.child(words(item.kind.label(), 0.034 * k, text(), true, [left, top, LIFT]).build())
			.child(
				words(meta, 0.019 * k, text2(), false, [w / 2.0 - 0.028 * k, top, LIFT])
					.align_x(XAlign::Right)
					.build(),
			)
			.maybe_child(status)
			.maybe_child(body)
			.maybe_child(traces)
			.maybe_child(channel_labels)
			.child(
				Lines::new([line_from_points(vec![
					[left, -h / 2.0 + 0.062 * k, LIFT],
					[w / 2.0 - 0.028 * k, -h / 2.0 + 0.062 * k, LIFT],
				])
				.thickness(0.0012 * k)
				.color(text2())])
				.build(),
			)
			.children(placed_actions.into_iter().map(|(action, x, width)| {
				let y = -h / 2.0 + 0.031 * k;
				let focused = self.rt.hovered == Some(Focus::Action(i, action));
				Spatial::default()
					.pos([x, y, 0.0])
					.build()
					.child(
						words(action.word(recording), word_h, text(), true, [0.0, 0.0, LIFT])
							.align_y(YAlign::Center)
							.build(),
					)
					.maybe_child(focused.then(|| {
						Spatial::default()
							.pos([width / 2.0 - 0.01 * k, 0.0, 0.0])
							.build()
							.child(outline(width, 0.04 * k, 0.01 * k, 0.003, focus()))
					}))
					.child(
						Hotspot::new(
							[width, 0.04 * k],
							move |s: &mut State, on| s.hover(Focus::Action(i, action), on),
							move |s: &mut State| s.activate(Focus::Action(i, action), ConfirmSource::Controller),
						)
						.pos([width / 2.0 - 0.01 * k, 0.0, LIFT * 2.0])
						.build(),
					)
			}))
	}

	fn arc_segments(&self, following: bool) -> Vec<Segment> {
		let mut segs = vec![
			Segment::text(chrono::Local::now().format("%H:%M").to_string(), true),
			Segment::text(
				self.rt
					.battery
					.map_or("Plugged in".to_string(), |b| format!("{b}%")),
				false,
			),
		];
		let now = self.rt.time;
		let (good, total) = self.rt.sensing.quality(now);
		segs.push(Segment {
			dots: (total > 0).then_some((good, total.min(8))),
			..Segment::text(self.rt.sensing.label(now), false)
		});
		if self.rt.sensing.recording.is_some() {
			segs.push(Segment {
				glyph: Some((Glyph::Dot, recording())),
				action: Some(ArcAction::StopRecording),
				..Segment::text("Recording", false)
			});
		}
		if let Some(adaptation) = &self.rt.sensing.adaptation {
			segs.push(Segment {
				glyph: Some((Glyph::Diamond, adapted())),
				action: Some(ArcAction::ReverseAdaptation),
				..Segment::text(adaptation.reason.clone(), false)
			});
		}
		if following {
			segs.push(Segment {
				action: Some(ArcAction::Recall),
				..Segment::text("Bring the ring", true)
			});
		}
		segs
	}

	fn arc(&self, following: bool) -> impl Element<State> {
		let segs = self.arc_segments(following);
		let gap = 0.012;
		let total: f32 = segs.iter().map(Segment::width).sum::<f32>() + gap * (segs.len() as f32 - 1.0);
		let mut s = -total / 2.0;
		let placed: Vec<(Segment, f32)> = segs
			.into_iter()
			.map(|seg| {
				let w = seg.width();
				let angle = ((s + w / 2.0) / ARC_RADIUS).to_degrees();
				s += w + gap;
				(seg, angle)
			})
			.collect();
		Spatial::default()
			.build()
			.children(placed.into_iter().map(|(seg, angle)| self.segment(seg, angle)))
	}

	fn segment(&self, seg: Segment, angle: f32) -> impl Element<State> {
		let pos = on_circle(ARC_RADIUS, angle, ARC_HEIGHT);
		let (mesh, w) = pill_for(seg.width());
		let pad = 0.016;
		let mut x = -w / 2.0 + pad;
		let glyph_el = seg.glyph.map(|(g, c)| {
			let el = glyph(g, c, 0.018, [x + 0.009, 0.0, LIFT]);
			x += 0.026;
			el
		});
		let text_x = x;
		x += text_width(&seg.text, SEG_TEXT, seg.bold) + 0.008;
		let dots = seg.dots.map(|(good, total)| {
			Spatial::default().build().children((0..total).map(move |d| {
				glyph(
					if d < good { Glyph::Dot } else { Glyph::Ring },
					text(),
					0.009,
					[x + 0.006 + d as f32 * 0.014, 0.0, LIFT],
				)
			}))
		});
		let focused = seg.action.is_some_and(|a| self.rt.hovered == Some(Focus::Arc(a)));
		Spatial::default()
			.pos(pos)
			.rot(facing(pos, eye()))
			.build()
			.child(surface_model(&mesh, surface()))
			.maybe_child(glyph_el)
			.child(
				words(seg.text.clone(), SEG_TEXT, text(), seg.bold, [text_x, 0.0, LIFT])
					.align_y(YAlign::Center)
					.build(),
			)
			.maybe_child(dots)
			.maybe_child(focused.then(|| {
				outline(w + 0.006, PILL_HEIGHT + 0.006, PILL_HEIGHT / 2.0, 0.003, focus())
			}))
			.maybe_child(seg.action.map(|action| {
				Hotspot::new(
					[w, PILL_HEIGHT],
					move |s: &mut State, on| s.hover(Focus::Arc(action), on),
					move |s: &mut State| s.activate(Focus::Arc(action), ConfirmSource::Controller),
				)
				.pos([0.0, 0.0, LIFT * 2.0])
				.build()
			}))
	}
}

const SEG_TEXT: f32 = 0.022;

#[derive(Debug, Clone)]
struct Segment {
	text: String,
	bold: bool,
	glyph: Option<(Glyph, Color)>,
	/// Good channels of all channels.
	dots: Option<(usize, usize)>,
	action: Option<ArcAction>,
}
impl Segment {
	fn text(text: impl Into<String>, bold: bool) -> Self {
		Segment {
			text: text.into(),
			bold,
			glyph: None,
			dots: None,
			action: None,
		}
	}
	fn width(&self) -> f32 {
		let mut w = 0.032 + text_width(&self.text, SEG_TEXT, self.bold);
		if self.glyph.is_some() {
			w += 0.026;
		}
		if let Some((_, total)) = self.dots {
			w += 0.008 + total as f32 * 0.014;
		}
		pill_for(w).1
	}
}

impl Reify for State {
	fn reify(
		&self,
		_context: &Context,
		_tasks: impl Tasker<Self>,
		_props: (),
	) -> impl Element<Self> {
		let raised = self.items.iter().position(|i| i.place == Place::Raised);
		let pinned: Vec<(usize, Pin)> = self
			.items
			.iter()
			.enumerate()
			.filter_map(|(i, item)| match item.place {
				Place::Pinned(pin) => Some((i, pin)),
				_ => None,
			})
			.collect();
		let following = self.rt.follow.as_ref().map(|f| (f.floor, f.yaw));
		StageSpace
			.build()
			// Keys arrive while the pointer is on any of Loop's targets: the server gives keyboard
			// focus to handlers near the pointer's hit, and this field covers the room.
			.child(
				Entity::new(Shape::Box {
					size: [40.0, 8.0, 40.0].into(),
				})
				.component(KeyboardHandler::new().on_key(|s: &mut State, key, _| s.on_key(key)))
				.build(),
			)
			.child(
				SkyTex {
					resource: resource("passthrough"),
					opaque: true,
				}
				.build(),
			)
			.child(
				Spatial::default()
					.pos(Vec3::from(self.ring_origin))
					.rot(Quat::from_rotation_y(self.ring_yaw))
					.build()
					.child(
						Lines::new([circle(160, 0.0, RING_RADIUS).thickness(0.0022).color(text2())])
							.pos([0.0, RING_HEIGHT, 0.0])
							.build(),
					)
					.children(self.items.iter().enumerate().map(|(i, item)| self.tile(i, item)))
					.maybe_child(raised.map(|i| self.panel(i, true)))
					.maybe_child(following.is_none().then(|| self.arc(false))),
			)
			.children(pinned.into_iter().map(|(i, pin)| {
				Spatial::default()
					.pos(Vec3::from(pin.anchor))
					.rot(Quat::from_rotation_y(pin.yaw))
					.build()
					.child(self.panel(i, false))
			}))
			.maybe_child(following.map(|(floor, yaw)| {
				Spatial::default()
					.pos(floor)
					.rot(Quat::from_rotation_y(yaw))
					.build()
					.child(self.arc(true))
			}))
	}
}
