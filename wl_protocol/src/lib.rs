pub mod wl {
	pub mod private {
		include!(concat!(env!("OUT_DIR"), "/wayland_api.rs"));
	}

	pub use private::prelude::*;
}