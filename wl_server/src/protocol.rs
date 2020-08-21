mod private {
	pub(in self) use crate::{
		client::{ClientMap},
		resource::{Resource, NewResource, Untyped}
	};

	include!(concat!(env!("OUT_DIR"), "/wayland_api.rs"));
}

pub use private::prelude::*;