//! Design tokens from docs/SPEC.md §11 and the ring geometry from §5.
use glam::{Mat3, Quat, Vec3};
use stardust_xr_fusion::{
	drawable::MaterialParameter,
	types::{Color, Resource, rgba_linear},
};

pub const EYE_HEIGHT: f32 = 1.65;
pub const RING_RADIUS: f32 = 0.70;
pub const RING_HEIGHT: f32 = 1.00;
pub const TILE_HEIGHT: f32 = 1.08;
pub const ARC_RADIUS: f32 = 0.60;
pub const ARC_HEIGHT: f32 = 0.965;
pub const TILE_SIZE: [f32; 2] = [0.20, 0.12];
pub const TILE_PAD: f32 = 0.015;
pub const RAISED_SIZE: [f32; 2] = [0.52, 0.36];
pub const PINNED_SIZE: [f32; 2] = [0.95, 0.56];
pub const PILL_HEIGHT: f32 = 0.046;
/// Distance at which the status arc leaves the ring and follows the wearer (§5).
pub const ARC_FOLLOW_DISTANCE: f32 = 1.5;
/// Text lifts this far off its surface so it never z-fights.
pub const LIFT: f32 = 0.002;

fn srgb(hex: u32, alpha: f32) -> Color {
	let channel = |shift: u32| {
		let c = ((hex >> shift) & 0xff) as f32 / 255.0;
		if c <= 0.04045 {
			c / 12.92
		} else {
			((c + 0.055) / 1.055).powf(2.4)
		}
	};
	rgba_linear!(channel(16), channel(8), channel(0), alpha)
}

pub fn surface() -> Color {
	srgb(0x0B0B0C, 1.0)
}
pub fn ghost_surface() -> Color {
	srgb(0x0B0B0C, 0.6)
}
pub fn text() -> Color {
	srgb(0xF2F2F2, 1.0)
}
pub fn text2() -> Color {
	srgb(0xC9C9C9, 1.0)
}
pub fn ghost_line() -> Color {
	srgb(0xBDBDBD, 1.0)
}
pub fn focus() -> Color {
	srgb(0xFFFFFF, 1.0)
}
pub fn recording() -> Color {
	srgb(0xFF7B6B, 1.0)
}
pub fn listening() -> Color {
	srgb(0x6FE3FF, 1.0)
}
#[allow(dead_code)] // inline decisions (M3)
pub fn needs_you() -> Color {
	srgb(0xFFC857, 1.0)
}
pub fn adapted() -> Color {
	srgb(0xC9A7FF, 1.0)
}

pub fn resource(name: &str) -> Resource {
	Resource::Namespaced {
		namespace: "loop".into(),
		path: name.into(),
	}
}
pub fn font_regular() -> Resource {
	resource("atkinson-regular")
}
pub fn font_bold() -> Resource {
	resource("atkinson-bold")
}

pub fn unlit() -> MaterialParameter {
	MaterialParameter::Bool { value: true }
}
pub fn color(c: Color) -> MaterialParameter {
	MaterialParameter::Color { value: c }
}

/// Rough width of a line of Atkinson Hyperlegible, for sizing status pills without a layout round trip.
pub fn text_width(text: &str, character_height: f32, bold: bool) -> f32 {
	let per_char = if bold { 0.60 } else { 0.56 };
	text.chars().count() as f32 * character_height * per_char
}

/// Name of the prebuilt pill mesh closest to `width` (pill_05 … pill_40, in cm).
pub fn pill_for(width: f32) -> (String, f32) {
	let cm = (width * 100.0).ceil().clamp(5.0, 40.0) as u32;
	(format!("pill_{cm:02}"), cm as f32 / 100.0)
}

/// Rotation whose +Z faces `target` and whose +Y stays as close to world up as possible.
pub fn facing(from: Vec3, target: Vec3) -> Quat {
	let z = (target - from).normalize();
	let x = Vec3::Y.cross(z).normalize();
	let y = z.cross(x);
	Quat::from_mat3(&Mat3::from_cols(x, y, z))
}

/// Point on a horizontal circle around the ring centre; angle in degrees, positive to the right, 0 = forward (-Z).
pub fn on_circle(radius: f32, angle_deg: f32, height: f32) -> Vec3 {
	let a = angle_deg.to_radians();
	Vec3::new(radius * a.sin(), height, -radius * a.cos())
}

pub fn eye() -> Vec3 {
	Vec3::new(0.0, EYE_HEIGHT, 0.0)
}
