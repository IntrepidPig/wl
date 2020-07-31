pub mod xdg_shell {
	pub mod private {
		pub use wl_protocol::wl::*;
		
		include!(concat!(env!("OUT_DIR"), "/xdg_shell_api.rs"));
	}

	pub use private::prelude::*;
}