//! Timed actions for unattended screenshots and smoke tests, read from `LOOP_SCRIPT`:
//! `"2:press 1; 5:pin 1; 8:undo; 10:hover 2; 12:move 2.5 0; 15:recall; 17:reverse"`.
//! `move X Z` shifts the ring away from the wearer, as if they had walked off.
//!
//! These drive the synthetic headband over the bus (loop-sense must run with `--sim`), so a
//! confirm travels the whole way: band → LSL → loop-sense → bus → ring:
//! `clench`, `blink`, `blink2` (double blink), `focus 0.9` / `focus drift`, `contact TP10 lost|good`.
//! And: `key` (a keyboard confirm), `record on|off`, `calibrate`.

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
	Press(usize),
	Pin(usize),
	Return(usize),
	Undo,
	Hover(Option<usize>),
	Move(f32, f32),
	Recall,
	Reverse,
	Clench,
	Blink,
	DoubleBlink,
	Focus(Option<f32>),
	Contact(String, bool),
	Key,
	Record(bool),
	Calibrate,
}

#[derive(Debug, Default)]
pub struct Script {
	steps: Vec<(f32, Cmd)>,
}
impl Script {
	pub fn from_env() -> Self {
		let Ok(src) = std::env::var("LOOP_SCRIPT") else {
			return Script::default();
		};
		let mut steps: Vec<(f32, Cmd)> = src
			.split(';')
			.filter_map(|step| {
				let (at, cmd) = step.trim().split_once(':')?;
				let mut words = cmd.split_whitespace();
				let num = |w: Option<&str>| w.and_then(|w| w.parse::<f32>().ok());
				let cmd = match words.next()? {
					"press" => Cmd::Press(num(words.next())? as usize),
					"pin" => Cmd::Pin(num(words.next())? as usize),
					"return" => Cmd::Return(num(words.next())? as usize),
					"undo" => Cmd::Undo,
					"hover" => Cmd::Hover(num(words.next()).map(|n| n as usize)),
					"move" => Cmd::Move(num(words.next())?, num(words.next())?),
					"recall" => Cmd::Recall,
					"reverse" => Cmd::Reverse,
					"clench" => Cmd::Clench,
					"blink" => Cmd::Blink,
					"blink2" => Cmd::DoubleBlink,
					"focus" => Cmd::Focus(num(words.next())),
					"contact" => Cmd::Contact(words.next()?.to_string(), words.next()? == "good"),
					"key" => Cmd::Key,
					"record" => Cmd::Record(words.next()? == "on"),
					"calibrate" => Cmd::Calibrate,
					_ => return None,
				};
				Some((at.trim().parse().ok()?, cmd))
			})
			.collect();
		steps.sort_by(|a, b| b.0.total_cmp(&a.0));
		Script { steps }
	}

	/// Commands whose time has come, in order.
	pub fn due(&mut self, now: f32) -> Vec<Cmd> {
		let mut due = Vec::new();
		while self.steps.last().is_some_and(|(at, _)| *at <= now) {
			due.push(self.steps.pop().unwrap().1);
		}
		due
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_and_orders_steps() {
		// SAFETY: tests in this crate don't read LOOP_SCRIPT concurrently.
		unsafe { std::env::set_var("LOOP_SCRIPT", "5:record on; 2:press 1; 3:contact TP10 lost; 4:focus drift; 6:bogus") };
		let mut s = Script::from_env();
		assert_eq!(s.due(1.0), []);
		assert_eq!(s.due(3.5), [Cmd::Press(1), Cmd::Contact("TP10".into(), false)]);
		assert_eq!(s.due(10.0), [Cmd::Focus(None), Cmd::Record(true)]);
	}
}
