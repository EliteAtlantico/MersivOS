//! A minimal XDF writer (https://github.com/sccn/xdf/wiki/Specifications): file header, stream
//! headers, samples, clock offsets, boundaries and stream footers.
use std::io::{self, Write};

const BOUNDARY: [u8; 16] = [
	0x43, 0xA5, 0x46, 0xDC, 0xCB, 0xF5, 0x41, 0x0F, 0xB3, 0x0E, 0xD5, 0x46, 0x73, 0x83, 0xCB, 0xE4,
];

/// XDF's variable-length integer: one byte giving the width (1, 4 or 8), then the value.
fn varlen(out: &mut Vec<u8>, n: u64) {
	if n <= u8::MAX as u64 {
		out.extend([1, n as u8]);
	} else if n <= u32::MAX as u64 {
		out.push(4);
		out.extend((n as u32).to_le_bytes());
	} else {
		out.push(8);
		out.extend(n.to_le_bytes());
	}
}

pub fn escape(s: &str) -> String {
	s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub struct Writer<W: Write> {
	w: W,
}

impl<W: Write> Writer<W> {
	pub fn new(mut w: W, datetime: &str) -> io::Result<Self> {
		w.write_all(b"XDF:")?;
		let mut writer = Writer { w };
		let header = format!(
			"<?xml version=\"1.0\"?><info><version>1.0</version><datetime>{}</datetime><recorder>loop-record</recorder></info>",
			escape(datetime)
		);
		writer.chunk(1, header.as_bytes())?;
		Ok(writer)
	}

	fn chunk(&mut self, tag: u16, content: &[u8]) -> io::Result<()> {
		let mut head = Vec::with_capacity(11);
		varlen(&mut head, content.len() as u64 + 2);
		head.extend(tag.to_le_bytes());
		self.w.write_all(&head)?;
		self.w.write_all(content)
	}

	fn with_id(id: u32, rest: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
		let mut c = id.to_le_bytes().to_vec();
		rest(&mut c);
		c
	}

	/// `xml` is the stream's LSL description; its `<channel_format>` must match what is written.
	pub fn stream_header(&mut self, id: u32, xml: &str) -> io::Result<()> {
		let c = Self::with_id(id, |c| c.extend(xml.as_bytes()));
		self.chunk(2, &c)
	}

	fn samples(&mut self, id: u32, stamps: &[f64], mut values: impl FnMut(&mut Vec<u8>, usize)) -> io::Result<()> {
		if stamps.is_empty() {
			return Ok(());
		}
		let c = Self::with_id(id, |c| {
			varlen(c, stamps.len() as u64);
			for (i, t) in stamps.iter().enumerate() {
				c.push(8);
				c.extend(t.to_le_bytes());
				values(c, i);
			}
		});
		self.chunk(3, &c)
	}

	pub fn samples_f32(&mut self, id: u32, stamps: &[f64], data: &[f32], channels: usize) -> io::Result<()> {
		self.samples(id, stamps, |c, i| {
			for v in &data[i * channels..(i + 1) * channels] {
				c.extend(v.to_le_bytes());
			}
		})
	}

	pub fn samples_f64(&mut self, id: u32, stamps: &[f64], data: &[f64], channels: usize) -> io::Result<()> {
		self.samples(id, stamps, |c, i| {
			for v in &data[i * channels..(i + 1) * channels] {
				c.extend(v.to_le_bytes());
			}
		})
	}

	pub fn samples_str(&mut self, id: u32, stamps: &[f64], data: &[Vec<String>]) -> io::Result<()> {
		self.samples(id, stamps, |c, i| {
			for v in &data[i] {
				varlen(c, v.len() as u64);
				c.extend(v.as_bytes());
			}
		})
	}

	/// The stream's clock offset, measured at `collection_time` on the recorder's clock.
	pub fn clock_offset(&mut self, id: u32, collection_time: f64, offset: f64) -> io::Result<()> {
		let c = Self::with_id(id, |c| {
			c.extend(collection_time.to_le_bytes());
			c.extend(offset.to_le_bytes());
		});
		self.chunk(4, &c)
	}

	/// Lets a reader resynchronise after a damaged stretch of the file.
	pub fn boundary(&mut self) -> io::Result<()> {
		self.chunk(5, &BOUNDARY)
	}

	pub fn stream_footer(
		&mut self,
		id: u32,
		first: f64,
		last: f64,
		count: u64,
		offsets: &[(f64, f64)],
	) -> io::Result<()> {
		let offsets: String = offsets
			.iter()
			.map(|(t, v)| format!("<offset><time>{t}</time><value>{v}</value></offset>"))
			.collect();
		let xml = format!(
			"<?xml version=\"1.0\"?><info><first_timestamp>{first}</first_timestamp><last_timestamp>{last}</last_timestamp><sample_count>{count}</sample_count><clock_offsets>{offsets}</clock_offsets></info>"
		);
		let c = Self::with_id(id, |c| c.extend(xml.as_bytes()));
		self.chunk(6, &c)
	}

	pub fn flush(&mut self) -> io::Result<()> {
		self.w.flush()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn chunks_are_length_prefixed_and_tagged() {
		let mut w = Writer::new(Vec::new(), "now").unwrap();
		w.samples_f32(7, &[1.5], &[2.0, 3.0], 2).unwrap();
		let bytes = w.w;
		assert_eq!(&bytes[..4], b"XDF:");
		// Skip the file header chunk.
		assert_eq!(bytes[4], 1);
		let header_len = bytes[5] as usize;
		let samples = &bytes[6 + header_len..];
		// length byte width, length, tag 3, stream 7, one sample, 8-byte stamp, value, value
		let mut want = vec![1, 2 + 4 + 2 + 1 + 8 + 8, 3, 0, 7, 0, 0, 0, 1, 1, 8];
		want.extend(1.5f64.to_le_bytes());
		want.extend(2.0f32.to_le_bytes());
		want.extend(3.0f32.to_le_bytes());
		assert_eq!(samples, want.as_slice());
	}

	#[test]
	fn long_lengths_widen() {
		let mut v = Vec::new();
		varlen(&mut v, 300);
		assert_eq!(v, [4, 44, 1, 0, 0]);
	}
}
