//! The parts of liblsl (Lab Streaming Layer) that Loop uses. The library is loaded at run time, so
//! Loop still runs where it isn't installed and says so instead of failing to start. It is looked
//! for at `$LOOP_LIBLSL`, then on the system library path, then in `upstream/liblsl/build` of the
//! workspace the binary was built in.
use libloading::Library;
use std::ffi::{CStr, CString, c_char, c_ulong, c_void};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

type Ptr = *mut c_void;

macro_rules! api {
	($($field:ident : fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
		struct Api {
			$($field: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
			_lib: Library,
		}
		impl Api {
			/// Safety: `lib` must be liblsl, whose symbols have the signatures above.
			unsafe fn load(lib: Library) -> Result<Api, libloading::Error> {
				Ok(Api {
					$($field: *unsafe {
						lib.get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(
							concat!("lsl_", stringify!($field), "\0").as_bytes(),
						)
					}?,)*
					_lib: lib,
				})
			}
		}
	};
}

api! {
	library_version: fn() -> i32;
	local_clock: fn() -> f64;
	destroy_string: fn(*mut c_char);
	resolve_all: fn(*mut Ptr, u32, f64) -> i32;
	create_streaminfo: fn(*const c_char, *const c_char, i32, f64, i32, *const c_char) -> Ptr;
	destroy_streaminfo: fn(Ptr);
	get_name: fn(Ptr) -> *const c_char;
	get_type: fn(Ptr) -> *const c_char;
	get_source_id: fn(Ptr) -> *const c_char;
	get_uid: fn(Ptr) -> *const c_char;
	get_hostname: fn(Ptr) -> *const c_char;
	get_channel_count: fn(Ptr) -> i32;
	get_nominal_srate: fn(Ptr) -> f64;
	get_channel_format: fn(Ptr) -> i32;
	get_desc: fn(Ptr) -> Ptr;
	get_xml: fn(Ptr) -> *mut c_char;
	append_child: fn(Ptr, *const c_char) -> Ptr;
	append_child_value: fn(Ptr, *const c_char, *const c_char) -> Ptr;
	create_outlet: fn(Ptr, i32, i32) -> Ptr;
	destroy_outlet: fn(Ptr);
	push_chunk_ft: fn(Ptr, *const f32, c_ulong, f64) -> i32;
	push_sample_strt: fn(Ptr, *const *const c_char, f64) -> i32;
	create_inlet: fn(Ptr, i32, i32, i32) -> Ptr;
	destroy_inlet: fn(Ptr);
	get_fullinfo: fn(Ptr, f64, *mut i32) -> Ptr;
	open_stream: fn(Ptr, f64, *mut i32);
	time_correction: fn(Ptr, f64, *mut i32) -> f64;
	pull_chunk_f: fn(Ptr, *mut f32, *mut f64, c_ulong, c_ulong, f64, *mut i32) -> c_ulong;
	pull_chunk_d: fn(Ptr, *mut f64, *mut f64, c_ulong, c_ulong, f64, *mut i32) -> c_ulong;
	pull_sample_str: fn(Ptr, *mut *mut c_char, i32, f64, *mut i32) -> f64;
}

static API: OnceLock<Option<Api>> = OnceLock::new();

fn candidates() -> Vec<PathBuf> {
	let mut paths: Vec<PathBuf> = Vec::new();
	if let Some(p) = std::env::var_os("LOOP_LIBLSL") {
		paths.push(p.into());
	}
	paths.push("liblsl.so".into());
	paths.push("liblsl.so.2".into());
	// target/{debug,release}/<binary> → upstream/liblsl/build
	if let Ok(exe) = std::env::current_exe() {
		for up in exe.ancestors().skip(1).take(4) {
			paths.push(up.join("upstream/liblsl/build/liblsl.so"));
		}
	}
	paths
}

fn api() -> Option<&'static Api> {
	API.get_or_init(|| {
		for path in candidates() {
			// Safety: liblsl's initialisers have no preconditions.
			let Ok(lib) = (unsafe { Library::new(&path) }) else { continue };
			// Safety: a library that exports `lsl_local_clock` is taken to be liblsl.
			if let Ok(api) = unsafe { Api::load(lib) } {
				return Some(api);
			}
		}
		None
	})
	.as_ref()
}

/// Whether liblsl was found.
pub fn available() -> bool {
	api().is_some()
}

/// liblsl's version as `major * 100 + minor`, e.g. 117.
pub fn version() -> Option<i32> {
	api().map(|a| unsafe { (a.library_version)() })
}

/// Seconds on LSL's clock (the monotonic clock). Without liblsl, seconds since this process
/// first asked, which is good enough for a session that has no LSL streams to line up with.
pub fn local_clock() -> f64 {
	static START: OnceLock<Instant> = OnceLock::new();
	match api() {
		Some(a) => unsafe { (a.local_clock)() },
		None => START.get_or_init(Instant::now).elapsed().as_secs_f64(),
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
	Missing,
	Timeout,
	Lost,
	Argument,
	Internal,
	Other(i32),
}
impl std::fmt::Display for Error {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Error::Missing => write!(f, "liblsl not found"),
			Error::Timeout => write!(f, "timed out"),
			Error::Lost => write!(f, "stream lost"),
			Error::Argument => write!(f, "bad argument"),
			Error::Internal => write!(f, "internal liblsl error"),
			Error::Other(c) => write!(f, "liblsl error {c}"),
		}
	}
}
impl std::error::Error for Error {}

fn check(ec: i32) -> Result<(), Error> {
	match ec {
		0 => Ok(()),
		-1 => Err(Error::Timeout),
		-2 => Err(Error::Lost),
		-3 => Err(Error::Argument),
		-4 => Err(Error::Internal),
		c => Err(Error::Other(c)),
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
	Float32,
	Double64,
	String,
	Int32,
	Int16,
	Int8,
	Int64,
	Undefined,
}
impl Format {
	fn from_raw(raw: i32) -> Format {
		match raw {
			1 => Format::Float32,
			2 => Format::Double64,
			3 => Format::String,
			4 => Format::Int32,
			5 => Format::Int16,
			6 => Format::Int8,
			7 => Format::Int64,
			_ => Format::Undefined,
		}
	}
	fn raw(self) -> i32 {
		match self {
			Format::Undefined => 0,
			Format::Float32 => 1,
			Format::Double64 => 2,
			Format::String => 3,
			Format::Int32 => 4,
			Format::Int16 => 5,
			Format::Int8 => 6,
			Format::Int64 => 7,
		}
	}
	/// The name XDF and LSL's XML use, e.g. `float32`.
	pub fn xml_name(self) -> &'static str {
		match self {
			Format::Float32 => "float32",
			Format::Double64 => "double64",
			Format::String => "string",
			Format::Int32 => "int32",
			Format::Int16 => "int16",
			Format::Int8 => "int8",
			Format::Int64 => "int64",
			Format::Undefined => "undefined",
		}
	}
}

fn owned_str(a: &Api, s: *mut c_char) -> String {
	if s.is_null() {
		return String::new();
	}
	let out = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
	unsafe { (a.destroy_string)(s) };
	out
}

fn borrowed_str(s: *const c_char) -> String {
	if s.is_null() {
		return String::new();
	}
	unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
}

fn cstr(s: &str) -> CString {
	CString::new(s.replace('\0', "")).unwrap()
}

/// Description of a stream: what it is, how many channels at what rate, and its metadata.
pub struct StreamInfo {
	ptr: Ptr,
}
// liblsl stream infos are plain heap objects; they may move between threads.
unsafe impl Send for StreamInfo {}

impl StreamInfo {
	/// A new stream description with channel labels under `<desc><channels>`.
	pub fn new(
		name: &str,
		kind: &str,
		labels: &[&str],
		rate: f64,
		format: Format,
		source_id: &str,
	) -> Result<StreamInfo, Error> {
		let a = api().ok_or(Error::Missing)?;
		let (name, kind, source_id) = (cstr(name), cstr(kind), cstr(source_id));
		let ptr = unsafe {
			(a.create_streaminfo)(
				name.as_ptr(),
				kind.as_ptr(),
				labels.len() as i32,
				rate,
				format.raw(),
				source_id.as_ptr(),
			)
		};
		if ptr.is_null() {
			return Err(Error::Internal);
		}
		let info = StreamInfo { ptr };
		let desc = unsafe { (a.get_desc)(ptr) };
		let manufacturer = (cstr("manufacturer"), cstr("Loop"));
		unsafe { (a.append_child_value)(desc, manufacturer.0.as_ptr(), manufacturer.1.as_ptr()) };
		let channels_tag = cstr("channels");
		let channels = unsafe { (a.append_child)(desc, channels_tag.as_ptr()) };
		let (channel_tag, label_tag) = (cstr("channel"), cstr("label"));
		for label in labels {
			let label = cstr(label);
			let channel = unsafe { (a.append_child)(channels, channel_tag.as_ptr()) };
			unsafe { (a.append_child_value)(channel, label_tag.as_ptr(), label.as_ptr()) };
		}
		Ok(info)
	}

	fn api(&self) -> &'static Api {
		// A StreamInfo only exists once the library has loaded.
		api().unwrap()
	}
	pub fn name(&self) -> String {
		borrowed_str(unsafe { (self.api().get_name)(self.ptr) })
	}
	/// The content type, e.g. `EEG` or `Markers`.
	pub fn kind(&self) -> String {
		borrowed_str(unsafe { (self.api().get_type)(self.ptr) })
	}
	pub fn source_id(&self) -> String {
		borrowed_str(unsafe { (self.api().get_source_id)(self.ptr) })
	}
	pub fn uid(&self) -> String {
		borrowed_str(unsafe { (self.api().get_uid)(self.ptr) })
	}
	/// The machine the stream comes from.
	pub fn hostname(&self) -> String {
		borrowed_str(unsafe { (self.api().get_hostname)(self.ptr) })
	}
	pub fn channel_count(&self) -> usize {
		unsafe { (self.api().get_channel_count)(self.ptr) }.max(0) as usize
	}
	/// Nominal sampling rate in Hz; 0 for irregular streams such as markers.
	pub fn rate(&self) -> f64 {
		unsafe { (self.api().get_nominal_srate)(self.ptr) }
	}
	pub fn format(&self) -> Format {
		Format::from_raw(unsafe { (self.api().get_channel_format)(self.ptr) })
	}
	/// The whole description as XML; this is what an XDF stream header holds.
	pub fn xml(&self) -> String {
		owned_str(self.api(), unsafe { (self.api().get_xml)(self.ptr) })
	}
	/// Channel labels from `<desc><channels>`, if the stream declares them.
	pub fn channel_labels(&self) -> Vec<String> {
		channel_labels(&self.xml())
	}
}
impl Drop for StreamInfo {
	fn drop(&mut self) {
		unsafe { (self.api().destroy_streaminfo)(self.ptr) };
	}
}
impl std::fmt::Debug for StreamInfo {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("StreamInfo")
			.field("name", &self.name())
			.field("kind", &self.kind())
			.field("channels", &self.channel_count())
			.field("rate", &self.rate())
			.finish()
	}
}

/// The `<label>`s inside `<channels>` of a stream's XML description.
pub fn channel_labels(xml: &str) -> Vec<String> {
	let Some(start) = xml.find("<channels>") else {
		return Vec::new();
	};
	let end = xml[start..].find("</channels>").map_or(xml.len(), |e| start + e);
	let mut rest = &xml[start..end];
	let mut labels = Vec::new();
	while let Some(open) = rest.find("<channel>") {
		rest = &rest[open + "<channel>".len()..];
		let close = rest.find("</channel>").unwrap_or(rest.len());
		let channel = &rest[..close];
		let label = channel
			.find("<label>")
			.and_then(|s| {
				let s = s + "<label>".len();
				channel[s..].find("</label>").map(|e| &channel[s..s + e])
			})
			.unwrap_or("");
		labels.push(label.trim().to_string());
		rest = &rest[close..];
	}
	labels
}

/// Every stream visible on the network, after waiting `wait` seconds for answers.
pub fn resolve_all(wait: f64) -> Vec<StreamInfo> {
	let Some(a) = api() else {
		return Vec::new();
	};
	let mut buf: Vec<Ptr> = vec![std::ptr::null_mut(); 64];
	let n = unsafe { (a.resolve_all)(buf.as_mut_ptr(), buf.len() as u32, wait) };
	buf.truncate(n.max(0) as usize);
	buf.into_iter()
		.filter(|p| !p.is_null())
		.map(|ptr| StreamInfo { ptr })
		.collect()
}

/// Receives one stream.
pub struct Inlet {
	ptr: Ptr,
	channels: usize,
}
// An inlet is used from one thread at a time; liblsl allows moving it between threads.
unsafe impl Send for Inlet {}

impl Inlet {
	/// Buffers up to six minutes and recovers on its own if the source restarts.
	pub fn new(info: &StreamInfo) -> Result<Inlet, Error> {
		let a = api().ok_or(Error::Missing)?;
		let ptr = unsafe { (a.create_inlet)(info.ptr, 360, 0, 1) };
		if ptr.is_null() {
			return Err(Error::Internal);
		}
		Ok(Inlet {
			ptr,
			channels: info.channel_count().max(1),
		})
	}
	fn api(&self) -> &'static Api {
		api().unwrap()
	}
	/// The full description, including `<desc>` metadata.
	pub fn info(&self, timeout: f64) -> Result<StreamInfo, Error> {
		let mut ec = 0;
		let ptr = unsafe { (self.api().get_fullinfo)(self.ptr, timeout, &mut ec) };
		check(ec)?;
		if ptr.is_null() {
			return Err(Error::Internal);
		}
		Ok(StreamInfo { ptr })
	}
	pub fn open(&self, timeout: f64) -> Result<(), Error> {
		let mut ec = 0;
		unsafe { (self.api().open_stream)(self.ptr, timeout, &mut ec) };
		check(ec)
	}
	/// Offset to add to this stream's time stamps to bring them onto the local clock.
	pub fn time_correction(&self, timeout: f64) -> Result<f64, Error> {
		let mut ec = 0;
		let v = unsafe { (self.api().time_correction)(self.ptr, timeout, &mut ec) };
		check(ec).map(|_| v)
	}
	/// Whatever samples are ready now (up to `max` of them), as channel-interleaved values and one
	/// time stamp per sample. Returns the number of samples.
	pub fn pull_f32(&self, data: &mut Vec<f32>, stamps: &mut Vec<f64>, max: usize) -> Result<usize, Error> {
		data.resize(max * self.channels, 0.0);
		stamps.resize(max, 0.0);
		let mut ec = 0;
		let n = unsafe {
			(self.api().pull_chunk_f)(
				self.ptr,
				data.as_mut_ptr(),
				stamps.as_mut_ptr(),
				data.len() as c_ulong,
				stamps.len() as c_ulong,
				0.0,
				&mut ec,
			)
		} as usize;
		check(ec)?;
		let samples = n / self.channels;
		data.truncate(samples * self.channels);
		stamps.truncate(samples);
		Ok(samples)
	}
	pub fn pull_f64(&self, data: &mut Vec<f64>, stamps: &mut Vec<f64>, max: usize) -> Result<usize, Error> {
		data.resize(max * self.channels, 0.0);
		stamps.resize(max, 0.0);
		let mut ec = 0;
		let n = unsafe {
			(self.api().pull_chunk_d)(
				self.ptr,
				data.as_mut_ptr(),
				stamps.as_mut_ptr(),
				data.len() as c_ulong,
				stamps.len() as c_ulong,
				0.0,
				&mut ec,
			)
		} as usize;
		check(ec)?;
		let samples = n / self.channels;
		data.truncate(samples * self.channels);
		stamps.truncate(samples);
		Ok(samples)
	}
	/// Whatever string samples are ready now, each with its time stamp.
	pub fn pull_strings(&self) -> Result<Vec<(f64, Vec<String>)>, Error> {
		let a = self.api();
		let mut out = Vec::new();
		loop {
			let mut buf: Vec<*mut c_char> = vec![std::ptr::null_mut(); self.channels];
			let mut ec = 0;
			let t = unsafe { (a.pull_sample_str)(self.ptr, buf.as_mut_ptr(), buf.len() as i32, 0.0, &mut ec) };
			check(ec)?;
			if t == 0.0 {
				return Ok(out);
			}
			out.push((t, buf.into_iter().map(|s| owned_str(a, s)).collect()));
		}
	}
}
impl Drop for Inlet {
	fn drop(&mut self) {
		unsafe { (self.api().destroy_inlet)(self.ptr) };
	}
}

/// Publishes one stream.
pub struct Outlet {
	ptr: Ptr,
}
unsafe impl Send for Outlet {}

impl Outlet {
	pub fn new(info: &StreamInfo) -> Result<Outlet, Error> {
		let a = api().ok_or(Error::Missing)?;
		let ptr = unsafe { (a.create_outlet)(info.ptr, 0, 360) };
		if ptr.is_null() {
			return Err(Error::Internal);
		}
		Ok(Outlet { ptr })
	}
	fn api(&self) -> &'static Api {
		api().unwrap()
	}
	/// Channel-interleaved samples; `timestamp` is the capture time of the last one, and earlier
	/// ones are stamped back from it at the nominal rate.
	pub fn push_f32(&self, data: &[f32], timestamp: f64) {
		unsafe { (self.api().push_chunk_ft)(self.ptr, data.as_ptr(), data.len() as c_ulong, timestamp) };
	}
	/// One sample of a single-channel string stream (a marker).
	pub fn push_str(&self, value: &str, timestamp: f64) {
		let value = cstr(value);
		let ptrs = [value.as_ptr()];
		unsafe { (self.api().push_sample_strt)(self.ptr, ptrs.as_ptr(), timestamp) };
	}
}
impl Drop for Outlet {
	fn drop(&mut self) {
		unsafe { (self.api().destroy_outlet)(self.ptr) };
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_channel_labels_from_xml() {
		let xml = "<info><desc><channels><channel><label>TP9</label></channel>\
			<channel><label>AF7</label><unit>uV</unit></channel><channel></channel></channels></desc></info>";
		assert_eq!(channel_labels(xml), ["TP9", "AF7", ""]);
		assert!(channel_labels("<info/>").is_empty());
	}

	/// Needs liblsl; skipped (passes) without it.
	#[test]
	fn outlet_to_inlet_round_trip() {
		if !available() {
			eprintln!("liblsl not found; skipping");
			return;
		}
		let id = format!("loop-lsl-test-{}", std::process::id());
		let info = StreamInfo::new("loop-lsl test", "Test", &["A", "B"], 100.0, Format::Float32, &id).unwrap();
		let outlet = Outlet::new(&info).unwrap();
		let found = (0..20)
			.find_map(|_| resolve_all(0.5).into_iter().find(|s| s.source_id() == id))
			.expect("the test outlet resolves");
		let inlet = Inlet::new(&found).unwrap();
		inlet.open(5.0).unwrap();
		assert_eq!(inlet.info(5.0).unwrap().channel_labels(), ["A", "B"]);
		std::thread::sleep(std::time::Duration::from_millis(200));
		let now = local_clock();
		outlet.push_f32(&[1.0, 2.0, 3.0, 4.0], now);
		let (mut data, mut stamps) = (Vec::new(), Vec::new());
		let mut got = 0;
		for _ in 0..50 {
			got += inlet.pull_f32(&mut data, &mut stamps, 64).unwrap();
			if got > 0 {
				break;
			}
			std::thread::sleep(std::time::Duration::from_millis(50));
		}
		assert_eq!(got, 2);
		assert_eq!(data, [1.0, 2.0, 3.0, 4.0]);
		assert!((stamps[1] - now).abs() < 1e-6);
		assert!((stamps[1] - stamps[0] - 0.01).abs() < 1e-6);
	}
}
