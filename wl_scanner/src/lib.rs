use std::{
	io::{Write},
	process::{Command, Stdio},
};

pub mod scanner;
pub mod generator;
//pub mod doc;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GenerationError {
	#[error(transparent)]
	ParseError(#[from] scanner::ProtocolParseError),
	#[error(transparent)]
	GenerationError(#[from] generator::ProtocolGenError),
}

pub fn generate_api(protocol: &str) -> Result<String, GenerationError> {
	let mut reader = quick_xml::Reader::from_str(protocol);
	reader.trim_text(true);
	let mut buf = Vec::new();
	let desc = scanner::parse_protocol(&mut reader, &mut buf)?;
	let api = generator::generate_api(&desc)?;
	Ok(api)
}

pub fn format_rustfmt_external(source: &str) -> Result<String, ()> {
    let mut proc = Command::new("rustfmt")
        .arg("--emit=stdout")
        .arg("--edition=2018")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
		.spawn()
		.map_err(|_| ())?;
	
	let stdin = proc.stdin.as_mut().unwrap();
	stdin.write_all(source.as_bytes()).unwrap();
	let output = proc.wait_with_output().map_err(|_| ())?;
	if output.status.success() {
		String::from_utf8(output.stdout).map_err(|_| ())
	} else {
		Err(())
	}
}