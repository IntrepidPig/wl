static PROTOCOL: &str = include_str!("wayland.xml");

use std::{
	env,
	fs,
	path,
};

fn main() {
	let api = wl_scanner::generate_api(PROTOCOL).expect("Failed to generate Rust API");
	let formatted_api = wl_scanner::format_rustfmt_external(&api).expect("Failed to format Rust API");
	let out_dir = env::var("OUT_DIR").expect("OUT_DIR not specified");
	let mut out_path = path::PathBuf::from(out_dir);
	out_path.push("wayland_api.rs");
	fs::write(&out_path, &formatted_api).expect("Failed to write API to file");
}