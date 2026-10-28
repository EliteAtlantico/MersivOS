//! loop-record (docs/SPEC.md §4, §7, §10) has two jobs:
//! - mirror loop-bus events onto an LSL marker stream, "Loop events", so a recording made with any
//!   LSL tool holds what the wearer and Loop did, on the same clock as the signals;
//! - when asked over the bus, record every LSL stream from this machine into one XDF file per
//!   session, under `$XDG_DATA_HOME/loop/sessions` (`~/.local/share/loop/sessions`).
//!
//! Nothing leaves the device. Streams from other machines on the network are left out unless
//! `--any-host` is given.
mod xdf;

use loop_bus::Event;
use loop_lsl::{Format, Inlet, Outlet, StreamInfo};
use std::collections::HashSet;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::signal::unix::{SignalKind, signal};

fn hostname() -> String {
	std::fs::read_to_string("/proc/sys/kernel/hostname")
		.map(|s| s.trim().to_string())
		.unwrap_or_default()
}

fn sessions_dir() -> PathBuf {
	let data = std::env::var_os("XDG_DATA_HOME")
		.map(PathBuf::from)
		.or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
		.unwrap_or_else(|| PathBuf::from("."));
	data.join("loop/sessions")
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
	let any_host = std::env::args().skip(1).any(|a| a == "--any-host");
	let mut bus = loop_bus::connect("loop-record");
	let markers = StreamInfo::new(
		"Loop events",
		"Markers",
		&["event"],
		0.0,
		Format::String,
		&format!("loop-events-{}", hostname()),
	)
	.and_then(|info| Outlet::new(&info));
	let markers = match markers {
		Ok(m) => Some(m),
		Err(e) => {
			eprintln!("loop-record: {e}; events aren't mirrored and nothing can be recorded");
			None
		}
	};

	let mut recording: Option<Recording> = None;
	let mut announce = tokio::time::interval(Duration::from_secs(5));
	let mut term = signal(SignalKind::terminate()).unwrap();
	let mut int = signal(SignalKind::interrupt()).unwrap();
	loop {
		tokio::select! {
			envelope = bus.rx.recv() => {
				let Some(envelope) = envelope else { break };
				match envelope.event {
					Event::Record { on: true } if recording.is_none() && markers.is_some() => {
						match tokio::task::spawn_blocking(move || Recording::start(any_host)).await.unwrap() {
							Ok(r) => {
								eprintln!("loop-record: recording to {}", r.path.display());
								recording = Some(r);
							}
							Err(e) => eprintln!("loop-record: couldn't start recording: {e}"),
						}
						let _ = bus.tx.send(state(&recording));
					}
					Event::Record { on: false } => {
						if let Some(r) = recording.take() {
							let (path, streams) = tokio::task::spawn_blocking(move || r.stop()).await.unwrap();
							eprintln!("loop-record: saved {path}");
							let _ = bus.tx.send(Event::Recording { on: false, path: Some(path), streams });
						}
					}
					_ => {}
				}
				// Signals and traces are high-rate; their numbers are on LSL already as "Loop features"
				// and the raw streams.
				if !matches!(envelope.event, Event::Signals(_) | Event::Traces { .. })
					&& let Some(m) = &markers
				{
					m.push_str(&serde_json::to_string(&envelope).unwrap(), loop_lsl::local_clock());
				}
			}
			_ = announce.tick() => {
				let _ = bus.tx.send(state(&recording));
			}
			_ = term.recv() => break,
			_ = int.recv() => break,
		}
	}
	// Close the file properly when the session ends, so it keeps its footers.
	if let Some(r) = recording.take() {
		let (path, _) = r.stop();
		eprintln!("loop-record: saved {path}");
	}
}

fn state(recording: &Option<Recording>) -> Event {
	match recording {
		Some(r) => Event::Recording {
			on: true,
			path: Some(r.path.display().to_string()),
			streams: r.streams.lock().unwrap().clone(),
		},
		None => Event::Recording {
			on: false,
			path: None,
			streams: Vec::new(),
		},
	}
}

struct Recording {
	path: PathBuf,
	streams: Arc<Mutex<Vec<String>>>,
	stop: Arc<AtomicBool>,
	thread: JoinHandle<()>,
}

impl Recording {
	fn start(any_host: bool) -> std::io::Result<Recording> {
		let dir = sessions_dir();
		{
			use std::os::unix::fs::DirBuilderExt;
			std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
		}
		let now = chrono::Local::now();
		let path = dir.join(now.format("%Y-%m-%d_%H-%M-%S.xdf").to_string());
		let writer = xdf::Writer::new(BufWriter::new(File::create(&path)?), &now.to_rfc3339())?;
		let mut session = Session {
			writer,
			tracks: Vec::new(),
			seen: HashSet::new(),
			any_host,
			host: hostname(),
			streams: Arc::new(Mutex::new(Vec::new())),
		};
		session.add_new_streams(1.0)?;
		let streams = session.streams.clone();
		let stop = Arc::new(AtomicBool::new(false));
		let stop_flag = stop.clone();
		let thread = std::thread::spawn(move || {
			if let Err(e) = session.run(&stop_flag) {
				eprintln!("loop-record: recording stopped: {e}");
			}
		});
		Ok(Recording {
			path,
			streams,
			stop,
			thread,
		})
	}

	fn stop(self) -> (String, Vec<String>) {
		self.stop.store(true, Ordering::Relaxed);
		let _ = self.thread.join();
		let streams = self.streams.lock().unwrap().clone();
		(self.path.display().to_string(), streams)
	}
}

/// How a stream's samples are pulled and stored.
#[derive(Clone, Copy, PartialEq)]
enum Store {
	F32,
	F64,
	Str,
}

struct Track {
	id: u32,
	inlet: Inlet,
	store: Store,
	channels: usize,
	first: Option<f64>,
	last: f64,
	count: u64,
	offsets: Vec<(f64, f64)>,
}

struct Session {
	writer: xdf::Writer<BufWriter<File>>,
	tracks: Vec<Track>,
	seen: HashSet<String>,
	any_host: bool,
	host: String,
	streams: Arc<Mutex<Vec<String>>>,
}

impl Session {
	/// Opens and writes headers for streams not yet in the file.
	fn add_new_streams(&mut self, wait: f64) -> std::io::Result<()> {
		for info in loop_lsl::resolve_all(wait) {
			if self.seen.contains(&info.uid()) {
				continue;
			}
			if !self.any_host && info.hostname() != self.host {
				continue;
			}
			self.seen.insert(info.uid());
			let store = match info.format() {
				Format::Float32 => Store::F32,
				Format::String => Store::Str,
				Format::Undefined => continue,
				// Doubles and integers are stored as doubles, which hold every int32 exactly.
				_ => Store::F64,
			};
			let opened = Inlet::new(&info).and_then(|inlet| {
				inlet.open(2.0)?;
				let full = inlet.info(2.0)?;
				Ok((inlet, full))
			});
			let Ok((inlet, full)) = opened else {
				eprintln!("loop-record: couldn't open '{}'", info.name());
				continue;
			};
			let mut header = full.xml();
			if store == Store::F64 && full.format() != Format::Double64 {
				header = header.replace(
					&format!("<channel_format>{}</channel_format>", full.format().xml_name()),
					"<channel_format>double64</channel_format>",
				);
			}
			let id = self.tracks.len() as u32 + 1;
			self.writer.stream_header(id, &header)?;
			self.streams.lock().unwrap().push(full.name());
			self.tracks.push(Track {
				id,
				inlet,
				store,
				channels: full.channel_count().max(1),
				first: None,
				last: 0.0,
				count: 0,
				offsets: Vec::new(),
			});
		}
		Ok(())
	}

	fn pull_all(&mut self) -> std::io::Result<()> {
		let (mut f32s, mut f64s, mut stamps) = (Vec::new(), Vec::new(), Vec::new());
		for track in &mut self.tracks {
			let n = match track.store {
				Store::F32 => {
					let n = track.inlet.pull_f32(&mut f32s, &mut stamps, 4096).unwrap_or(0);
					self.writer.samples_f32(track.id, &stamps, &f32s, track.channels)?;
					n
				}
				Store::F64 => {
					let n = track.inlet.pull_f64(&mut f64s, &mut stamps, 4096).unwrap_or(0);
					self.writer.samples_f64(track.id, &stamps, &f64s, track.channels)?;
					n
				}
				Store::Str => {
					let samples = track.inlet.pull_strings().unwrap_or_default();
					stamps = samples.iter().map(|(t, _)| *t).collect();
					let values: Vec<Vec<String>> = samples.into_iter().map(|(_, v)| v).collect();
					self.writer.samples_str(track.id, &stamps, &values)?;
					values.len()
				}
			};
			if n > 0 {
				track.first.get_or_insert(stamps[0]);
				track.last = stamps[n - 1];
				track.count += n as u64;
			}
		}
		Ok(())
	}

	fn run(mut self, stop: &AtomicBool) -> std::io::Result<()> {
		let mut next_resolve = Instant::now() + Duration::from_secs(5);
		let mut next_offsets = Instant::now();
		let mut next_boundary = Instant::now() + Duration::from_secs(10);
		while !stop.load(Ordering::Relaxed) {
			self.pull_all()?;
			let now = Instant::now();
			if now >= next_offsets {
				next_offsets = now + Duration::from_secs(5);
				for track in &mut self.tracks {
					let collected = loop_lsl::local_clock();
					if let Ok(offset) = track.inlet.time_correction(1.0) {
						// As LabRecorder does: the collection time on the stream's own clock.
						self.writer.clock_offset(track.id, collected - offset, offset)?;
						track.offsets.push((collected - offset, offset));
					}
				}
			}
			if now >= next_boundary {
				next_boundary = now + Duration::from_secs(10);
				self.writer.boundary()?;
				self.writer.flush()?;
			}
			if now >= next_resolve {
				next_resolve = now + Duration::from_secs(5);
				self.add_new_streams(0.5)?;
			}
			std::thread::sleep(Duration::from_millis(50));
		}
		self.pull_all()?;
		for track in &self.tracks {
			let first = track.first.unwrap_or(0.0);
			self.writer
				.stream_footer(track.id, first, track.last, track.count, &track.offsets)?;
		}
		self.writer.flush()
	}
}
