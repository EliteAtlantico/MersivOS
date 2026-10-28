//! Streaming filters and spectra. Filters run sample by sample with their state kept, so there
//! are no window edges to pad; phase doesn't matter for anything Loop measures.
use realfft::{RealFftPlanner, RealToComplex};
use std::collections::VecDeque;
use std::f64::consts::TAU;
use std::sync::Arc;

/// One second-order section (RBJ cookbook), transposed direct form II.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
	b0: f64,
	b1: f64,
	b2: f64,
	a1: f64,
	a2: f64,
	z1: f64,
	z2: f64,
}
impl Biquad {
	fn new(b: [f64; 3], a: [f64; 3]) -> Self {
		Biquad {
			b0: b[0] / a[0],
			b1: b[1] / a[0],
			b2: b[2] / a[0],
			a1: a[1] / a[0],
			a2: a[2] / a[0],
			z1: 0.0,
			z2: 0.0,
		}
	}
	fn terms(fs: f64, f0: f64, q: f64) -> (f64, f64) {
		let w0 = TAU * f0 / fs;
		(w0.cos(), w0.sin() / (2.0 * q))
	}
	pub fn lowpass(fs: f64, f0: f64, q: f64) -> Self {
		let (cos, alpha) = Self::terms(fs, f0, q);
		Self::new(
			[(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0],
			[1.0 + alpha, -2.0 * cos, 1.0 - alpha],
		)
	}
	pub fn highpass(fs: f64, f0: f64, q: f64) -> Self {
		let (cos, alpha) = Self::terms(fs, f0, q);
		Self::new(
			[(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
			[1.0 + alpha, -2.0 * cos, 1.0 - alpha],
		)
	}
	pub fn notch(fs: f64, f0: f64, q: f64) -> Self {
		let (cos, alpha) = Self::terms(fs, f0, q);
		Self::new([1.0, -2.0 * cos, 1.0], [1.0 + alpha, -2.0 * cos, 1.0 - alpha])
	}
	pub fn process(&mut self, x: f64) -> f64 {
		let y = self.b0 * x + self.z1;
		self.z1 = self.b1 * x - self.a1 * y + self.z2;
		self.z2 = self.b2 * x - self.a2 * y;
		y
	}
}

/// Section Q values of 2nd- and 4th-order Butterworth filters.
const BUTTER2: [f64; 1] = [std::f64::consts::FRAC_1_SQRT_2];
const BUTTER4: [f64; 2] = [0.541_196_100_146_197, 1.306_562_964_876_376_7];

fn butter_q(order: usize) -> &'static [f64] {
	match order {
		2 => &BUTTER2,
		4 => &BUTTER4,
		_ => panic!("only 2nd- and 4th-order Butterworth sections are provided"),
	}
}

/// A cascade of sections.
#[derive(Debug, Clone, Default)]
pub struct Chain(Vec<Biquad>);
impl Chain {
	pub fn highpass(self, fs: f64, f0: f64, order: usize) -> Self {
		self.with(butter_q(order).iter().map(|q| Biquad::highpass(fs, f0, *q)))
	}
	pub fn lowpass(self, fs: f64, f0: f64, order: usize) -> Self {
		self.with(butter_q(order).iter().map(|q| Biquad::lowpass(fs, f0, *q)))
	}
	/// A notch at `f0`, skipped when it lies above Nyquist.
	pub fn notch(self, fs: f64, f0: f64) -> Self {
		if f0 >= fs / 2.0 {
			return self;
		}
		self.with([Biquad::notch(fs, f0, 20.0)])
	}
	fn with(mut self, sections: impl IntoIterator<Item = Biquad>) -> Self {
		self.0.extend(sections);
		self
	}
	pub fn process(&mut self, x: f64) -> f64 {
		self.0.iter_mut().fold(x, |x, s| s.process(x))
	}
}

/// Fixed-length history, oldest first.
#[derive(Debug, Clone)]
pub struct History {
	buf: VecDeque<f64>,
	len: usize,
}
impl History {
	pub fn new(len: usize) -> Self {
		History {
			buf: VecDeque::with_capacity(len),
			len,
		}
	}
	pub fn push(&mut self, x: f64) {
		if self.buf.len() == self.len {
			self.buf.pop_front();
		}
		self.buf.push_back(x);
	}
	pub fn full(&self) -> bool {
		self.buf.len() == self.len
	}
	pub fn len(&self) -> usize {
		self.buf.len()
	}
	pub fn values(&self) -> Vec<f64> {
		self.buf.iter().copied().collect()
	}
}

pub fn mean(x: &[f64]) -> f64 {
	x.iter().sum::<f64>() / x.len().max(1) as f64
}
pub fn std(x: &[f64]) -> f64 {
	let m = mean(x);
	(x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}
pub fn ptp(x: &[f64]) -> f64 {
	let (lo, hi) = x
		.iter()
		.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
	hi - lo
}
pub fn median(x: &[f64]) -> f64 {
	if x.is_empty() {
		return f64::NAN;
	}
	let mut v = x.to_vec();
	v.sort_by(f64::total_cmp);
	let n = v.len();
	if n % 2 == 1 {
		v[n / 2]
	} else {
		(v[n / 2 - 1] + v[n / 2]) / 2.0
	}
}

/// Welch's power spectral density: Hann-windowed segments with half overlap, mean removed from
/// each, one-sided, in units²/Hz (matches `scipy.signal.welch` defaults).
pub struct Welch {
	fs: f64,
	n: usize,
	window: Vec<f64>,
	fft: Arc<dyn RealToComplex<f64>>,
}
impl Welch {
	pub fn new(fs: f64, segment: usize) -> Self {
		// Periodic Hann, as scipy's get_window uses for spectral analysis.
		let window = (0..segment)
			.map(|i| 0.5 - 0.5 * (TAU * i as f64 / segment as f64).cos())
			.collect();
		Welch {
			fs,
			n: segment,
			window,
			fft: RealFftPlanner::<f64>::new().plan_fft_forward(segment),
		}
	}
	/// Frequency of bin `k`.
	pub fn freq(&self, k: usize) -> f64 {
		k as f64 * self.fs / self.n as f64
	}
	pub fn psd(&self, x: &[f64]) -> Vec<f64> {
		let bins = self.n / 2 + 1;
		let mut acc = vec![0.0; bins];
		let step = self.n / 2;
		let mut segments = 0;
		let mut input = self.fft.make_input_vec();
		let mut output = self.fft.make_output_vec();
		let mut start = 0;
		while start + self.n <= x.len() {
			let seg = &x[start..start + self.n];
			let m = mean(seg);
			for (i, v) in seg.iter().enumerate() {
				input[i] = (v - m) * self.window[i];
			}
			self.fft.process(&mut input, &mut output).unwrap();
			for (a, c) in acc.iter_mut().zip(&output) {
				*a += c.norm_sqr();
			}
			segments += 1;
			start += step;
		}
		let scale = 1.0 / (self.fs * self.window.iter().map(|w| w * w).sum::<f64>() * segments.max(1) as f64);
		for (k, a) in acc.iter_mut().enumerate() {
			*a *= scale;
			if k != 0 && !(self.n % 2 == 0 && k == bins - 1) {
				*a *= 2.0;
			}
		}
		acc
	}
	/// Power in [lo, hi) Hz.
	pub fn band(&self, psd: &[f64], lo: f64, hi: f64) -> f64 {
		let df = self.fs / self.n as f64;
		psd.iter()
			.enumerate()
			.filter(|(k, _)| {
				let f = self.freq(*k);
				f >= lo && f < hi
			})
			.map(|(_, p)| p * df)
			.sum()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::f64::consts::PI;

	fn sine(fs: f64, f: f64, amp: f64, n: usize) -> Vec<f64> {
		(0..n).map(|i| amp * (2.0 * PI * f * i as f64 / fs).sin()).collect()
	}

	fn gain(mut chain: Chain, fs: f64, f: f64) -> f64 {
		let x = sine(fs, f, 1.0, (fs * 20.0) as usize);
		let y: Vec<f64> = x.iter().map(|v| chain.process(*v)).collect();
		let tail = &y[y.len() / 2..];
		ptp(tail) / 2.0
	}

	#[test]
	fn band_pass_keeps_alpha_and_drops_drift_and_emg() {
		let fs = 256.0;
		let band = || Chain::default().highpass(fs, 1.0, 4).lowpass(fs, 30.0, 4);
		assert!((gain(band(), fs, 10.0) - 1.0).abs() < 0.02);
		assert!(gain(band(), fs, 0.2) < 0.01);
		assert!(gain(band(), fs, 80.0) < 0.01);
	}

	#[test]
	fn notch_removes_mains() {
		let fs = 256.0;
		let n = || Chain::default().notch(fs, 60.0);
		assert!(gain(n(), fs, 60.0) < 0.01);
		assert!((gain(n(), fs, 80.0) - 1.0).abs() < 0.05);
	}

	#[test]
	fn welch_finds_a_sine_and_its_power() {
		let fs = 256.0;
		let w = Welch::new(fs, 256);
		let psd = w.psd(&sine(fs, 10.0, 4.0, 512));
		let peak = (0..psd.len()).max_by(|a, b| psd[*a].total_cmp(&psd[*b])).unwrap();
		assert_eq!(w.freq(peak), 10.0);
		// A sine of amplitude A carries A²/2 of power.
		let p = w.band(&psd, 8.0, 12.0);
		assert!((p - 8.0).abs() < 0.2, "{p}");
	}
}
