use crate::{
	wl::{
		doc::{self, DocGen},
	},
};

pub struct HtmlGenerator {
	buf: String,
	sections: Vec<SectionData>,
}

struct SectionData {
	count: i32,
	kind: &'static str,
}

impl HtmlGenerator {
	pub fn new() -> Self {
		HtmlGenerator {
			buf: String::new(),
			sections: vec![0],
		}
	}
}

impl DocGen for HtmlGenerator {
	type Error = ();

	fn add_paragraph(&mut self, text: &str) {
		
	}

	fn begin_section(&mut self, title: &str) {
		for (i, section) in self.sections.iter().enumerate() {
			self.buf.push_str(&format!("{}", section + 1));
			if i < self.sections.len() - 1 {
				self.buf.push_str(".");
			}
		}
		self.sections.push(SectionData { count: 0 });
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