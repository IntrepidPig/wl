use crate::{
	wl::{
		doc::{self, DocGen},
	},
};

pub struct MarkdownGenerator {
	buf: String,
	sections: Vec<i32>,
}

impl MarkdownGenerator {
	pub fn new() -> Self {
		MarkdownGenerator {
			buf: String::new(),
			sections: vec![0],
		}
	}
}

impl DocGen for MarkdownGenerator {
	type Error = ();

	fn add_paragraph(&mut self, text: &str) {
		self.buf.push_str(&doc::combine_whitespace(&text));
		self.buf.push_str("\n\n");
	}

	fn begin_section(&mut self, title: &str) {
		let header_prefix = match self.sections.len() {
			0 => "#",
			1 => "##",
			2 => "###",
			3 => "####",
			4 => "#####",
			_ => "######",
		};
		self.buf.push_str(header_prefix);
		self.buf.push_str(" ");
		for (i, section) in self.sections.iter().enumerate() {
			self.buf.push_str(&format!("{}", section + 1));
			if i < self.sections.len() - 1 {
				self.buf.push_str(".");
			}
		}
		self.buf.push_str(" ");
		self.buf.push_str(&title);
		self.buf.push_str("\n\n");
		self.sections.push(0);
	}

	fn end_section(&mut self) {
		self.sections.pop();
		if let Some(section) = self.sections.last_mut() {
			*section += 1;
		}
	}

	fn generate(&mut self) -> Result<String, Self::Error> {
		Ok(self.buf.clone())
	}
}