use thiserror::{Error};

use wl_common::wire::*;

use crate::{
	scanner::{*, ArgumentDesc},
	generator::{
		helpers::*,
	},
};

pub mod helpers;

// TODO! clean this mess up with lots of named consts and Don't-Repeat-Yourself-icisms

#[derive(Debug, Copy, Clone)]
enum MessageSide {
	Request,
	Event,
}

impl MessageSide {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Request => "Request",
			Self::Event => "Event",
		}
	}
}

#[derive(Debug, Error)]
pub enum ProtocolGenError {
	#[error("An error occurred while generating the protocol bindings")]
	Uknown,
}

pub fn generate_api(protocol: &ProtocolDesc) -> Result<String, ProtocolGenError> {
	let mut buf = String::new();
	
	for interface in &protocol.interfaces {
		buf.push_str(&generate_interface(interface)?);
	}

	buf.push_str(&generate_protocol_summary(&protocol.interfaces));

	buf.push_str("pub mod prelude {");
	for interface in &protocol.interfaces {
		buf.push_str("pub use super::");
		buf.push_str(&interface.name);
		buf.push_str("::{self, ");
		buf.push_str(&snake_to_camel(&interface.name));
		buf.push_str(", ");
		buf.push_str(&snake_to_camel(&interface.name));
		buf.push_str("Request");
		buf.push_str(", ");
		buf.push_str(&snake_to_camel(&interface.name));
		buf.push_str("Event");
		buf.push_str("};")
	}
	buf.push_str("}");

	Ok(buf)
}

fn generate_protocol_summary(interfaces: &[InterfaceDesc]) -> String {
	/* format!(
		"pub static INTERFACES: [DynInterface; {interface_count}] = [{}];",
		interfaces.iter().map(|interface| {
			format!("{}::as_dyn(),", interface.name)
		}).collect::<String>(),
		interface_count=interfaces.len(),
	) */
	//format!("pub static INTERFACES: [DynInterface; 0] = [];")
	String::new()
}

fn generate_interface(interface: &InterfaceDesc) -> Result<String, ProtocolGenError> {
	let enum_definitions = interface.enums.iter().map(|enum_desc| {
		let name = snake_to_camel(&enum_desc.name);
		if enum_desc.bitfield {
			format!(
				"bitflags!{{pub struct {}:u32{{{}}}}} \
				impl TryFrom<u32> for {} {{type Error=InvalidEnumValue;fn try_from(v:u32)->Result<Self,Self::Error>{{Self::from_bits(v).ok_or(InvalidEnumValue)}}}} \
				impl From<{}> for u32 {{fn from(v:{})->u32{{v.bits()}}}}",
				name,
				enum_desc.entries.iter().map(|entry| {
					format!(
						"const {}={};",
						sanitize_enum_variant_name(&snake_to_camel(&entry.name)).to_ascii_uppercase(),
						entry.value,
					)
				}).collect::<String>(),
				name,
				name,
				name,
			)
		} else {
			format!(
				"#[derive(Debug, Clone, Copy, PartialEq, Eq)]#[repr(u32)]pub enum {}{{{}}} \
				impl TryFrom<u32> for {} {{type Error=InvalidEnumValue;fn try_from(v:u32)->Result<Self,Self::Error>{{Ok(match v {{{}}})}}}} \
				impl TryFrom<i32> for {} {{type Error=InvalidEnumValue;fn try_from(v:i32)->Result<Self,Self::Error>{{Ok(match v {{{}}})}}}} \
				impl From<{}> for u32 {{fn from(v:{})->u32{{v as u32}}}} \
				impl From<{}> for i32 {{fn from(v:{})->i32{{i32::try_from(v as u32).unwrap()}}}}",
				name,
				enum_desc.entries.iter().map(|entry| {
					format!(
						"{}={},",
						sanitize_enum_variant_name(&snake_to_camel(&entry.name)),
						entry.value,
					)
				}).collect::<String>(),
				name,
				enum_desc.entries.iter().map(|entry| {
					format!(
						"{} => Self::{},",
						entry.value,
						sanitize_enum_variant_name(&snake_to_camel(&entry.name)),
					)
				}).chain(std::iter::once(String::from("_ => return Err(InvalidEnumValue),"))).collect::<String>(),
				name,
				enum_desc.entries.iter().map(|entry| {
					format!(
						"{} => Self::{},",
						entry.value,
						sanitize_enum_variant_name(&snake_to_camel(&entry.name)),
					)
				}).chain(std::iter::once(String::from("_ => return Err(InvalidEnumValue),"))).collect::<String>(),
				name,
				name,
				name,
				name,
			)
		}
	}).collect::<String>();

	let msg_from_header = "fn from_args(resources: &mut ResourceManager, client_handle: ClientHandle, opcode: u16, args: Vec<DynArgument>) -> Result<Self, FromArgsError>";
	let msg_into_header = "fn into_args(&self, resources: &ResourceManager, client_handle: ClientHandle) -> Result<(u16, Vec<DynArgument>), IntoArgsError>";

	let request_struct_definitions = interface.requests
		.iter()
		.filter(|request| !request.arguments.is_empty())
		.map(|request| {
			define_struct(&StructDefinition {
				derives: String::from("Debug, Clone"),
				name: format!("{}Request", snake_to_camel(&request.name)),
				visibility: String::from("pub"),
				data: StructData::Fields(request.arguments.iter().map(|argument| {
					let mut arg_type_clause = generate_argument_type(
						argument.arg_type,
						argument.interface.as_ref().map(String::as_str),
						argument.enum_type.as_ref().map(|(ns, enum_type)| {
							(ns.as_ref().map(String::as_str), enum_type.as_str())
						})
					);
					if argument.allow_null {
						arg_type_clause = format!("Option<{}>", arg_type_clause);
					}
					(argument.name.clone(), arg_type_clause)
				}).collect()),
			})
		})
		.collect::<String>();
	let requests_enum = define_enum(&EnumDefinition {
		derives: String::from("Debug, Clone"),
		name: format!("{}Request", snake_to_camel(&interface.name)),
		visibility: String::from("pub"),
		variants: interface.requests.iter().map(|request| {
			(snake_to_camel(&request.name), if request.arguments.is_empty() {
				StructData::Unit
			} else {
				StructData::Tuple(vec![format!("{}Request", snake_to_camel(&request.name))])
			})
		}).collect(),
	});
	let request_impl = format!(
		"impl Message for {}Request{{fn opcode(&self)->u16{{{}}}{}{{{}}}{}{{{}}}}}",
		snake_to_camel(&interface.name),
		create_request_opcode_fn(&interface),
		msg_from_header,
		create_from_bytes_fn(MessageSide::Request, &interface),
		msg_into_header,
		create_to_args_fn(MessageSide::Request, &interface),
	);

	let event_struct_definitions = interface.events.iter()
		.filter(|event| !event.arguments.is_empty())
		.map(|event| {
			define_struct(&StructDefinition {
				derives: String::from("Debug, Clone"),
				name: format!("{}Event", snake_to_camel(&event.name)),
				visibility: String::from("pub"),
				data: StructData::Fields(event.arguments.iter().map(|argument| {
					let mut arg_type_clause = generate_argument_type(
						argument.arg_type,
						argument.interface.as_ref().map(String::as_str),
						argument.enum_type.as_ref().map(|(ns, enum_type)| {
							(ns.as_ref().map(String::as_str), enum_type.as_str())
						}),
					);
					if argument.allow_null {
						arg_type_clause = format!("Option<{}>", arg_type_clause);
					}
					(argument.name.clone(), arg_type_clause)
				}).collect()),
			})
		})
		.collect::<String>();
	let events_enum = define_enum(&EnumDefinition {
		derives: String::from("Debug, Clone"),
		name: format!("{}Event", snake_to_camel(&interface.name)),
		visibility: String::from("pub"),
		variants: interface.events.iter().map(|event| {
			(snake_to_camel(&event.name), if event.arguments.is_empty() {
				StructData::Unit
			} else {
				StructData::Tuple(vec![format!("{}Event", snake_to_camel(&event.name))])
			})
		}).collect(),
	});
	let event_impl = format!(
		"impl Message for {}Event{{fn opcode(&self)->u16{{{}}}{}{{{}}}{}{{{}}}}}",
		snake_to_camel(&interface.name),
		create_event_opcode_fn(&interface),
		msg_from_header,
		create_from_bytes_fn(MessageSide::Event, &interface),
		msg_into_header,
		create_to_args_fn(MessageSide::Event, &interface),
	);

	let requests_array = generate_requests_arg_array(&interface);
	let events_array = generate_events_arg_array(&interface);

	let interface_impl = format!{
		"
		static _COW: Cow<'static, str> = Cow::Borrowed(\"{snake_name}\");
		impl Interface for {camel_name}{{
			type Request={camel_name}Request;
			type Event={camel_name}Event;
			const NAME: &'static str = \"{snake_name}\";
			
			const VERSION: u32 = {version};
			const REQUESTS: &'static [&'static [ArgumentDesc]] = &[{requests}];
			const EVENTS: &'static [&'static [ArgumentDesc]] = &[{events}];
			fn new()->Self{{Self}}
			fn as_dyn() -> DynInterface {{
				DynInterface {{
					name: Cow::Borrowed(Self::NAME),
					version: Self::VERSION,
					requests: Self::REQUESTS,
					events: Self::EVENTS,
				}}
			}}
		}}
		",
		snake_name=interface.name,
		camel_name=snake_to_camel(&interface.name),
		version=interface.version,
		requests=requests_array,
		events=events_array,
	};

	let buf = format!(
		"pub mod {}{{#![allow(unused)]
		use super::*;
		use bitflags::bitflags;
		use std::os::unix::io::RawFd;
		use std::convert::TryFrom;
		use std::ffi::CString;
		use std::borrow::Cow;
		use byteorder::{{ByteOrder,NativeEndian,ReadBytesExt,WriteBytesExt}};
		use std::io::Cursor;
		use wl_common::{{
			protocol::{{Interface, InterfaceTitle, DynInterface, Message, InvalidEnumValue, FromArgsError, IntoArgsError}},
			wire::{{RawMessage, RawMessageReader, MessageHeader, Fixed, ArgumentDesc, ArgumentType, DynArgument, DynArgumentReader}},
			resource::{{Resource, ResourceManager, ClientHandle, Untyped}},
		}};\
		{}{}{}{}{}{}{}{}{}}}",
		interface.name,
		define_struct(&StructDefinition {
			derives: String::from("Debug, Clone, Copy"),
			name: snake_to_camel(&interface.name),
			visibility: "pub".to_owned(),
			data: StructData::Unit,
		}),
		interface_impl,
		enum_definitions,
		requests_enum,
		request_impl,
		request_struct_definitions,
		events_enum,
		event_struct_definitions,
		event_impl,
	);

	Ok(buf)
}

fn generate_wire_arg_desc(arg: &ArgumentDesc) -> String {
	format!(
		"ArgumentDesc {{ arg_type: {}, interface: {}, allow_null: {} }}",
		match arg.arg_type {
			ArgumentType::Int => "ArgumentType::Int",
			ArgumentType::Uint => "ArgumentType::Uint",
			ArgumentType::Fixed => "ArgumentType::Fixed",
			ArgumentType::String => "ArgumentType::String",
			ArgumentType::Object => "ArgumentType::Object",
			ArgumentType::NewId => "ArgumentType::NewId",
			ArgumentType::Array => "ArgumentType::Array",
			ArgumentType::Fd => "ArgumentType::Fd",
		},
		if let Some(ref interface) = arg.interface {
			format!("Some(\"{}\")", interface)
		} else {
			format!("None")
		},
		arg.allow_null,
	)
}

fn generate_arg_array(arguments: &[ArgumentDesc]) -> String {
	arguments.iter().map(|arg| {
		format!(
			"{},",
			generate_wire_arg_desc(arg)
		)
	}).collect::<String>()
}

fn generate_requests_arg_array(interface: &InterfaceDesc) -> String {
	interface.requests.iter().map(|request| {
		format!(
			"&[{}],",
			generate_arg_array(&request.arguments)
		)
	}).collect::<String>()
}

fn generate_events_arg_array(interface: &InterfaceDesc) -> String {
	interface.events.iter().map(|event| {
		format!(
			"&[{}],",
			generate_arg_array(&event.arguments)
		)
	}).collect::<String>()
}

fn generate_argument_type(arg_type: ArgumentType, interface: Option<&str>, enum_type: Option<(Option<&str>, &str)>) -> String {
	match arg_type {
	    ArgumentType::Int | ArgumentType::Uint => {
			if let Some((ns, enum_type)) = enum_type {
				if let Some(ns) = ns {
					format!("super::{}::{}", ns, snake_to_camel(enum_type))
				} else {
					snake_to_camel(enum_type)
				}
			} else if arg_type == ArgumentType::Int {
				"i32".to_owned()
			} else {
				"u32".to_owned()
			}
		},
	    ArgumentType::Fixed => "Fixed".to_owned(),
	    ArgumentType::String => "Vec<u8>".to_owned(),
	    ArgumentType::Object | ArgumentType::NewId => {
			if let Some(interface) = interface {
				format!("Resource<super::{}::{}>", interface, snake_to_camel(interface))
			} else {
				String::from("Resource<Untyped>")
			}
		},
	    ArgumentType::Array => "Vec<u8>".to_owned(),
	    ArgumentType::Fd => "RawFd".to_owned(),
	}
}

fn create_from_bytes_fn(side: MessageSide, interface: &InterfaceDesc) -> String {
	match side {
		MessageSide::Request => {
			format!(
				"let mut reader = DynArgumentReader::from_args(args);Ok(match opcode {{{}}})",
				interface.requests.iter().enumerate().map(|(i, request)| {
					format!(
						"{}=>{{{}}},",
						i,
						create_message_parser(side, &interface.name, &request.name, &request.arguments),
					)
				}).chain(std::iter::once("_=>return Err(FromArgsError::UnknownOpcode(opcode)),".to_owned())).collect::<String>(),
			)
		},
		MessageSide::Event => {
			format!(
				"let mut reader = DynArgumentReader::from_args(args);Ok(match opcode {{{}}})",
				interface.events.iter().enumerate().map(|(i, event)| {
					format!(
						"{}=>{{{}}},",
						i,
						create_message_parser(side, &interface.name, &event.name, &event.arguments),
					)
				}).chain(std::iter::once("_=>return Err(FromArgsError::UnknownOpcode(opcode)),".to_owned())).collect::<String>(),
			)
		}
	}
}

fn create_message_parser(side: MessageSide, interface_name: &str, message_name: &str, arguments: &[ArgumentDesc]) -> String {
	arguments.iter().enumerate().map(|(i, arg)| {
		let mut buf = match arg.arg_type {
			ArgumentType::Int => format!("let val{} = reader.next_int()?;", i),
		    ArgumentType::Uint => format!("let val{} = reader.next_uint()?;", i),
		    ArgumentType::Fixed => format!("let val{} = reader.next_fixed()?;", i),
		    ArgumentType::String => {
				if arg.allow_null {
					format!("let val{} = reader.next_string()?;", i)
				} else {
					format!("let val{} = reader.next_string()?.ok_or(FromArgsError::NullArgument)?;", i)
				}
			},
		    ArgumentType::Object => {
				if arg.allow_null {
					if arg.interface.is_none() {
						format!("let val{i} = reader.next_object()?;", i=i)
					} else {
						format!("let val{i} = reader.next_object()?.map(|r| {{let r = r.downcast_unchecked();resources.update_resource_interface_to(&r);r}});", i=i) // TODO downcast checked? client bug could possibly cause a server panic if not
					}
				} else {
					if arg.interface.is_none() {
						format!("let val{i} = reader.next_object()?.ok_or(FromArgsError::NullArgument)?;", i=i)
					} else {
						format!("let val{i} = reader.next_object()?.ok_or(FromArgsError::NullArgument)?.downcast_unchecked();resources.update_resource_interface_to(&val{i});", i=i)
					}
				}
			},
			ArgumentType::NewId => {
				if arg.interface.is_none() {
					format!("let val{i} = reader.next_new_id()?;", i=i)
				} else {
					format!("let val{i} = reader.next_new_id()?.downcast_unchecked();resources.update_resource_interface_to(&val{i});", i=i)
				}
			},
		    ArgumentType::Array => format!("let val{} = reader.next_array()?;", i),
		    ArgumentType::Fd => format!("let val{} = reader.next_fd()?;", i),
		};
		if let Some((ref ns , ref enum_type)) = arg.enum_type {
			let enum_type = sanitize_enum_variant_name(&snake_to_camel(enum_type));
			if let Some(ref ns) = ns {
				buf.push_str(&format!(
					"let val{} = super::{}::{}::try_from(val{})?;",
					i,
					ns,
					enum_type,
					i,
				))
			} else {
				buf.push_str(&format!(
					"let val{} = {}::try_from(val{})?;",
					i,
					enum_type,
					i,
				))
			}
		}
		buf
	}).chain(std::iter::once(format!(
		"{}{}::{}{}",
		snake_to_camel(interface_name),
		side.as_str(),
		snake_to_camel(&sanitize_enum_variant_name(message_name)),
		if arguments.is_empty() {
			String::new()
		} else {
			format!(
				"({}{} {{{}}})",
				snake_to_camel(&sanitize_enum_variant_name(message_name)),
				side.as_str(),
				arguments.iter().enumerate().map(|(i, argument)| {
					format!("{}: val{},", argument.name, i)
				}).collect::<String>()
			)
		},
	))).collect()
}

// TODO remove clones of strings and arrays
fn create_message_writer(arguments: &[ArgumentDesc]) -> String {
	arguments.iter().map(|argument| {
		match argument.arg_type {
		    ArgumentType::Int => format!("args.push(DynArgument::Int(data.{}.into()));", argument.name),
		    ArgumentType::Uint => format!("args.push(DynArgument::Uint(data.{}.into()));", argument.name),
		    ArgumentType::Fixed => format!("args.push(DynArgument::Fixed(data.{}));", argument.name),
		    ArgumentType::String => {
				if argument.allow_null {
					format!("args.push(DynArgument::String(data.{}.clone()));", argument.name)
				} else {
					format!("args.push(DynArgument::String(Some(data.{}.clone())));", argument.name)
				}
			}
		    ArgumentType::Object => {
				if argument.allow_null {
					format!("if let Some(resource) = data.{}.clone() {{
						args.push(DynArgument::Object(Some(resource.to_untyped())));
					}} else {{
						args.push(DynArgument::Object(None));
					}};", argument.name)
				} else {
					format!("args.push(DynArgument::Object(Some(data.{}.to_untyped())));", argument.name)
				}
			}
			ArgumentType::NewId => {
				format!("args.push(DynArgument::NewId(data.{}.to_untyped(), {}));", argument.name, if let Some(ref interface) = argument.interface {
					format!("Some(InterfaceTitle::new(\"{}\", 1))", interface) // TODO probably get rid of version number
				} else {
					format!("None")
				})
			},
		    ArgumentType::Array => format!("args.push(DynArgument::Array(data.{}.clone()));", argument.name), // TODO handle allow_null
		    ArgumentType::Fd => format!("args.push(DynArgument::Fd(data.{}));", argument.name),
		}
	}).collect()
}

fn create_request_opcode_fn(interface: &InterfaceDesc) -> String {
	format!(
		"match *self{{{}}}",
		interface.requests.iter().enumerate().map(|(i, request)| {
			format!(
				"Self::{}{} => {},",
				sanitize_enum_variant_name(&snake_to_camel(&request.name)),
				if request.arguments.is_empty() { "" } else { "(_)" },
				i,
			)
		}).collect::<String>()
	)
}

fn create_to_args_fn(side: MessageSide, interface: &InterfaceDesc) -> String {
	match side {
		MessageSide::Request => {
			format!(
				"Ok(match *self {{{}}})",
				interface.requests.iter().map(|request|{
					if request.arguments.is_empty() {
						format!(
							"{}Request::{}=>(self.opcode(),Vec::new()),",
							snake_to_camel(&interface.name),
							snake_to_camel(&request.name),
						)
					} else {
						format!(
							"{}Request::{}(ref data)=>{{let mut args = Vec::new();{}(self.opcode(),args)}},",
							snake_to_camel(&interface.name),
							snake_to_camel(&request.name),
							create_message_writer(&request.arguments),
						)
					}
				}).collect::<String>(),
			)
		},
		MessageSide::Event => {
			format!(
				"Ok(match *self {{{}}})",
				interface.events.iter().map(|event|{
					if event.arguments.is_empty() {
						format!(
							"{}Event::{}=>(self.opcode(),Vec::new()),",
							snake_to_camel(&interface.name),
							snake_to_camel(&event.name),
						)
					} else {
						format!(
							"{}Event::{}(ref data)=>{{let mut args = Vec::new();{}(self.opcode(),args)}},",
							snake_to_camel(&interface.name),
							snake_to_camel(&event.name),
							create_message_writer(&event.arguments),
						)
					}
				}).collect::<String>(),
			)
		},
	}
}


fn create_event_opcode_fn(interface: &InterfaceDesc) -> String {
	format!(
		"match *self{{{}}}",
		interface.events.iter().enumerate().map(|(i, event)| {
			format!(
				"Self::{}{} => {},",
				sanitize_enum_variant_name(&snake_to_camel(&event.name)),
				if event.arguments.is_empty() { "" } else { "(_)" },
				i,
			)
		}).collect::<String>()
	)
}

#[test]
fn snake_camel_test() {
	assert_eq!(snake_to_camel("wow_cool"), String::from("WowCool"));
	assert_eq!(snake_to_camel("wow"), String::from("Wow"));
}

fn snake_to_camel(snake: &str) -> String {
	let mut buf = String::with_capacity(snake.len());

	let mut last_snake = None;
	for c in snake.chars() {
		if let Some(last_snake) = last_snake {
			if c != '_' {
				if last_snake == '_' {
					let c = c.to_ascii_uppercase();
					buf.push(c);
				} else {
					buf.push(c);
				}
			}
		} else {
			let c = c.to_ascii_uppercase();
			buf.push(c);
		}

		last_snake = Some(c)
	}

	buf
}

fn sanitize_enum_variant_name(name: &str) -> String {
	if name.parse::<u32>().is_ok() {
		format!("_{}", name)
	} else {
		name.to_owned()
	}
}