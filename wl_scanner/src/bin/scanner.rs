use std::io::{Read, Write};

pub fn main() {
	let mut buf = String::new();
	unwrap(std::io::stdin().read_to_string(&mut buf));
	let api = unwrap(wl_scanner::generate_api(&buf));
	unwrap(std::io::stdout().write_all(api.as_bytes()));
}

fn unwrap<T, E: std::error::Error>(res: Result<T, E>) -> T {
	match res {
		Ok(t) => t,
		Err(e) => {
			eprintln!("{}", e);
			std::process::exit(1);
		}
	}
}