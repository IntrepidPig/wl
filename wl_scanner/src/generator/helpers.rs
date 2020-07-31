pub struct StructDefinition {
	pub derives: String,
	pub name: String,
	pub visibility: String,
	pub data: StructData,
}

pub enum StructData {
	Unit,
	Tuple(Vec<String>),
	Fields(Vec<(String, String)>),
}

pub fn define_struct(def: &StructDefinition) -> String {
	format!("#[derive({})]{} struct {}{}", def.derives, def.visibility, def.name, match def.data {
		StructData::Unit => ";".to_owned(),
		StructData::Tuple(ref fields) => format!("({})", fields.join(",")),
		StructData::Fields(ref fields) => format!("{{{}}}", fields.iter().map(|field| format!("pub {}:{},", field.0, field.1)).collect::<String>()),
	})
}

pub struct EnumDefinition {
	pub derives: String,
	pub name: String,
	pub visibility: String,
	pub variants: Vec<(String, StructData)>,
}

pub fn define_enum(def: &EnumDefinition) -> String {
	format!("#[derive({})]{} enum {}{{{}}}", def.derives, def.visibility, def.name, def.variants.iter().map(|variant| {
		format!("{}{}", variant.0, match variant.1 {
			StructData::Unit => ",".to_owned(),
			StructData::Tuple(ref fields) => format!("({}),", fields.join(",")),
			StructData::Fields(ref fields) => format!("{{{}}},", fields.iter().map(|field| format!("{}:{},", field.0, field.1)).collect::<String>()),
		})
	}).collect::<String>())
}