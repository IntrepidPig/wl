use quick_xml::{
	Reader,
	events::{Event},
};
use thiserror::Error;

use wl_common::wire::ArgumentType;

#[derive(Debug)]
pub struct ProtocolDesc {
	pub name: String,
	pub copyright: String,
	pub interfaces: Vec<InterfaceDesc>,
}

#[derive(Debug)]
pub struct InterfaceDesc {
	pub name: String,
	pub version: i32,
	pub description: String,
	pub summary: String,
	pub requests: Vec<RequestDesc>,
	pub events: Vec<EventDesc>,
	pub enums: Vec<EnumDesc>,
}

#[derive(Debug)]
pub struct MessageDesc {
	pub name: String,
	pub since: Option<i32>,
	pub description: String,
	pub summary: String,
	pub arguments: Vec<ArgumentDesc>,
}

#[derive(Debug)]
pub struct RequestDesc {
	pub message: MessageDesc,
	pub destructor: bool,
}

#[derive(Debug)]
pub struct EventDesc {
	pub message: MessageDesc,
}

#[derive(Debug)]
pub struct ArgumentDesc {
	pub name: String,
	pub summary: String,
	pub arg_type: ArgumentType,
	pub interface: Option<String>,
	pub enum_type: Option<(Option<String>, String)>,
	pub allow_null: bool,
}

#[derive(Debug)]
pub struct EnumDesc {
	pub name: String,
	pub bitfield: bool,
	pub since: Option<i32>,
	pub description: String,
	pub summary: String,
	pub entries: Vec<EntryDesc>,
}

#[derive(Debug)]
pub struct EntryDesc {
	pub name: String,
	pub since: Option<i32>,
	// Can be interpreted as u32, by adding 2^31 // What did I mean by this
	pub value: i32,
	pub summary: String,
}

#[derive(Debug, Error)]
pub enum ProtocolParseError {
	#[error("Failed to parse protocol XML description")]
	XmlParseError(#[from] quick_xml::Error),
	#[error("Invalid protocol XML description: {0}")]
	InvalidXmlError(String),
	#[error("Protocol XML description contained invalid UTF-8")]
	Utf8Error(#[from] std::str::Utf8Error),
}

pub fn parse_protocol(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> Result<ProtocolDesc, ProtocolParseError> {
	let mut protocol = None;

	let tree = build_tree(reader, buf)?;

	for element in tree {
		match element {
			Element::Node(node) => {
				match node.name.as_str() {
					"protocol" => {
						if protocol.is_some() {
							return Err(ProtocolParseError::InvalidXmlError(String::from("Multiple protocol definitions found")));
						} else {
							protocol = Some(read_protocol(node)?);
						}
					},
					_ => return Err(ProtocolParseError::InvalidXmlError(format!("Unexpected root node '{}'", node.name))),
				}
			},
			Element::Text(_) => {},
		}
	}

	protocol.ok_or(ProtocolParseError::InvalidXmlError(String::from("No protocol element found")))
}

fn build_tree(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>) -> Result<Vec<Element>, ProtocolParseError> {
	let mut elements = Vec::new();
	let mut stack = Vec::new();

	loop {
		match reader.read_event(buf)? {
		    Event::Start(start) => {
				stack.push(Node {
					name: std::str::from_utf8(start.name())?.to_owned(),
					attributes: start.attributes().map(|res| res
						.map_err(ProtocolParseError::from)
						.and_then(|attr| -> Result<(String, String), ProtocolParseError> {
							Ok((std::str::from_utf8(attr.key)?.to_owned(), std::str::from_utf8(attr.unescaped_value()?.as_ref())?.to_owned()))
						}))
						.collect::<Result<Vec<_>, ProtocolParseError>>()?,
					children: Vec::new(),
				});
			},
		    Event::End(_) => {
				let this = stack.pop().unwrap();
				if let Some(last) = stack.last_mut() {
					last.children.push(Element::Node(this));
				} else {
					elements.push(Element::Node(this));
				}
			}
		    Event::Empty(empty) => {
				let node = Node {
					name: std::str::from_utf8(empty.name())?.to_owned(),
					attributes: empty.attributes().map(|res| res
						.map_err(ProtocolParseError::from)
						.and_then(|attr| -> Result<(String, String), ProtocolParseError> {
							Ok((std::str::from_utf8(attr.key)?.to_owned(), std::str::from_utf8(attr.unescaped_value()?.as_ref())?.to_owned()))
						}))
						.collect::<Result<Vec<_>, ProtocolParseError>>()?,
					children: Vec::new(),
				};
				if let Some(last) = stack.last_mut() {
					last.children.push(Element::Node(node));
				} else {
					elements.push(Element::Node(node));
				}
				
			}
		    Event::Text(text) => {
				let text = std::str::from_utf8(text.unescaped()?.as_ref())?.to_owned();
				if let Some(last) = stack.last_mut() {
					last.children.push(Element::Text(text));
				} else {
					elements.push(Element::Text(text));
				}
			}
		    Event::Comment(_) => {}
		    Event::CData(_) => {}
		    Event::Decl(_) => {}
		    Event::PI(_) => {}
		    Event::DocType(_) => {}
		    Event::Eof => {
				break;
			}
		}
	}

	Ok(elements)
}

pub fn read_protocol(node: Node) -> Result<ProtocolDesc, ProtocolParseError> {
	let mut interfaces = Vec::new();
	let mut copyright = None;
	let name = node.attributes
		.iter()
		.find_map(|(k, v)| if &*k == "name" { Some(v.clone()) } else { None })
		.ok_or(ProtocolParseError::InvalidXmlError(String::from("Protocol node has no 'name' attribute")))?;

	for element in node.children {
		match element {
			Element::Node(node) => {
				match node.name.as_str() {
					"copyright" => {
						if copyright.is_some() {
							return Err(ProtocolParseError::InvalidXmlError(String::from("Multiple copyright statements found")));
						} else {
							copyright = Some(read_copyright(node)?);
						}
					}
					"interface" => interfaces.push(read_interface(node)?),
					u => return Err(ProtocolParseError::InvalidXmlError(format!("Unexpected protocol child element '{}'", u))),
				}
			}
			Element::Text(_) => {},
		}
	}
	
	Ok(ProtocolDesc {
		name: name,
		copyright: copyright.ok_or(ProtocolParseError::InvalidXmlError(String::from("Protocol node has no copyright definition")))?,
		interfaces,
	})
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
	name: String,
	attributes: Vec<(String, String)>,
	children: Vec<Element>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Element {
	Text(String),
	Node(Node)
}

fn read_copyright(node: Node) -> Result<String, ProtocolParseError> {
	let mut content = None;
	for element in node.children {
		match element {
			Element::Node(_) => return Err(ProtocolParseError::InvalidXmlError(String::from("Unexpected node in copyright statement"))),
			Element::Text(text) => {
				if content.is_some() {
					return Err(ProtocolParseError::InvalidXmlError(String::from("Multiple text nodes found in copyright statement")));
				} else {
					content = Some(text);
				}
			},
		}
	}

	Ok(content.unwrap_or(String::new()))
}

fn read_interface(node: Node) -> Result<InterfaceDesc, ProtocolParseError> {
	let Node { name: _name, attributes, children} = node;

	let mut name = None;
	let mut version = None;
	for attribute in attributes {
		match attribute.0.as_str() {
			"name" => if name.replace(attribute.1).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Interface has multiple 'name' attributes")));
			},
			"version" => if version.replace(attribute.1.parse::<i32>().map_err(|_| ProtocolParseError::InvalidXmlError(String::from("Version was not an integer")))?).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Interface has multiple 'version' attributes")));
			},
			u => {
				return Err(ProtocolParseError::InvalidXmlError(format!("Got unexpected attribute for interface: '{}'", u)));
			}
		}
	}
	let name = name.ok_or(ProtocolParseError::InvalidXmlError(String::from("Interface was missing a 'name' attribute")))?;
	let version = version.ok_or(ProtocolParseError::InvalidXmlError(String::from("Interface was missing a 'version' attribute")))?;

	let mut description = None;
	let mut requests = Vec::new();
	let mut events = Vec::new();
	let mut enums = Vec::new();
	for element in children {
		match element {
			Element::Node(node) => {
				match node.name.as_str() {
					"description" => if description.replace(read_description(node)?).is_some() {
						return Err(ProtocolParseError::InvalidXmlError(String::from("Interface has multiple 'description' elements")));
					},
					"request" => requests.push(read_request(node)?),
					"event" => events.push(read_event(node)?),
					"enum" => enums.push(read_enum(node)?),
					u => {
						return Err(ProtocolParseError::InvalidXmlError(format!("Got unexpected element in interface: '{}'", u)));
					}
				}
			},
			Element::Text(_) => {},
		}
	}
	let (description, summary) = description.ok_or(ProtocolParseError::InvalidXmlError(String::from("Interface was missing a 'description' element")))?;

	Ok(InterfaceDesc {
	    name,
	    version,
	    description,
	    summary,
	    requests,
	    events,
	    enums,
	})
}

fn read_description(node: Node) -> Result<(String, String), ProtocolParseError> {
	let Node { name: _name, attributes, children} = node;

	let mut description = None;
	let mut summary = None;

	for (k, v) in attributes {
		match k.as_str() {
			"summary" => {
				if summary.replace(v).is_some() {
					return Err(ProtocolParseError::InvalidXmlError(String::from("Description has multiple 'summary' attributes")));
				}
			},
			u => return Err(ProtocolParseError::InvalidXmlError(format!("Description had unknown attribute '{}'", u))),
		}
	}

	for element in children {
		match element {
			Element::Text(text) => {
				if description.replace(text).is_some() {
					return Err(ProtocolParseError::InvalidXmlError(String::from("Description has multiple text children")));
				}
			},
			Element::Node(node) => {
				return Err(ProtocolParseError::InvalidXmlError(format!("Description has unexpected child node '{}'", node.name)));
			}
		}
	}

	//let description = description.ok_or(ProtocolParseError::InvalidXmlError(String::from("Description has no text child")))?;
	let description = description.unwrap_or(String::new()); // Apparently, the description of wl_keyboard.release only has a summary. Gross.
	let summary = summary.ok_or(ProtocolParseError::InvalidXmlError(String::from("Description is missing 'summary' attribute")))?;

	Ok((description, summary))
}

fn read_request(node: Node) -> Result<RequestDesc, ProtocolParseError> {
	let Node { name: _name, attributes, children} = node;

	let mut name = None;
	let mut destructor = None;
	let mut since = None;
	let mut description = None;
	let mut arguments = Vec::new();
	
	for (k, v) in attributes {
		match &*k {
			"name" => if name.replace(v).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Request has multiple 'name' attributes")));
			},
			"type" => match &*v {
				"destructor" => {
					if destructor.replace(true).is_some() {
						return Err(ProtocolParseError::InvalidXmlError(String::from("Request has multiple 'type' attributes")))
					}
				},
				u => return Err(ProtocolParseError::InvalidXmlError(format!("Request has unknown type '{}'", u))),
			},
			"since" => if since.replace(v.parse::<i32>().map_err(|_| ProtocolParseError::InvalidXmlError(String::from("Version was not an integer")))?).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Request has multiple 'since' attributes")));
			},
			u => return Err(ProtocolParseError::InvalidXmlError(format!("Unexpected request attribute '{}'", u))),
		}
	}

	//print!("{:#?}", children);
	for element in children {
		match element {
			Element::Node(node) => {
				match node.name.as_str() {
					"description" => if description.replace(read_description(node)?).is_some() {
						return Err(ProtocolParseError::InvalidXmlError(String::from("Request has multiple 'description' elements")));
					},
					"arg" => arguments.push(read_arg(node)?),
					u => return Err(ProtocolParseError::InvalidXmlError(format!("Request had unexpeceted child element '{}'", u))),
				}
			},
			Element::Text(_) => {},
		}
	}
	
	let name = name.ok_or(ProtocolParseError::InvalidXmlError(String::from("Request was missing 'name' attribute")))?;
	let destructor = destructor.unwrap_or(false);
	let (description, summary) = description.ok_or(ProtocolParseError::InvalidXmlError(String::from("Request was missing 'description' element")))?;
	
	let request_desc = RequestDesc {
		message: MessageDesc {
			name,
			since,
			description,
			summary,
			arguments,
		},
		destructor,
	};
	Ok(request_desc)
}

fn read_arg(node: Node) -> Result<ArgumentDesc, ProtocolParseError> {
	let Node { name: _name, attributes, children: _children } = node;

	let mut name = None;
	let mut summary = None;
	let mut arg_type = None;
	let mut interface = None;
	let mut enum_type = None;
	let mut allow_null = None;
	
	for (k, v) in attributes {
		match &*k {
			"name" => if name.replace(v).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Argument has multiple 'name' attributes")));
			},
			"type" => if arg_type.replace(ArgumentType::from_str(&v).ok_or(ProtocolParseError::InvalidXmlError(format!("Unknown argument type '{}'", v)))?).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Argument has multiple 'summary' attributes")));
			},
			"interface" => if interface.replace(v).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Argument has multiple 'interface' attributes")));
			},
			"summary" => if summary.replace(v).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Argument has multiple 'summary' attributes")));
			},
			"enum" => if enum_type.replace(v).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Argument has multiple 'enum' attributes")));
			},
			"allow-null" => if allow_null.replace(v.parse::<bool>().map_err(|_| ProtocolParseError::InvalidXmlError(format!("Unknown boolean '{}'", v)))?).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Argument has multiple 'allow-null' attributes")));
			},
			u => return Err(ProtocolParseError::InvalidXmlError(format!("Unknown argument attribute '{}'", u))),
		}
	}
	
	let name = name.ok_or(ProtocolParseError::InvalidXmlError(format!("Argument name not present")))?;
	//let summary = summary.ok_or(ProtocolParseError::InvalidXmlError(format!("Argument summary not present")))?;
	let summary = summary.unwrap_or(String::new()); // this is not epic
	let arg_type = arg_type.ok_or(ProtocolParseError::InvalidXmlError(format!("Argument type not present")))?;
	let enum_type = enum_type.map(|s| {
		let split = s.split(".").collect::<Vec<&str>>();
		if split.len() == 1 {
			Ok((None, split[0].to_owned()))
		} else if split.len() == 2 {
			Ok((Some(split[0].to_owned()), split[1].to_owned()))
		} else {
			Err(ProtocolParseError::InvalidXmlError(format!("Invalid enum type: {}", s)))
		}
	}).transpose()?;
	let allow_null = allow_null.unwrap_or(false);

	let arg_desc = ArgumentDesc {
		name,
		summary,
		arg_type,
		interface,
		enum_type,
		allow_null,
	};
	Ok(arg_desc)
}

fn read_event(node: Node) -> Result<EventDesc, ProtocolParseError> {
	let Node { name: _name, attributes, children} = node;

	let mut name = None;
	let mut since = None;
	let mut description = None;
	let mut arguments = Vec::new();
	
	for (k, v) in attributes {
		match &*k {
			"name" => if name.replace(v).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Event has multiple 'name' attributes")));
			},
			"since" => if since.replace(v.parse::<i32>().map_err(|_| ProtocolParseError::InvalidXmlError(String::from("Version was not an integer")))?).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Event has multiple 'since' attributes")));
			},
			u => return Err(ProtocolParseError::InvalidXmlError(format!("Unexpected attribute '{}' in event", u))),
		}
	}

	for element in children {
		match element {
			Element::Node(node) => {
				match node.name.as_str() {
					"description" => if description.replace(read_description(node)?).is_some() {
						return Err(ProtocolParseError::InvalidXmlError(String::from("Event has multiple 'description' elements")));
					},
					"arg" => arguments.push(read_arg(node)?),
					u => return Err(ProtocolParseError::InvalidXmlError(format!("Event had unexpeceted child element '{}'", u))),
				}
			},
			Element::Text(_) => {},
		}
	}
	
	let name = name.ok_or(ProtocolParseError::InvalidXmlError(String::from("Event was missing 'name' attribute")))?;
	let (description, summary) = description.ok_or(ProtocolParseError::InvalidXmlError(String::from("Event was missing 'description' element")))?;
	
	let event_desc = EventDesc {
		message: MessageDesc {
			name,
			since,
			description,
			summary,
			arguments,
		},
	};
	Ok(event_desc)
}

fn read_enum(node: Node) -> Result<EnumDesc, ProtocolParseError> {
	let Node { name: _name, attributes, children} = node;

	let mut name = None;
	let mut since = None;
	let mut bitfield = None;
	for (k, v) in attributes {
		match &*k {
			"name" => if name.replace(v).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Enum has multiple name attributes")));
			},
			"since" => if since.replace(v.parse::<i32>().map_err(|_| ProtocolParseError::InvalidXmlError(String::from("Version was not an integer")))?).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Enum has multiple 'since' attributes")));
			},
			"bitfield" => if bitfield.replace(v.parse::<bool>().map_err(|_| ProtocolParseError::InvalidXmlError(format!("Unknown boolean '{}'", v)))?).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Event has multiple 'bitfield' attributes")));
			},
			u => return Err(ProtocolParseError::InvalidXmlError(format!("Enum had unexpeceted attribute '{}'", u))),
		}
	}
	let name = name.ok_or(ProtocolParseError::InvalidXmlError(String::from("Enum missing 'name' attribute")))?;
	let bitfield = bitfield.unwrap_or(false);

	let mut description = None;
	let mut entries = Vec::new();
	for element in children {
		match element {
			Element::Node(node) => {
				match &*node.name {
					"description" => if description.replace(read_description(node)?).is_some() {
						return Err(ProtocolParseError::InvalidXmlError(String::from("Enum had multiple 'description' elements")));
					},
					"entry" => entries.push(read_entry(node)?),
					u => return Err(ProtocolParseError::InvalidXmlError(format!("Enum had unexpected child element '{}'", u))),
				}
			},
			Element::Text(_) => {},
		}
	}
	//let (description, summary) = description.ok_or(ProtocolParseError::InvalidXmlError(String::from("Enum missing 'description' element")))?;
	let (description, summary) = description.unwrap_or((String::new(), String::new())); // Some enums have no description (GNOME, plz stop inting)

	Ok(EnumDesc {
	    name,
	    bitfield,
	    since,
	    description,
	    summary,
	    entries,
	})
}

fn read_entry(node: Node) -> Result<EntryDesc, ProtocolParseError> {
	let Node { name: _name, attributes, children: _children } = node;

	let mut name = None;
	let mut since = None;
	let mut value = None;
	let mut summary = None;
	
	for (k, v) in attributes {
		match &*k {
			"name" => if name.replace(v).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Entry has multiple 'name' attributes")));
			},
			"since" => if since.replace(v.parse::<i32>().map_err(|_| ProtocolParseError::InvalidXmlError(String::from("Version was not an integer")))?).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Entry has multiple 'since' attributes")));
			},
			"value" => if value.replace(parse_value(&v).ok_or(ProtocolParseError::InvalidXmlError(format!("Version was not an integer: '{}'", v)))?).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Entry has multiple 'summary' attributes")));
			},
			"summary" => if summary.replace(v).is_some() {
				return Err(ProtocolParseError::InvalidXmlError(String::from("Entry has multiple 'summary' attributes")));
			},
			u => return Err(ProtocolParseError::InvalidXmlError(format!("Unknown entry attribute '{}'", u))),
		}
	}
	
	let name = name.ok_or(ProtocolParseError::InvalidXmlError(format!("Entry name not present")))?;
	let value = value.ok_or(ProtocolParseError::InvalidXmlError(format!("Entry value not present")))?;
	//let summary = summary.ok_or(ProtocolParseError::InvalidXmlError(format!("Entry summary not present")))?;
	let summary = summary.unwrap_or(String::new()); // Some entries have no summaries... (why ubiso- I mean, GNOME project!?)

	let entry_desc = EntryDesc {
		name,
		since,
		value,
		summary,
	};

	Ok(entry_desc)
}

fn parse_value(s: &str) -> Option<i32> {
	if s.starts_with("0x") {
		i32::from_str_radix(&s[2..], 16).ok()
	} else {
		i32::from_str_radix(s, 10).ok()
	}
}
