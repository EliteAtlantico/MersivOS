//! From raw samples to what Loop uses (SPEC §6, §7):
//! - signal quality per channel, in plain words;
//! - a focus index: relative alpha against the wearer's own baseline, the method of
//!   `~/Projects/muse-alpha-motor/attention.py`;
//! - the confirm gestures a forehead band supports: a held jaw clench on the temporal sensors and a
//!   double blink on the frontal ones. Both are muscle and eye signals, not brain activity.
use crate::dsp::{Chain, History, Welch, mean, median, ptp, std};
use loop_bus::{Channel, ConfirmSource, Event, Signals};
use std::collections::VecDeque;

/// Features are refreshed this often (seconds of signal time).
pub const TICK: f64 = 0.25;
const WINDOW: f64 = 2.0;
const BASELINE_SECONDS: f64 = 20.0;
const CALIBRATION_SECONDS: f64 = 20.0;
const TRACE_RATE: f64 = 64.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
	/// Over the temporalis: picks up jaw clenches.
	Muscle,
	/// Near the eyes: picks up blinks.
	Ocular,
	Other,
}

/// What a channel label says about where it sits; `None` for non-EEG channels such as Muse's AUX.
fn role(label: &str) -> Option<Role> {
	let l = label.to_ascii_uppercase();
	if l.starts_with("AUX") {
		return None;
	}
	Some(match l.as_str() {
		"TP9" | "TP10" | "TP7" | "TP8" | "T7" | "T8" | "T3" | "T4" | "FT9" | "FT10" => Role::Muscle,
		"AF7" | "AF8" | "AF3" | "AF4" | "AFZ" | "FP1" | "FP2" | "FPZ" => Role::Ocular,
		_ => Role::Other,
	})
}

struct ChannelState {
	name: String,
	index: usize,
	role: Role,
	offset: Option<f64>,
	/// 1–30 Hz: quality, focus and the live trace.
	band: Chain,
	/// Above 30 Hz with mains notched out: muscle.
	emg: Chain,
	/// 0.1–8 Hz: eye movements and blinks.
	eog: Chain,
	/// Above 0.5 Hz, unfiltered otherwise: for spotting mains hum.
	wide: Chain,
	band_hist: History,
	wide_hist: History,
	emg_power: f64,
	eog_value: f64,
	last_band: f64,
	votes: VecDeque<bool>,
	issue: Option<&'static str>,
	good: bool,
}

pub struct Pipeline {
	fs: f64,
	source: String,
	channels: Vec<ChannelState>,
	welch: Welch,
	focus: Focus,
	clench: Clench,
	clench_ratio: f64,
	blink: Blink,
	calibration: Option<Calibration>,
	emg_alpha: f64,
	decimate: usize,
	since_trace: usize,
	traces: Vec<Vec<f32>>,
	next_tick: Option<f64>,
}

impl Pipeline {
	/// `labels` are the stream's channel labels; unlabelled streams get quality and focus only.
	pub fn new(source: &str, fs: f64, labels: &[String], channel_count: usize) -> Self {
		let window = (WINDOW * fs).round() as usize;
		let channels: Vec<ChannelState> = (0..channel_count)
			.filter_map(|index| {
				let name = labels
					.get(index)
					.filter(|l| !l.is_empty())
					.cloned()
					.unwrap_or_else(|| format!("Ch{}", index + 1));
				let role = role(&name)?;
				Some(ChannelState {
					name,
					index,
					role,
					offset: None,
					band: Chain::default().highpass(fs, 1.0, 4).lowpass(fs, 30.0, 4),
					emg: Chain::default()
						.notch(fs, 50.0)
						.notch(fs, 60.0)
						.notch(fs, 100.0)
						.notch(fs, 120.0)
						.highpass(fs, 30.0, 4),
					eog: Chain::default().highpass(fs, 0.1, 2).lowpass(fs, 8.0, 2),
					wide: Chain::default().highpass(fs, 0.5, 2),
					band_hist: History::new(window),
					wide_hist: History::new(window),
					emg_power: 0.0,
					eog_value: 0.0,
					last_band: 0.0,
					votes: VecDeque::new(),
					issue: None,
					good: false,
				})
			})
			.collect();
		let traces = vec![Vec::new(); channels.len()];
		Pipeline {
			fs,
			source: source.to_string(),
			welch: Welch::new(fs, fs.round() as usize),
			focus: Focus::default(),
			clench: Clench::new(fs),
			clench_ratio: Clench::DEFAULT_RATIO,
			blink: Blink::new(fs),
			calibration: None,
			emg_alpha: 1.0 / (0.05 * fs),
			decimate: (fs / TRACE_RATE).round().max(1.0) as usize,
			since_trace: 0,
			traces,
			channels,
			next_tick: None,
		}
	}

	pub fn channel_names(&self) -> Vec<String> {
		self.channels.iter().map(|c| c.name.clone()).collect()
	}

	/// Start learning the wearer's clench (SPEC §6: inline, "clench twice").
	pub fn calibrate(&mut self, t: f64) -> Event {
		self.calibration = Some(Calibration {
			peaks: Vec::new(),
			until: t + CALIBRATION_SECONDS,
		});
		Event::Calibration {
			done: 0,
			of: 2,
			finished: false,
		}
	}

	/// One sample (all channels of the stream) at time `t`. Returns gesture events, and a
	/// `Signals` summary whenever a tick has passed.
	pub fn push(&mut self, sample: &[f32], t: f64) -> Vec<Event> {
		let mut events = Vec::new();
		for ch in &mut self.channels {
			let v = sample.get(ch.index).copied().unwrap_or(f32::NAN) as f64;
			if !v.is_finite() {
				continue;
			}
			// Start the filters near zero rather than ringing from a large DC offset.
			let x = v - *ch.offset.get_or_insert(v);
			ch.last_band = ch.band.process(x);
			ch.band_hist.push(ch.last_band);
			ch.wide_hist.push(ch.wide.process(x));
			let e = ch.emg.process(x);
			ch.emg_power += (e * e - ch.emg_power) * self.emg_alpha;
			ch.eog_value = ch.eog.process(x);
		}

		let muscle: Vec<f64> = self
			.channels
			.iter()
			.filter(|c| c.good && c.role == Role::Muscle)
			.map(|c| c.emg_power)
			.collect();
		if !muscle.is_empty() {
			let ratio = if self.calibration.is_some() { 3.0 } else { self.clench_ratio };
			if let Some(peak) = self.clench.update(mean(&muscle).sqrt(), t, ratio) {
				events.extend(self.on_clench(peak));
			}
		}
		let ocular: Vec<f64> = self
			.channels
			.iter()
			.filter(|c| c.good && c.role == Role::Ocular)
			.map(|c| c.eog_value)
			.collect();
		if !ocular.is_empty() && self.blink.update(mean(&ocular), t) == Some(true) && self.calibration.is_none() {
			events.push(Event::Confirm {
				source: ConfirmSource::DoubleBlink,
			});
		}

		self.since_trace += 1;
		if self.since_trace >= self.decimate {
			self.since_trace = 0;
			for (trace, ch) in self.traces.iter_mut().zip(&self.channels) {
				trace.push((ch.last_band * 10.0).round() as f32 / 10.0);
			}
		}

		let mut next = *self.next_tick.get_or_insert(t + TICK);
		if t >= next {
			while next <= t {
				next += TICK;
			}
			self.next_tick = Some(next);
			events.extend(self.tick(t));
		}
		events
	}

	fn on_clench(&mut self, peak: f64) -> Vec<Event> {
		let Some(cal) = &mut self.calibration else {
			return vec![Event::Confirm {
				source: ConfirmSource::Clench,
			}];
		};
		cal.peaks.push(peak);
		let done = cal.peaks.len() as u8;
		if done < 2 {
			return vec![Event::Calibration {
				done,
				of: 2,
				finished: false,
			}];
		}
		// Trigger at 40% of the way from rest to the wearer's own clench.
		self.clench_ratio = (1.0 + 0.4 * (median(&cal.peaks) - 1.0)).clamp(3.0, 30.0);
		self.calibration = None;
		vec![Event::Calibration {
			done,
			of: 2,
			finished: true,
		}]
	}

	fn tick(&mut self, t: f64) -> Vec<Event> {
		let mut events = Vec::new();
		for ch in &mut self.channels {
			if !ch.band_hist.full() {
				continue;
			}
			let band = ch.band_hist.values();
			let issue = if std(&band) < 0.3 {
				Some("no signal")
			} else if ptp(&band) > 500.0 {
				Some("noisy")
			} else {
				let psd = self.welch.psd(&ch.wide_hist.values());
				let hum = [50.0, 60.0]
					.map(|f| self.welch.band(&psd, f - 1.0, f + 1.0))
					.into_iter()
					.fold(0.0, f64::max);
				(hum > self.welch.band(&psd, 1.0, 30.0)).then_some("mains hum")
			};
			ch.votes.push_back(issue.is_none());
			if ch.votes.len() > 8 {
				ch.votes.pop_front();
			}
			if issue.is_some() {
				ch.issue = issue;
			}
			// A blink or a clench spoils a window or two; contact problems persist.
			let good_votes = ch.votes.iter().filter(|v| **v).count();
			ch.good = good_votes * 8 >= ch.votes.len() * 5;
		}

		let windows: Vec<Vec<f64>> = self
			.channels
			.iter()
			.filter(|c| c.good)
			.map(|c| c.band_hist.values())
			.filter(|w| {
				let p = ptp(w);
				p > 0.5 && p < 200.0
			})
			.collect();
		self.focus.update(&windows, &self.welch, t);
		self.blink.refresh();

		if let Some(cal) = &self.calibration
			&& t > cal.until
		{
			events.push(Event::Calibration {
				done: cal.peaks.len() as u8,
				of: 2,
				finished: true,
			});
			self.calibration = None;
		}

		let any = |role: Role| self.channels.iter().any(|c| c.good && c.role == role);
		let mut confirms = Vec::new();
		if any(Role::Muscle) && self.clench.ready() {
			confirms.push(ConfirmSource::Clench);
		}
		if any(Role::Ocular) && self.blink.ready() {
			confirms.push(ConfirmSource::DoubleBlink);
		}
		events.push(Event::Signals(Signals {
			source: Some(self.source.clone()),
			rate: self.fs as f32,
			channels: self
				.channels
				.iter()
				.map(|c| Channel {
					name: c.name.clone(),
					good: c.good,
					issue: (!c.good).then(|| c.issue.unwrap_or("settling").to_string()),
				})
				.collect(),
			focus: self.focus.value().map(|f| f as f32),
			baseline: self.focus.progress(t) as f32,
			confirms,
		}));
		events
	}

	/// The trace samples gathered since the last call, one list per channel.
	pub fn take_traces(&mut self) -> Event {
		Event::Traces {
			rate: (self.fs / self.decimate as f64) as f32,
			samples: self.traces.iter_mut().map(std::mem::take).collect(),
		}
	}

	/// Per channel 1 = good, then focus (NaN until the baseline is learned): the feature stream
	/// loop-sense publishes on LSL so recordings hold it.
	pub fn feature_row(&self) -> Vec<f32> {
		std::iter::once(self.focus.value().map_or(f32::NAN, |f| f as f32))
			.chain(self.channels.iter().map(|c| c.good as u8 as f32))
			.collect()
	}
}

/// Relative alpha (8–12 Hz over 1–30 Hz) against the wearer's baseline; focus rises as alpha is
/// suppressed. Baseline: the first 20 s, then a slow drift so sweat and settling don't pin it.
#[derive(Default)]
struct Focus {
	started: Option<f64>,
	learning: Vec<f64>,
	mu: Option<f64>,
	sigma: f64,
	value: f64,
}
impl Focus {
	const GAIN: f64 = 1.5;
	const SMOOTHING: f64 = 0.15;
	const DRIFT: f64 = 0.002;

	fn value(&self) -> Option<f64> {
		self.mu.map(|_| self.value)
	}
	fn progress(&self, t: f64) -> f64 {
		if self.mu.is_some() {
			return 1.0;
		}
		self.started.map_or(0.0, |s| ((t - s) / BASELINE_SECONDS).clamp(0.0, 1.0))
	}
	fn update(&mut self, windows: &[Vec<f64>], welch: &Welch, t: f64) {
		if self.mu.is_none()
			&& self.progress(t) >= 1.0
			&& self.learning.len() >= 5
		{
			self.mu = Some(mean(&self.learning));
			self.sigma = std(&self.learning).max(0.15);
			self.value = 0.5;
		}
		if windows.is_empty() {
			return;
		}
		let rel = mean(
			&windows
				.iter()
				.map(|w| {
					let psd = welch.psd(w);
					welch.band(&psd, 8.0, 12.0) / welch.band(&psd, 1.0, 30.0).max(1e-12)
				})
				.collect::<Vec<_>>(),
		);
		if !rel.is_finite() {
			return;
		}
		let v = rel.max(1e-6).ln();
		let Some(mu) = &mut self.mu else {
			self.started.get_or_insert(t);
			self.learning.push(v);
			return;
		};
		let z = (v - *mu) / self.sigma;
		let target = 1.0 / (1.0 + (Self::GAIN * z).exp());
		self.value += Self::SMOOTHING * (target - self.value);
		*mu += Self::DRIFT * (v - *mu);
	}
}

/// A held jaw clench: muscle power above the resting level for 0.15–1.5 s, reported on release.
struct Clench {
	rate: f64,
	warmup: Vec<f64>,
	baseline: Option<f64>,
	active: Option<(f64, f64)>,
	last: f64,
}
impl Clench {
	const DEFAULT_RATIO: f64 = 5.0;

	fn new(rate: f64) -> Self {
		Clench {
			rate,
			warmup: Vec::new(),
			baseline: None,
			active: None,
			last: f64::NEG_INFINITY,
		}
	}
	fn ready(&self) -> bool {
		self.baseline.is_some()
	}
	/// `e`: muscle-band RMS in µV. Returns the peak as a multiple of rest when a clench ends.
	fn update(&mut self, e: f64, t: f64, ratio: f64) -> Option<f64> {
		let Some(base) = self.baseline else {
			self.warmup.push(e);
			if self.warmup.len() as f64 >= self.rate {
				self.baseline = Some(median(&self.warmup).max(0.2));
				self.warmup.clear();
			}
			return None;
		};
		match self.active {
			Some((start, peak)) => {
				let peak = peak.max(e);
				self.active = Some((start, peak));
				let held = t - start;
				if e < base * (1.0 + (ratio - 1.0) * 0.5) {
					self.active = None;
					if (0.15..=1.5).contains(&held) && t - self.last > 0.6 {
						self.last = t;
						return Some(peak / base);
					}
				} else if held > 3.0 {
					// Too long for a gesture: the resting level itself has changed (talking,
					// chewing, the band shifting). Start again from here; it settles back down.
					self.active = None;
					self.baseline = Some(e);
				}
			}
			None if e > base * ratio => self.active = Some((t, e)),
			None => self.baseline = Some(base + (e - base) / (5.0 * self.rate)),
		}
		None
	}
}

/// Blinks on the frontal sensors: deflections well above the background, 30–600 ms wide. Two of
/// the same polarity within 0.12–0.8 s make a double blink; the filter's rebound after a blink has
/// the opposite polarity, so it never pairs with the blink before it.
struct Blink {
	every: usize,
	count: usize,
	scale: History,
	sigma: Option<f64>,
	active: Option<Deflection>,
	previous: Option<(f64, bool)>,
	last_double: f64,
}
#[derive(Clone, Copy)]
struct Deflection {
	start: f64,
	peak: f64,
	peak_t: f64,
	positive: bool,
}
impl Blink {
	fn new(rate: f64) -> Self {
		let every = 4;
		Blink {
			every,
			count: 0,
			scale: History::new((10.0 * rate / every as f64) as usize),
			sigma: None,
			active: None,
			previous: None,
			last_double: f64::NEG_INFINITY,
		}
	}
	fn ready(&self) -> bool {
		self.sigma.is_some()
	}
	/// Robust background level: the median absolute value is 0.674σ for Gaussian noise, and
	/// blinks are too brief to move it.
	fn refresh(&mut self) {
		if self.scale.len() >= 128 {
			self.sigma = Some(median(&self.scale.values()) / 0.6745);
		}
	}
	/// `x`: frontal 0.1–8 Hz signal in µV. `Some(true)` for a double blink, `Some(false)` for a
	/// single one.
	fn update(&mut self, x: f64, t: f64) -> Option<bool> {
		self.count += 1;
		if self.count % self.every == 0 {
			self.scale.push(x.abs());
		}
		let sigma = self.sigma?;
		let threshold = (4.5 * sigma).max(40.0);
		let a = x.abs();
		let Some(mut d) = self.active else {
			if a > threshold {
				self.active = Some(Deflection {
					start: t,
					peak: a,
					peak_t: t,
					positive: x > 0.0,
				});
			}
			return None;
		};
		let same_side = (x > 0.0) == d.positive;
		if same_side && a > d.peak {
			d.peak = a;
			d.peak_t = t;
		}
		self.active = Some(d);
		// A deflection ends once it falls well back from its own peak, so two blinks close together
		// stay two even when the signal between them never returns to the background.
		if same_side && a >= (threshold * 0.5).max(d.peak * 0.4) {
			if t - d.start > 1.0 {
				self.active = None;
			}
			return None;
		}
		self.active = None;
		if !(0.03..=0.6).contains(&(t - d.start)) {
			return None;
		}
		if let Some((prev, positive)) = self.previous
			&& positive == d.positive
			&& (0.12..=0.8).contains(&(d.peak_t - prev))
			&& d.peak_t - self.last_double > 1.0
		{
			self.previous = None;
			self.last_double = d.peak_t;
			return Some(true);
		}
		self.previous = Some((d.peak_t, d.positive));
		Some(false)
	}
}

struct Calibration {
	peaks: Vec<f64>,
	until: f64,
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::synth::{LABELS, RATE, Synth};
	use loop_bus::SimCmd;

	fn pipeline() -> Pipeline {
		let labels: Vec<String> = LABELS.iter().map(|s| s.to_string()).collect();
		Pipeline::new("Loop sim EEG", RATE, &labels, 4)
	}

	/// Runs the synthetic band for `seconds`, applying `cmds` at their times; returns every event.
	fn run(p: &mut Pipeline, synth: &mut Synth, seconds: f64, cmds: &[(f64, SimCmd)]) -> Vec<(f64, Event)> {
		let mut out = Vec::new();
		let end = synth.time() + seconds;
		let mut pending: Vec<&(f64, SimCmd)> = cmds.iter().collect();
		while synth.time() < end {
			let t = synth.time();
			pending.retain(|(at, cmd)| {
				if *at <= t {
					synth.command(cmd);
					false
				} else {
					true
				}
			});
			let s = synth.sample();
			out.extend(p.push(&s, t).into_iter().map(|e| (t, e)));
		}
		out
	}

	fn confirms(events: &[(f64, Event)]) -> Vec<(f64, ConfirmSource)> {
		events
			.iter()
			.filter_map(|(t, e)| match e {
				Event::Confirm { source } => Some((*t, *source)),
				_ => None,
			})
			.collect()
	}

	fn last_signals(events: &[(f64, Event)]) -> Signals {
		events
			.iter()
			.rev()
			.find_map(|(_, e)| match e {
				Event::Signals(s) => Some(s.clone()),
				_ => None,
			})
			.unwrap()
	}

	#[test]
	fn resting_wearer_with_spontaneous_blinks_never_confirms() {
		let (mut p, mut synth) = (pipeline(), Synth::default());
		let events = run(&mut p, &mut synth, 120.0, &[]);
		assert_eq!(confirms(&events), []);
		let s = last_signals(&events);
		assert!(s.channels.iter().all(|c| c.good), "{:?}", s.channels);
		assert_eq!(s.confirms, [ConfirmSource::Clench, ConfirmSource::DoubleBlink]);
		assert!(s.focus.is_some());
	}

	#[test]
	fn a_clench_confirms_once() {
		let (mut p, mut synth) = (pipeline(), Synth::default());
		run(&mut p, &mut synth, 10.0, &[]);
		let events = run(&mut p, &mut synth, 5.0, &[(11.0, SimCmd::Clench)]);
		let c = confirms(&events);
		assert_eq!(c.len(), 1, "{c:?}");
		assert_eq!(c[0].1, ConfirmSource::Clench);
		assert!(c[0].0 > 11.0 && c[0].0 < 12.0, "{c:?}");
	}

	#[test]
	fn a_double_blink_confirms_once_and_a_single_blink_does_not() {
		let (mut p, mut synth) = (pipeline(), Synth::default());
		synth.spontaneous_blinks = false;
		run(&mut p, &mut synth, 15.0, &[]);
		let events = run(
			&mut p,
			&mut synth,
			10.0,
			&[(16.0, SimCmd::Blink), (20.0, SimCmd::DoubleBlink)],
		);
		let c = confirms(&events);
		assert_eq!(c.len(), 1, "{c:?}");
		assert_eq!(c[0].1, ConfirmSource::DoubleBlink);
		assert!(c[0].0 > 20.0 && c[0].0 < 21.0, "{c:?}");
	}

	#[test]
	fn lost_contact_is_reported_and_its_gesture_withdrawn() {
		let (mut p, mut synth) = (pipeline(), Synth::default());
		run(&mut p, &mut synth, 10.0, &[]);
		let lose = |ch: &str| SimCmd::Contact {
			channel: ch.into(),
			good: false,
		};
		let events = run(&mut p, &mut synth, 6.0, &[(10.5, lose("TP9")), (10.5, lose("TP10"))]);
		let s = last_signals(&events);
		let tp9 = s.channels.iter().find(|c| c.name == "TP9").unwrap();
		assert!(!tp9.good);
		assert_eq!(tp9.issue.as_deref(), Some("no signal"));
		assert!(s.channels.iter().find(|c| c.name == "AF7").unwrap().good);
		assert_eq!(s.confirms, [ConfirmSource::DoubleBlink]);
		// With no muscle channel left, a clench can't confirm.
		let events = run(&mut p, &mut synth, 4.0, &[(17.0, SimCmd::Clench)]);
		assert_eq!(confirms(&events), []);
	}

	#[test]
	fn focus_follows_alpha_against_the_baseline() {
		let (mut p, mut synth) = (pipeline(), Synth::default());
		let events = run(&mut p, &mut synth, 24.0, &[]);
		assert!(last_signals(&events).focus.is_some(), "baseline learned in the steady first 25 s");
		let events = run(&mut p, &mut synth, 15.0, &[(24.0, SimCmd::Focus { value: Some(0.95) })]);
		let high = last_signals(&events).focus.unwrap();
		let events = run(&mut p, &mut synth, 15.0, &[(39.0, SimCmd::Focus { value: Some(0.05) })]);
		let low = last_signals(&events).focus.unwrap();
		assert!(high > 0.75, "high {high}");
		assert!(low < 0.25, "low {low}");
	}

	#[test]
	fn calibration_takes_two_clenches_and_they_do_not_confirm() {
		let (mut p, mut synth) = (pipeline(), Synth::default());
		run(&mut p, &mut synth, 10.0, &[]);
		let start = p.calibrate(synth.time());
		assert_eq!(start, Event::Calibration { done: 0, of: 2, finished: false });
		let events = run(&mut p, &mut synth, 5.0, &[(10.5, SimCmd::Clench), (12.0, SimCmd::Clench)]);
		assert_eq!(confirms(&events), []);
		let progress: Vec<&Event> = events
			.iter()
			.map(|(_, e)| e)
			.filter(|e| matches!(e, Event::Calibration { .. }))
			.collect();
		assert_eq!(
			progress,
			[
				&Event::Calibration { done: 1, of: 2, finished: false },
				&Event::Calibration { done: 2, of: 2, finished: true }
			]
		);
		let events = run(&mut p, &mut synth, 3.0, &[(16.0, SimCmd::Clench)]);
		assert_eq!(confirms(&events).len(), 1);
	}

	/// Over several noise seeds: no confirms in ten minutes of rest with spontaneous blinks, and
	/// every one of twenty clenches and twenty double blinks caught.
	#[test]
	fn rest_stays_quiet_and_gestures_are_caught_across_seeds() {
		const SEEDS: u64 = 3;
		for seed in 1..=SEEDS {
			let (mut p, mut synth) = (pipeline(), Synth::with_seed(seed));
			let rest = run(&mut p, &mut synth, 600.0, &[]);
			assert_eq!(confirms(&rest), [], "seed {seed}: false confirms at rest");
			let t0 = synth.time();
			let cmds: Vec<(f64, SimCmd)> = (0..40)
				.map(|i| {
					let cmd = if i % 2 == 0 { SimCmd::Clench } else { SimCmd::DoubleBlink };
					(t0 + 1.0 + 3.0 * i as f64, cmd)
				})
				.collect();
			let events = run(&mut p, &mut synth, 125.0, &cmds);
			let c = confirms(&events);
			let clenches = c.iter().filter(|(_, s)| *s == ConfirmSource::Clench).count();
			let doubles = c.iter().filter(|(_, s)| *s == ConfirmSource::DoubleBlink).count();
			assert_eq!((clenches, doubles), (20, 20), "seed {seed}: {c:?}");
		}
	}

	/// Detector numbers over many seeds: `cargo test --release -p loop-sense stress -- --ignored --nocapture`.
	#[test]
	#[ignore]
	fn stress_report() {
		let (mut false_confirms, mut clenches, mut doubles, mut minutes) = (0, 0, 0, 0.0);
		let seeds = 12;
		for seed in 1..=seeds {
			let (mut p, mut synth) = (pipeline(), Synth::with_seed(seed));
			false_confirms += confirms(&run(&mut p, &mut synth, 600.0, &[])).len();
			minutes += 10.0;
			let t0 = synth.time();
			let cmds: Vec<(f64, SimCmd)> = (0..40)
				.map(|i| (t0 + 1.0 + 3.0 * i as f64, if i % 2 == 0 { SimCmd::Clench } else { SimCmd::DoubleBlink }))
				.collect();
			let c = confirms(&run(&mut p, &mut synth, 125.0, &cmds));
			clenches += c.iter().filter(|(_, s)| *s == ConfirmSource::Clench).count();
			doubles += c.iter().filter(|(_, s)| *s == ConfirmSource::DoubleBlink).count();
		}
		eprintln!(
			"{minutes} min of rest: {false_confirms} false confirms; clenches {clenches}/{}; double blinks {doubles}/{}",
			20 * seeds,
			20 * seeds
		);
	}
}
