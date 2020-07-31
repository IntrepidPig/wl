use crate::scanner::*;

pub mod markdown;
//pub mod html;

pub trait DocGen {
	type Error;

	fn add_paragraph(&mut self, text: &str);
	fn begin_section(&mut self, title: &str);
	fn end_section(&mut self);
	fn generate(&mut self) -> Result<String, Self::Error>;
}

pub fn generate_docs<G: DocGen>(protocol: &ProtocolDesc, mut generator: G) -> Result<String, G::Error> {
	for interface in &protocol.interfaces {
		generate_interface(interface, &mut generator)
	}
	
	generator.generate()
}

fn generate_interface<G: DocGen>(interface: &InterfaceDesc, gen: &mut G) {
	gen.begin_section(&format!("Interface: {} (version {})", interface.name, interface.version));
	gen.add_paragraph(&interface.summary);
	gen.add_paragraph(&interface.description);
	
	for request in &interface.requests {
		gen.begin_section("Requests");
		generate_request(&request, gen);
		gen.end_section();
	}

	for event in &interface.events {
		gen.begin_section("Events");
		generate_event(&event, gen);
		gen.end_section();
	}

	for r#enum in &interface.enums {
		gen.begin_section("Enums");
		generate_enum(&r#enum, gen);
		gen.end_section();
	}
	gen.end_section();
}

fn generate_request<G: DocGen>(request: &RequestDesc, gen: &mut G) {
	gen.begin_section(
		&format!(
			"{}{}{}",
			request.name,
			if request.destructor { " (destructor)" } else { "" },
			if let Some(since) = request.since {
				format!(" (since version {})", since)
			} else {
				String::new()
			}
		)
	);
	gen.add_paragraph(&request.summary);
	gen.add_paragraph(&request.description);
	gen.begin_section("Arguments");
	for argument in &request.arguments {
		generate_argument(argument, gen);
	}
	gen.end_section();
	gen.end_section();
}

fn generate_event<G: DocGen>(event: &EventDesc, gen: &mut G) {
	gen.begin_section(
		&format!(
			"{}{}",
			event.name,
			if let Some(since) = event.since {
				format!(" (since version {})", since)
			} else {
				String::new()
			}
		)
	);
	gen.add_paragraph(&event.summary);
	gen.add_paragraph(&event.description);
	gen.begin_section("Arguments");
	for argument in &event.arguments {
		generate_argument(argument, gen);
	}
	gen.end_section();
	gen.end_section();
}

fn generate_argument<G: DocGen>(argument: &ArgumentDesc, gen: &mut G) {
	gen.begin_section(&format!("{}: {}", argument.name, argument.arg_type.to_string()));
	gen.add_paragraph(&argument.summary);
	if let Some(ref interface) = argument.interface {
		gen.add_paragraph(&format!("Interface: {}", interface))
	}
	if let Some(ref enum_type) = argument.enum_type {
		gen.add_paragraph(&format!("Enum: {}{}", if let Some(ref ns) = enum_type.0 { format!("{}.", ns) } else { String::new() }, enum_type.1));
	}
	if argument.allow_null {
		gen.add_paragraph("Nullable");
	} else {
		gen.add_paragraph("Non-nullable");
	}
	gen.end_section();
}

fn generate_enum<G: DocGen>(r#enum: &EnumDesc, gen: &mut G) {
	let e = r#enum;
	gen.begin_section(&format!(
		"Enum: {}{}{}",
		e.name,
		if e.bitfield { " (bitfield)" } else { "" },
		if let Some(since) = e.since { format!(" (since version {})", since) } else { String::new() },
	));
	gen.add_paragraph(&e.summary);
	gen.add_paragraph(&e.description);
	let mut buf = String::new();
	for entry in &e.entries {
		buf.push_str(&format!(" - {} = {}: {}\n", entry.name, entry.value, entry.summary));
	}
	gen.add_paragraph(&buf);
	gen.end_section();
}

pub(crate) fn combine_whitespace(text: &str) -> String {
	let mut buf = String::with_capacity(text.len());
	let mut whitespace = false;
	for c in text.chars() {
		if c.is_whitespace() {
			if !whitespace {
				whitespace = true;
				buf.push(' ');
			}
		} else {
			whitespace = false;
			buf.push(c);
		}
	}
	buf.shrink_to_fit();
	buf
}