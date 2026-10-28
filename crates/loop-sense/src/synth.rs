//! A synthetic Muse-class headband (TP9, AF7, AF8, TP10 at 256 Hz, in µV) for testing the whole
//! chain without hardware. It has what the detectors must cope with on a real band: a large DC
//! offset, 1/f background, mains hum, an alpha rhythm that falls with focus, spontaneous single
//! blinks, and, on request, jaw clenches, double blinks and lost contact.
use loop_bus::SimCmd;

pub const LABELS: [&str; 4] = ["TP9", "AF7", "AF8", "TP10"];
pub const RATE: f64 = 256.0;
const MAINS_HZ: f64 = 60.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Contact {
	Good,
	/// Lifted off the skin: the amplifier sees almost nothing.
	Lost,
}

#[derive(Debug, Clone, Copy)]
struct Burst {
	start: f64,
	length: f64,
}

pub struct Synth {
	t: f64,
	rng: u64,
	pink: [f64; 4],
	focus: Option<f64>,
	clenches: Vec<Burst>,
	blinks: Vec<f64>,
	next_blink: f64,
	pub contact: [Contact; 4],
	/// Spontaneous blinks every few seconds, as a real wearer makes them.
	pub spontaneous_blinks: bool,
}

impl Default for Synth {
	fn default() -> Self {
		Synth {
			t: 0.0,
			rng: 0x2545_F491_4F6C_DD1D,
			pink: [0.0; 4],
			focus: None,
			clenches: Vec::new(),
			blinks: Vec::new(),
			next_blink: 3.0,
			contact: [Contact::Good; 4],
			spontaneous_blinks: true,
		}
	}
}

impl Synth {
	/// A band with its own noise.
	#[cfg(test)]
	pub fn with_seed(seed: u64) -> Self {
		Synth {
			rng: 0x2545_F491_4F6C_DD1D ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15),
			..Synth::default()
		}
	}

	#[cfg(test)]
	pub fn time(&self) -> f64 {
		self.t
	}

	fn uniform(&mut self) -> f64 {
		self.rng ^= self.rng << 13;
		self.rng ^= self.rng >> 7;
		self.rng ^= self.rng << 17;
		(self.rng >> 11) as f64 / (1u64 << 53) as f64
	}
	fn gauss(&mut self) -> f64 {
		let (u1, u2) = (self.uniform().max(1e-12), self.uniform());
		(-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
	}

	/// Focus held steady for the first 25 s (while loop-sense learns the baseline), then drifting
	/// through long high and low stretches, unless a test holds it.
	pub fn focus(&self) -> f64 {
		self.focus.unwrap_or_else(|| {
			if self.t < 25.0 {
				0.5
			} else {
				0.5 + 0.45 * (std::f64::consts::TAU * (self.t - 25.0) / 70.0).sin()
			}
		})
	}

	pub fn command(&mut self, cmd: &SimCmd) {
		let now = self.t;
		match cmd {
			SimCmd::Clench => self.clenches.push(Burst {
				start: now + 0.05,
				length: 0.45,
			}),
			SimCmd::Blink => self.blinks.push(now + 0.1),
			SimCmd::DoubleBlink => {
				self.blinks.push(now + 0.1);
				self.blinks.push(now + 0.42);
			}
			SimCmd::Focus { value } => self.focus = value.map(|v| v.clamp(0.0, 1.0) as f64),
			SimCmd::Contact { channel, good } => {
				if let Some(c) = LABELS.iter().position(|l| l.eq_ignore_ascii_case(channel)) {
					self.contact[c] = if *good { Contact::Good } else { Contact::Lost };
				}
			}
		}
		// Keep spontaneous blinks clear of a requested gesture.
		if matches!(cmd, SimCmd::Blink | SimCmd::DoubleBlink | SimCmd::Clench) {
			self.next_blink = self.next_blink.max(now + 2.0);
		}
	}

	/// The next sample, in µV.
	pub fn sample(&mut self) -> [f32; 4] {
		let dt = 1.0 / RATE;
		let t = self.t;
		self.t += dt;
		if self.spontaneous_blinks && t >= self.next_blink {
			self.blinks.push(t);
			self.next_blink = t + 3.0 + 3.0 * self.uniform();
		}
		self.blinks.retain(|b| t - b < 1.0);
		self.clenches.retain(|c| t < c.start + c.length + 0.1);

		let focus = self.focus();
		let alpha_amp = 2.0 + 25.0 * (1.0 - focus).powi(2);
		let beta_amp = 3.0 + 5.0 * focus;
		let emg_env: f64 = self
			.clenches
			.iter()
			.map(|c| {
				let u = (t - c.start) / c.length;
				if (0.0..=1.0).contains(&u) {
					(std::f64::consts::PI * u).sin().powf(0.5)
				} else {
					0.0
				}
			})
			.sum();
		let blink: f64 = self
			.blinks
			.iter()
			.map(|b| -160.0 * (-0.5 * ((t - b - 0.15) / 0.07).powi(2)).exp())
			.sum();

		let mut out = [0.0f32; 4];
		for c in 0..4 {
			let temporal = c == 0 || c == 3;
			let white = self.gauss();
			self.pink[c] = 0.97 * self.pink[c] + white;
			let background = 3.0 * self.pink[c] + 2.0 * self.gauss();
			let phase = c as f64 * 0.7;
			let alpha = alpha_amp
				* if temporal { 1.0 } else { 0.6 }
				* (std::f64::consts::TAU * 10.0 * t + phase).sin();
			let beta = 0.5 * beta_amp * (std::f64::consts::TAU * 20.0 * t + 2.0 * phase).sin();
			let mains = 1.5 * (std::f64::consts::TAU * MAINS_HZ * t).sin();
			let emg = emg_env * if temporal { 45.0 } else { 12.0 } * self.gauss();
			let eye = blink * if temporal { 0.12 } else { 1.0 };
			let dc = 800.0 + 20.0 * c as f64;
			let v = match self.contact[c] {
				Contact::Good => dc + background + alpha + beta + mains + emg + eye,
				Contact::Lost => dc + 0.05 * self.gauss(),
			};
			out[c] = v as f32;
		}
		out
	}
}
