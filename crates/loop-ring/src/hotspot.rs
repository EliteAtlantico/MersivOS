//! An invisible look-and-confirm target. Loop draws its own focus outline, so the molecules button
//! runs with its visuals off and only reports hover (pointing at it) and press (confirm).
use stardust_xr_asteroids::{
	Context, CreateInnerInfo, CustomElement, FnWrapper, Transformable, ValidState,
};
use stardust_xr_fusion::{
	client::FrameInfo,
	spatial::{Spatial, Transform},
};
use stardust_xr_molecules::{
	UIElement,
	button::{Button, ButtonSettings},
};

pub struct Hotspot<State: ValidState> {
	transform: Transform,
	size: [f32; 2],
	on_hover: FnWrapper<dyn Fn(&mut State, bool) + Send + Sync>,
	on_press: FnWrapper<dyn Fn(&mut State) + Send + Sync>,
}
impl<State: ValidState> std::fmt::Debug for Hotspot<State> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Hotspot")
			.field("transform", &self.transform)
			.field("size", &self.size)
			.finish()
	}
}
impl<State: ValidState> Hotspot<State> {
	pub fn new(
		size: [f32; 2],
		on_hover: impl Fn(&mut State, bool) + Send + Sync + 'static,
		on_press: impl Fn(&mut State) + Send + Sync + 'static,
	) -> Self {
		Hotspot {
			transform: Transform::IDENTITY,
			size,
			on_hover: FnWrapper(Box::new(on_hover)),
			on_press: FnWrapper(Box::new(on_press)),
		}
	}
}

pub struct HotspotInner {
	spatial: Spatial,
	button: Button,
	hovered: bool,
}

impl<State: ValidState> CustomElement<State> for Hotspot<State> {
	type Inner = HotspotInner;
	type Error = stardust_xr_fusion::Error;

	async fn create_inner(
		&self,
		context: &Context,
		info: CreateInnerInfo,
	) -> Result<Self::Inner, Self::Error> {
		let button = Button::new(
			&context.stardust_client,
			&info.parent_space,
			self.transform,
			self.size.into(),
			ButtonSettings {
				max_hover_distance: 0.02,
				visuals: None,
			},
		)
		.await?;
		info.child_space
			.set_parent(button.touch_plane().root().spatial_ref().await?)?;
		Ok(HotspotInner {
			spatial: info.child_space,
			button,
			hovered: false,
		})
	}

	fn diff(&self, old_self: &Self, _context: &Context, inner: &mut Self::Inner) {
		self.apply_transform(old_self, inner.button.touch_plane().root());
		if self.size != old_self.size {
			inner.button.set_size(self.size.into());
		}
		let _ = &inner.spatial;
	}

	fn frame(
		&self,
		_context: &Context,
		_info: &FrameInfo,
		state: &mut State,
		inner: &mut Self::Inner,
	) {
		inner.button.handle_events();
		let action = inner.button.touch_plane().action();
		let hovered = !action.hover().current().is_empty() || !action.interact().current().is_empty();
		if hovered != inner.hovered {
			inner.hovered = hovered;
			(self.on_hover.0)(state, hovered);
		}
		if inner.button.pressed() {
			(self.on_press.0)(state);
		}
	}
}
impl<State: ValidState> Transformable for Hotspot<State> {
	fn transform(&self) -> &Transform {
		&self.transform
	}
	fn transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}
}
