macro_rules! define_protocol {
	($name:ident, $path:expr) => {
		pub mod $name {
			mod private {
				#[allow(unused)]
				pub(in self) use wl_server::{
					client::{ClientMap},
					resource::{Resource, NewResource, Anonymous},
					protocol::*,
				};

				include!(concat!(env!("OUT_DIR"), $path));
			}

			pub use self::private::prelude::*;
		}
	};
}

define_protocol!(xdg_shell, "/xdg_shell_api.rs");