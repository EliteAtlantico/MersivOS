//! The loop bus (docs/SPEC.md §4): discrete events between Loop's services, one JSON object per
//! line over a Unix socket. The `loop-bus` hub repeats every line to every other connected
//! service. Sensor samples stay on LSL; only events and small summaries travel here.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

/// `$LOOP_BUS`, else `$XDG_RUNTIME_DIR/loop/bus.sock`.
pub fn socket_path() -> PathBuf {
	if let Some(path) = std::env::var_os("LOOP_BUS") {
		return path.into();
	}
	let dir = std::env::var_os("XDG_RUNTIME_DIR")
		.map(PathBuf::from)
		.unwrap_or_else(std::env::temp_dir);
	dir.join("loop").join("bus.sock")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
	pub from: String,
	pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
	/// loop-sense, about four times a second: what it reads and how well.
	Signals(Signals),
	/// loop-sense: new samples since the last message, band-passed and decimated, for live traces.
	Traces { rate: f32, samples: Vec<Vec<f32>> },
	/// A confirm from any source. loop-ring applies it to whatever the wearer is looking at.
	Confirm { source: ConfirmSource },
	/// loop-ring: something the wearer did, or something Loop did on its own. Recorded as a marker.
	Ui { what: String, source: Option<ConfirmSource> },
	/// Ask loop-record to start or stop recording the session.
	Record { on: bool },
	/// loop-record: whether a recording is running, where it goes and what it holds.
	Recording { on: bool, path: Option<String>, streams: Vec<String> },
	/// Ask loop-sense to learn the wearer's confirm gesture (SPEC §6: "clench twice").
	Calibrate,
	/// loop-sense: calibration progress.
	Calibration { done: u8, of: u8, finished: bool },
	/// Drives the synthetic EEG source (test harness only).
	Sim(SimCmd),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signals {
	/// LSL stream name, e.g. "Muse-EEG (…)" or "Loop sim EEG". `None` while nothing is connected.
	pub source: Option<String>,
	pub rate: f32,
	pub channels: Vec<Channel>,
	/// 0 = very relaxed, 1 = very focused, against the wearer's own baseline. `None` until the
	/// baseline is learned.
	pub focus: Option<f32>,
	/// Progress of learning that baseline, 0–1.
	pub baseline: f32,
	/// Confirm gestures the montage and current signal quality can support.
	pub confirms: Vec<ConfirmSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channel {
	pub name: String,
	pub good: bool,
	/// Why a channel isn't good, in plain words ("no contact", "noisy", "mains hum").
	pub issue: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmSource {
	Keyboard,
	Controller,
	Voice,
	/// Jaw clench, picked up by the temporal sensors (muscle, not brain activity).
	Clench,
	/// Two blinks in quick succession, picked up by the frontal sensors (eye, not brain activity).
	DoubleBlink,
	/// Timed actions from `LOOP_SCRIPT`.
	Script,
}
impl ConfirmSource {
	/// Whether this confirm comes from body signals, and so needs the listening mark (SPEC §6).
	pub fn is_sensed(self) -> bool {
		matches!(self, ConfirmSource::Clench | ConfirmSource::DoubleBlink)
	}
	pub fn word(self) -> &'static str {
		match self {
			ConfirmSource::Keyboard => "keyboard",
			ConfirmSource::Controller => "controller",
			ConfirmSource::Voice => "voice",
			ConfirmSource::Clench => "clench",
			ConfirmSource::DoubleBlink => "double blink",
			ConfirmSource::Script => "script",
		}
	}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum SimCmd {
	Clench,
	Blink,
	DoubleBlink,
	/// Hold focus at a value, or `None` to let it drift again.
	Focus { value: Option<f32> },
	/// Simulate a sensor losing or regaining skin contact.
	Contact { channel: String, good: bool },
}

/// A connection to the bus. Sends are dropped while the hub is unreachable; the connection is
/// retried every second, so services can start in any order.
pub struct Bus {
	pub tx: mpsc::UnboundedSender<Event>,
	pub rx: mpsc::UnboundedReceiver<Envelope>,
}

/// Connects in the background on the current tokio runtime.
pub fn connect(name: &str) -> Bus {
	let (tx, mut out_rx) = mpsc::unbounded_channel::<Event>();
	let (in_tx, rx) = mpsc::unbounded_channel::<Envelope>();
	let name = name.to_string();
	tokio::spawn(async move {
		let path = socket_path();
		loop {
			let Ok(stream) = UnixStream::connect(&path).await else {
				tokio::time::sleep(Duration::from_secs(1)).await;
				// Drop what was queued while disconnected rather than replaying stale events.
				while out_rx.try_recv().is_ok() {}
				continue;
			};
			let (read, mut write) = stream.into_split();
			let mut lines = BufReader::new(read).lines();
			loop {
				tokio::select! {
					line = lines.next_line() => {
						let Ok(Some(line)) = line else { break };
						if let Ok(envelope) = serde_json::from_str::<Envelope>(&line) {
							if in_tx.send(envelope).is_err() {
								return;
							}
						}
					}
					event = out_rx.recv() => {
						let Some(event) = event else { return };
						let mut line = serde_json::to_string(&Envelope { from: name.clone(), event }).unwrap();
						line.push('\n');
						if write.write_all(line.as_bytes()).await.is_err() {
							break;
						}
					}
				}
			}
		}
	});
	Bus { tx, rx }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn events_round_trip_as_tagged_json() {
		let e = Envelope {
			from: "loop-sense".into(),
			event: Event::Sim(SimCmd::Focus { value: Some(0.8) }),
		};
		let line = serde_json::to_string(&e).unwrap();
		assert_eq!(
			line,
			r#"{"from":"loop-sense","event":{"type":"sim","do":"focus","value":0.8}}"#
		);
		assert_eq!(serde_json::from_str::<Envelope>(&line).unwrap(), e);
		let c = Event::Confirm { source: ConfirmSource::DoubleBlink };
		assert_eq!(
			serde_json::to_string(&c).unwrap(),
			r#"{"type":"confirm","source":"double_blink"}"#
		);
	}
}
