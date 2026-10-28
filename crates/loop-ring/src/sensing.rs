//! What loop-sense and loop-record report, as the ring holds it: the latest signal summary, the live
//! traces, calibration and recording state, and the passive adaptation policy (loop-adapt, SPEC §7)
//! that turns sustained focus into a reversible mark on the arc.
use loop_bus::{Event, Signals};
use std::collections::VecDeque;

/// A summary older than this means loop-sense has stopped.
const STALE: f32 = 2.0;
const TRACE_SECONDS: f32 = 2.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Adaptation {
	pub reason: String,
	pub since: f32,
}

#[derive(Debug, Default)]
pub struct Sensing {
	latest: Option<(Signals, f32)>,
	pub traces: Vec<VecDeque<f32>>,
	/// `(done, of)` while the wearer is calibrating their clench.
	pub calibration: Option<(u8, u8)>,
	/// Where the running recording goes.
	pub recording: Option<String>,
	focus_high_since: Option<f32>,
	focus_low_since: Option<f32>,
	focus_missing_since: Option<f32>,
	pub adaptation: Option<Adaptation>,
	reversed_at: Option<f32>,
}

impl Sensing {
	pub fn on_event(&mut self, event: &Event, now: f32) {
		match event {
			Event::Signals(s) => {
				if s.source.is_none() {
					self.traces.clear();
				}
				self.latest = Some((s.clone(), now));
			}
			Event::Traces { rate, samples } => {
				let keep = (rate * TRACE_SECONDS) as usize;
				self.traces.resize_with(samples.len(), VecDeque::new);
				for (trace, new) in self.traces.iter_mut().zip(samples) {
					trace.extend(new);
					while trace.len() > keep {
						trace.pop_front();
					}
				}
			}
			Event::Calibration { done, of, finished } => {
				self.calibration = (!finished).then_some((*done, *of));
			}
			Event::Recording { on, path, .. } => {
				self.recording = if *on { path.clone().or(Some(String::new())) } else { None };
			}
			_ => {}
		}
	}

	/// The latest summary, if loop-sense is reading something right now.
	pub fn signals(&self, now: f32) -> Option<&Signals> {
		self.latest
			.as_ref()
			.filter(|(s, at)| now - at < STALE && s.source.is_some())
			.map(|(s, _)| s)
	}

	/// `(good, total)` channels.
	pub fn quality(&self, now: f32) -> (usize, usize) {
		self.signals(now).map_or((0, 0), |s| {
			(s.channels.iter().filter(|c| c.good).count(), s.channels.len())
		})
	}

	/// A short name for the arc: "Muse", "Sim EEG", or the stream's own name.
	pub fn label(&self, now: f32) -> String {
		match self.signals(now).and_then(|s| s.source.as_deref()) {
			None => "No EEG".into(),
			Some(name) if name.starts_with("Muse") => "Muse".into(),
			Some("Loop sim EEG") => "Sim EEG".into(),
			Some(name) => name.chars().take(14).collect(),
		}
	}

	/// Whether a body-signal confirm (clench, double blink) can be heard right now.
	pub fn armed(&self, now: f32) -> bool {
		self.signals(now)
			.is_some_and(|s| s.confirms.iter().any(|c| c.is_sensed()))
	}

	/// Runs the adaptation policy. Returns what changed, for the event log.
	pub fn step(&mut self, now: f32) -> Option<String> {
		let focus = self.signals(now).and_then(|s| s.focus);
		let Some(focus) = focus else {
			// Without a focus reading the adaptation can't be kept honest: let it go.
			let since = *self.focus_missing_since.get_or_insert(now);
			self.focus_high_since = None;
			self.focus_low_since = None;
			if now - since > 10.0 && self.adaptation.take().is_some() {
				return Some("adaptation released: no focus reading".into());
			}
			return None;
		};
		self.focus_missing_since = None;
		if focus > 0.75 {
			self.focus_high_since.get_or_insert(now);
			self.focus_low_since = None;
		} else if focus < 0.4 {
			self.focus_low_since.get_or_insert(now);
			self.focus_high_since = None;
		} else {
			self.focus_high_since = None;
			self.focus_low_since = None;
		}
		let recently_reversed = self.reversed_at.is_some_and(|t| now - t < 60.0);
		if self.adaptation.is_none()
			&& !recently_reversed
			&& self.focus_high_since.is_some_and(|t| now - t > 6.0)
		{
			let reason = "Sounds held · focus".to_string();
			self.adaptation = Some(Adaptation {
				reason: reason.clone(),
				since: now,
			});
			return Some(format!("adaptation shown: {reason}"));
		}
		// A natural break releases the held sounds on its own.
		if self.focus_low_since.is_some_and(|t| now - t > 4.0) && self.adaptation.take().is_some() {
			return Some("adaptation released: a break".into());
		}
		None
	}

	/// The wearer reversed the adaptation from its mark on the arc.
	pub fn reverse_adaptation(&mut self, now: f32) {
		self.adaptation = None;
		self.reversed_at = Some(now);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use loop_bus::{Channel, ConfirmSource};

	fn signals(focus: Option<f32>) -> Event {
		Event::Signals(Signals {
			source: Some("Muse-EEG (test)".into()),
			rate: 256.0,
			channels: vec![
				Channel { name: "TP9".into(), good: true, issue: None },
				Channel { name: "TP10".into(), good: false, issue: Some("no signal".into()) },
			],
			focus,
			baseline: 1.0,
			confirms: vec![ConfirmSource::DoubleBlink],
		})
	}

	#[test]
	fn summary_goes_stale() {
		let mut s = Sensing::default();
		s.on_event(&signals(Some(0.5)), 10.0);
		assert_eq!(s.label(10.5), "Muse");
		assert_eq!(s.quality(10.5), (1, 2));
		assert!(s.armed(10.5));
		assert_eq!(s.label(13.0), "No EEG");
		assert!(!s.armed(13.0));
	}

	#[test]
	fn sustained_focus_holds_sounds_and_a_break_releases_them() {
		let mut s = Sensing::default();
		let mut log = Vec::new();
		let mut t = 0.0;
		let mut feed = |s: &mut Sensing, focus: f32, seconds: f32, log: &mut Vec<String>| {
			let end = t + seconds;
			while t < end {
				s.on_event(&signals(Some(focus)), t);
				log.extend(s.step(t));
				t += 0.25;
			}
		};
		feed(&mut s, 0.9, 5.0, &mut log);
		assert!(s.adaptation.is_none());
		feed(&mut s, 0.9, 3.0, &mut log);
		assert!(s.adaptation.is_some());
		feed(&mut s, 0.2, 5.0, &mut log);
		assert!(s.adaptation.is_none());
		assert_eq!(log, ["adaptation shown: Sounds held · focus", "adaptation released: a break"]);
	}

	#[test]
	fn a_reversed_adaptation_stays_away_for_a_minute() {
		let mut s = Sensing::default();
		for i in 0..40 {
			let t = i as f32 * 0.25;
			s.on_event(&signals(Some(0.9)), t);
			s.step(t);
		}
		assert!(s.adaptation.is_some());
		s.reverse_adaptation(10.0);
		for i in 0..200 {
			let t = 10.0 + i as f32 * 0.25;
			s.on_event(&signals(Some(0.9)), t);
			s.step(t);
		}
		assert!(s.adaptation.is_none(), "still within the minute");
		for i in 0..60 {
			let t = 70.0 + i as f32 * 0.25;
			s.on_event(&signals(Some(0.9)), t);
			s.step(t);
		}
		assert!(s.adaptation.is_some());
	}
}
