//! loop-sense (docs/SPEC.md §7): reads EEG over LSL, works out signal quality, focus and the confirm
//! gestures, and tells the rest of Loop over the loop bus. The features also go back out on LSL as
//! "Loop features", so a recording holds them beside the raw signal on the same clock.
//!
//!   loop-sense                  the first Muse EEG stream, else any other EEG stream
//!   loop-sense --stream NAME    the first EEG stream whose name contains NAME
//!   loop-sense --sim            start a synthetic headband and read that
mod dsp;
mod features;
mod synth;

use features::Pipeline;
use loop_bus::{Event, SimCmd, Signals};
use loop_lsl::{Format, Inlet, Outlet, StreamInfo};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use synth::Synth;
use tokio::sync::mpsc::UnboundedSender;

const SIM_NAME: &str = "Loop sim EEG";
const SIM_ID: &str = "loop-sim-eeg";

struct Args {
	sim: bool,
	stream: Option<String>,
}

fn parse_args() -> Args {
	let mut args = Args {
		sim: false,
		stream: None,
	};
	let mut it = std::env::args().skip(1);
	while let Some(a) = it.next() {
		match a.as_str() {
			"--sim" => args.sim = true,
			"--stream" => args.stream = it.next(),
			_ => {
				eprintln!("usage: loop-sense [--sim] [--stream NAME]");
				std::process::exit(2);
			}
		}
	}
	args
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
	let args = parse_args();
	let mut bus = loop_bus::connect("loop-sense");
	match loop_lsl::version() {
		Some(v) => eprintln!("loop-sense: liblsl {}.{}", v / 100, v % 100),
		None => eprintln!("loop-sense: liblsl not found (set LOOP_LIBLSL); only --sim works, and nothing reaches LSL"),
	}
	let (cmd_tx, cmd_rx) = mpsc::channel::<Event>();
	let (sim_tx, sim_rx) = mpsc::channel::<SimCmd>();
	let direct = args.sim.then(|| run_synth(sim_rx)).flatten();
	let out = bus.tx.clone();
	std::thread::spawn(move || sense(args, cmd_rx, direct, out));
	while let Some(envelope) = bus.rx.recv().await {
		match envelope.event {
			Event::Calibrate => {
				let _ = cmd_tx.send(Event::Calibrate);
			}
			Event::Sim(cmd) => {
				let _ = sim_tx.send(cmd);
			}
			_ => {}
		}
	}
}

type Samples = mpsc::Receiver<(Vec<f32>, f64)>;

/// Runs the synthetic headband in real time. It publishes on LSL like a real band; without liblsl
/// its samples come back through the returned channel instead.
fn run_synth(cmds: mpsc::Receiver<SimCmd>) -> Option<Samples> {
	let outlet = StreamInfo::new(SIM_NAME, "EEG", &synth::LABELS, synth::RATE, Format::Float32, SIM_ID)
		.and_then(|info| Outlet::new(&info))
		.ok();
	let (tx, rx) = mpsc::channel();
	let direct = outlet.is_none();
	std::thread::spawn(move || {
		const CHUNK: usize = 8;
		let mut synth = Synth::default();
		let start = Instant::now();
		let mut produced = 0usize;
		loop {
			while let Ok(cmd) = cmds.try_recv() {
				synth.command(&cmd);
			}
			let rows: Vec<[f32; 4]> = (0..CHUNK).map(|_| synth.sample()).collect();
			produced += CHUNK;
			let now = loop_lsl::local_clock();
			match &outlet {
				Some(outlet) => outlet.push_f32(rows.as_flattened(), now),
				None => {
					for (i, row) in rows.iter().enumerate() {
						let t = now - (CHUNK - 1 - i) as f64 / synth::RATE;
						if tx.send((row.to_vec(), t)).is_err() {
							return;
						}
					}
				}
			}
			let due = start + Duration::from_secs_f64(produced as f64 / synth::RATE);
			std::thread::sleep(due.saturating_duration_since(Instant::now()));
		}
	});
	direct.then_some(rx)
}

enum Source {
	Lsl(Inlet),
	Direct(Samples, usize),
}
impl Source {
	/// Whatever samples are ready, channel-interleaved, with one time stamp each.
	fn pull(&mut self, data: &mut Vec<f32>, stamps: &mut Vec<f64>) -> Result<usize, String> {
		match self {
			Source::Lsl(inlet) => inlet.pull_f32(data, stamps, 1024).map_err(|e| e.to_string()),
			Source::Direct(rx, _) => {
				data.clear();
				stamps.clear();
				loop {
					match rx.try_recv() {
						Ok((row, t)) => {
							data.extend(row);
							stamps.push(t);
						}
						Err(mpsc::TryRecvError::Empty) => return Ok(stamps.len()),
						Err(mpsc::TryRecvError::Disconnected) => return Err("synthetic source stopped".into()),
					}
				}
			}
		}
	}
}

/// The EEG stream to read: the one asked for, else a Muse, else any other EEG stream.
fn find(args: &Args) -> Option<StreamInfo> {
	let mut eeg: Vec<StreamInfo> = loop_lsl::resolve_all(1.0)
		.into_iter()
		.filter(|s| s.kind().eq_ignore_ascii_case("EEG"))
		.collect();
	let rank = |s: &StreamInfo| {
		if args.sim {
			return (s.source_id() == SIM_ID) as u8 * 3;
		}
		if let Some(want) = &args.stream {
			return s.name().contains(want.as_str()) as u8 * 3;
		}
		match (s.name().contains("Muse"), s.source_id() == SIM_ID) {
			(true, _) => 3,
			(false, false) => 2,
			(false, true) => 1,
		}
	};
	eeg.sort_by_key(|s| std::cmp::Reverse(rank(s)));
	eeg.into_iter().next().filter(|s| rank(s) > 0)
}

fn nothing_connected() -> Event {
	Event::Signals(Signals {
		source: None,
		rate: 0.0,
		channels: Vec::new(),
		focus: None,
		baseline: 0.0,
		confirms: Vec::new(),
	})
}

fn sense(args: Args, cmds: mpsc::Receiver<Event>, direct: Option<Samples>, bus: UnboundedSender<Event>) {
	let mut direct = direct;
	loop {
		let (source, name, rate, labels, channels) = if let Some(rx) = direct.take() {
			let labels = synth::LABELS.map(String::from).to_vec();
			(Source::Direct(rx, 4), SIM_NAME.to_string(), synth::RATE, labels, 4)
		} else {
			let Some(info) = find(&args) else {
				let _ = bus.send(nothing_connected());
				while let Ok(cmd) = cmds.try_recv() {
					if cmd == Event::Calibrate {
						let _ = bus.send(Event::Calibration { done: 0, of: 2, finished: true });
					}
				}
				continue;
			};
			let opened = Inlet::new(&info).and_then(|inlet| {
				inlet.open(5.0)?;
				let full = inlet.info(5.0)?;
				Ok((inlet, full))
			});
			let (inlet, full) = match opened {
				Ok(v) => v,
				Err(e) => {
					eprintln!("loop-sense: couldn't open '{}': {e}", info.name());
					std::thread::sleep(Duration::from_secs(1));
					continue;
				}
			};
			let labels = full.channel_labels();
			(Source::Lsl(inlet), full.name(), full.rate(), labels, full.channel_count())
		};
		if let Source::Direct(_, n) = &source {
			debug_assert_eq!(*n, channels);
		}
		read(source, &name, rate, &labels, channels, &cmds, &bus);
		let _ = bus.send(nothing_connected());
	}
}

/// Reads one stream until it goes quiet for three seconds.
fn read(
	mut source: Source,
	name: &str,
	rate: f64,
	labels: &[String],
	channels: usize,
	cmds: &mpsc::Receiver<Event>,
	bus: &UnboundedSender<Event>,
) {
	let mut pipe = Pipeline::new(name, rate, labels, channels);
	eprintln!(
		"loop-sense: reading '{name}' ({channels} channels at {rate} Hz; using {})",
		pipe.channel_names().join(", ")
	);
	let feature_labels: Vec<String> = std::iter::once("focus".to_string())
		.chain(pipe.channel_names().iter().map(|n| format!("{n} good")))
		.collect();
	let feature_labels: Vec<&str> = feature_labels.iter().map(String::as_str).collect();
	let features = StreamInfo::new(
		"Loop features",
		"Features",
		&feature_labels,
		1.0 / features::TICK,
		Format::Float32,
		&format!("loop-features-{name}"),
	)
	.and_then(|info| Outlet::new(&info))
	.ok();

	let (mut data, mut stamps) = (Vec::new(), Vec::new());
	let mut last_data = Instant::now();
	let mut last_traces = Instant::now();
	let mut last_t = loop_lsl::local_clock();
	loop {
		match source.pull(&mut data, &mut stamps) {
			Ok(n) if n > 0 => {
				last_data = Instant::now();
				for (row, t) in data.chunks(channels).zip(&stamps) {
					last_t = *t;
					for event in pipe.push(row, *t) {
						if let (Event::Signals(_), Some(outlet)) = (&event, &features) {
							outlet.push_f32(&pipe.feature_row(), *t);
						}
						let _ = bus.send(event);
					}
				}
			}
			Ok(_) => {}
			Err(e) => {
				eprintln!("loop-sense: '{name}': {e}");
				return;
			}
		}
		if last_data.elapsed() > Duration::from_secs(3) {
			eprintln!("loop-sense: '{name}' went quiet");
			return;
		}
		if last_traces.elapsed() >= Duration::from_millis(100) {
			last_traces = Instant::now();
			let _ = bus.send(pipe.take_traces());
		}
		while let Ok(cmd) = cmds.try_recv() {
			if cmd == Event::Calibrate {
				let _ = bus.send(pipe.calibrate(last_t));
			}
		}
		std::thread::sleep(Duration::from_millis(15));
	}
}
