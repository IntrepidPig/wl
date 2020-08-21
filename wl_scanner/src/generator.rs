use std::convert::TryInto;

use quote::{quote, format_ident};
use proc_macro2::{TokenStream, Ident, Span, Literal};

use wl_common::wire::*;

use crate::{
	scanner::{*, ArgumentDesc},
};

pub mod helpers;

#[derive(Debug, Copy, Clone)]
pub(crate) enum MessageSide {
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

pub fn generate_api(protocol: &ProtocolDesc) -> String {
	let interfaces_code = protocol.interfaces.iter().map(|interface| generate_interface(interface));
	let prelude_uses_code = protocol.interfaces.iter().map(|interface| {
		let name = Ident::new(&interface.name, Span::call_site());
		let camel_name = Ident::new(&snake_to_camel(&interface.name), Span::call_site());
		let request_name = format_ident!("{}Request", camel_name);
		let event_name = format_ident!("{}Event", camel_name);
		quote!(pub use super::#name::{self, #camel_name, #request_name, #event_name})
	}).collect::<Vec<_>>();

	let code = quote!(
		#(#interfaces_code)*

		pub mod prelude {
			#(#prelude_uses_code;)*
		}
	);

	code.to_string()
}

fn generate_enum_definition(enum_desc: &EnumDesc) -> TokenStream {
	let name = Ident::new(&snake_to_camel(&enum_desc.name), Span::call_site());
	if enum_desc.bitfield {
		let entries = enum_desc.entries.iter().map(|entry| {
			let entry_name = Ident::new(&sanitize_enum_variant_name(&entry.name).to_ascii_uppercase(), Span::call_site());
			let entry_value = Literal::i32_unsuffixed(entry.value);
			quote! {
				const #entry_name = #entry_value;
			}
		});
		quote!(
			bitflags! {
				pub struct #name: u32 {
					#(#entries)*
				}
			}

			impl TryFrom<u32> for #name {
				type Error = InvalidEnumValue;

				fn try_from(v: u32) -> Result<Self, Self::Error> {
					Self::from_bits(v).ok_or(InvalidEnumValue)
				}
			}

			impl From<#name> for u32 {
				fn from(v: #name) -> u32 {
					v.bits()
				}
			}
		)
	} else {
		let variants = enum_desc.entries.iter().map(|entry| {
			let entry_name = Ident::new(&sanitize_enum_variant_name(&snake_to_camel(&entry.name)), Span::call_site());
			let entry_value = Literal::i32_unsuffixed(entry.value);
			quote!(#entry_name = #entry_value)
		});
		let from_matches = enum_desc.entries.iter().map(|entry| {
			let entry_name = Ident::new(&sanitize_enum_variant_name(&snake_to_camel(&entry.name)), Span::call_site());
			let entry_value = Literal::i32_unsuffixed(entry.value);
			quote!(#entry_value => Self::#entry_name)
		});
		let from_matches_2 = from_matches.clone();
		quote! {
			#[derive(Debug, Clone, Copy, PartialEq, Eq)]
			#[repr(u32)]
			pub enum #name {
				#(#variants,)*
			}

			impl TryFrom<u32> for #name {
				type Error = InvalidEnumValue;

				fn try_from(v: u32) -> Result<Self, Self::Error> {
					Ok(match v {
						#(#from_matches,)*
						_ => return Err(InvalidEnumValue),
					})
				}
			}

			impl TryFrom<i32> for #name {
				type Error = InvalidEnumValue;

				fn try_from(v: i32) -> Result<Self, Self::Error> {
					Ok(match v {
						#(#from_matches_2,)*
						_ => return Err(InvalidEnumValue),
					})
				}
			}

			impl From<#name> for u32 {
				fn from(v: #name) -> u32 {
					v as u32
				}
			}

			impl From<#name> for i32 {
				fn from(v: #name) -> i32 {
					i32::try_from(v as u32).unwrap()
				}
			}
		}
	}
}

fn generate_argument_type(argument: &ArgumentDesc) -> TokenStream {
	match argument.arg_type {
	    ArgumentType::Int | ArgumentType::Uint => {
			if let Some((ref ns, ref enum_type)) = argument.enum_type {
				let enum_type = Ident::new(&snake_to_camel(enum_type), Span::call_site());
				if let Some(ns) = ns {
					let ns = Ident::new(ns, Span::call_site());
					quote!(super::#ns::#enum_type)
				} else {
					quote!(#enum_type)
				}
			} else if argument.arg_type == ArgumentType::Int {
				quote!(i32)
			} else {
				quote!(u32)
			}
		},
	    ArgumentType::Fixed => quote!(Fixed),
		ArgumentType::String => {
			if argument.allow_null {
				quote!(Option<Vec<u8>>)
			} else {
				quote!(Vec<u8>)
			}
		},
		ArgumentType::Object => {
			let interface = if let Some(ref interface) = argument.interface {
				let interface_name = Ident::new(&snake_to_camel(interface), Span::call_site());
				let interface = Ident::new(interface, Span::call_site());
				quote!(super::#interface::#interface_name)
			} else {
				quote!(Untyped)
			};
			if argument.allow_null {
				quote!(Option<Resource<#interface>>)
			} else {
				quote!(Resource<#interface>)
			}
		},
	    ArgumentType::NewId => {
			let interface = if let Some(ref interface) = argument.interface {
				let interface_name = Ident::new(&snake_to_camel(interface), Span::call_site());
				let interface = Ident::new(interface, Span::call_site());
				quote!(super::#interface::#interface_name)
			} else {
				quote!(Untyped)
			};
			quote!(NewResource<#interface>)
		},
	    ArgumentType::Array => quote!(Vec<u8>),
	    ArgumentType::Fd => quote!(RawFd),
	}
}

fn generate_message_struct_definition(message: &MessageDesc, side: MessageSide) -> TokenStream {
	let struct_name = format_ident!("{}{}", snake_to_camel(&message.name), side.as_str());
	let struct_fields = message.arguments.iter().map(|argument| {
		let argument_name = Ident::new(&argument.name, Span::call_site());
		let argument_type = generate_argument_type(argument);
		quote!(pub #argument_name: #argument_type)
	});
	quote! {
		#[derive(Debug)]
		pub struct #struct_name {
			#(#struct_fields,)*
		}
	}
}

fn generate_message_enum(interface: &InterfaceDesc, side: MessageSide) -> TokenStream {
	let name = format_ident!("{}{}", snake_to_camel(&interface.name), side.as_str());
	let mut requests_iter = interface.requests.iter().map(|request| &request.message);
	let mut events_iter = interface.events.iter().map(|event| &event.message);
	let messages_iter: &mut dyn Iterator<Item=&MessageDesc> = match side {
		MessageSide::Request => &mut requests_iter,
		MessageSide::Event => &mut events_iter,
	};
	let variants = messages_iter.map(|message| {
		let name = Ident::new(&snake_to_camel(&message.name), Span::call_site());
		let contents_name = format_ident!("{}{}", name, side.as_str());
		let contents = if message.arguments.is_empty() { quote!() } else { quote!((#contents_name)) };
		quote!(#name#contents)
	});
	quote! {
		#[derive(Debug)]
		pub enum #name {
			#(#variants,)*
		}
	}
}

fn generate_message_impl(interface: &InterfaceDesc, side: MessageSide) -> TokenStream {
	let name = format_ident!("{}{}", snake_to_camel(&interface.name), side.as_str());
	let opcode_fn = generate_opcode_fn(interface, side);
	let from_args_fn = generate_from_args_fn(interface, side);
	let into_args_fn = generate_into_args_fn(interface, side);
	quote! {
		impl Message for #name {
			type ClientMap = ClientMap;

			#opcode_fn

			#from_args_fn

			#into_args_fn
		}
	}
}

fn generate_interface_impl(interface: &InterfaceDesc) -> TokenStream {
	let requests_array = generate_arg_arrays(&interface, MessageSide::Request);
	let events_array = generate_arg_arrays(&interface, MessageSide::Event);

	let snake_name = Ident::new(&interface.name, Span::call_site());
	let camel_name = Ident::new(&snake_to_camel(&interface.name), Span::call_site());
	let camel_name_request = format_ident!("{}Request", camel_name);
	let camel_name_event = format_ident!("{}Event", camel_name);
	let snake_name_str = format!("\"{}\"", snake_name);
	let version = Literal::i32_unsuffixed(interface.version);

	quote! {
		static _COW: Cow<'static, str> = Cow::Borrowed(#snake_name_str);

		impl Interface for #camel_name {
			type Request = #camel_name_request;
			type Event = #camel_name_event;

			const NAME: &'static str = #snake_name_str;
			const VERSION: u32 = #version;
			const REQUESTS: &'static [&'static [ArgumentDesc]] = #requests_array;
			const EVENTS: &'static [&'static [ArgumentDesc]] = #events_array;

			fn new() -> Self {
				Self
			}

			fn as_dyn() -> DynInterface {
				DynInterface {
					name: Cow::Borrowed(Self::NAME),
					version: Self::VERSION,
					requests: Self::REQUESTS,
					events: Self::EVENTS,
				}
			}
		}
	}
}

fn generate_interface(interface: &InterfaceDesc) -> TokenStream {
	let enum_definitions = interface.enums.iter().map(generate_enum_definition);

	let request_struct_definitions = interface.requests.iter().map(|request| generate_message_struct_definition(&request.message, MessageSide::Request));
	let requests_enum = generate_message_enum(interface, MessageSide::Request);
	let request_impl = generate_message_impl(interface, MessageSide::Request);

	let event_struct_definitions = interface.events.iter().map(|event| generate_message_struct_definition(&event.message, MessageSide::Event));
	let events_enum = generate_message_enum(interface, MessageSide::Event);
	let event_impl = generate_message_impl(interface, MessageSide::Event);
	
	let interface_impl = generate_interface_impl(interface);

	let interface_name = Ident::new(&interface.name, Span::call_site());
	let interface_camel_name = Ident::new(&snake_to_camel(&interface.name), Span::call_site());

	quote! {
		pub mod #interface_name {
			#![allow(unused)]
			use super::*;
			use bitflags::bitflags;
			use std::os::unix::io::RawFd;
			use std::convert::TryFrom;
			use std::borrow::Cow;
			use byteorder::{ByteOrder, NativeEndian, ReadBytesExt, WriteBytesExt};
			use wl_common::{
				interface::{Interface, InterfaceTitle, DynInterface, Message, InvalidEnumValue, FromArgsError, IntoArgsError},
				wire::{ArgumentDesc, ArgumentType, DynArgument, DynArgumentReader, Fixed},
			};

			#[derive(Debug, Clone, Copy)]
			pub struct #interface_camel_name;

			#interface_impl

			#(#enum_definitions)*

			#requests_enum

			#request_impl
			
			#(#request_struct_definitions)*

			#events_enum

			#event_impl

			#(#event_struct_definitions)*
		}
	}
}

fn generate_wire_arg_desc(arg: &ArgumentDesc) -> TokenStream {
	let arg_type = match arg.arg_type {
		ArgumentType::Int => quote!(ArgumentType::Int),
		ArgumentType::Uint => quote!(ArgumentType::Uint),
		ArgumentType::Fixed => quote!(ArgumentType::Fixed),
		ArgumentType::String => quote!(ArgumentType::String),
		ArgumentType::Object => quote!(ArgumentType::Object),
		ArgumentType::NewId => quote!(ArgumentType::NewId),
		ArgumentType::Array => quote!(ArgumentType::Array),
		ArgumentType::Fd => quote!(ArgumentType::Fd),
	};
	let interface = if let Some(ref interface) = arg.interface {
		quote!(Some(#interface))
	} else {
		quote!(None)
	};
	let allow_null = arg.allow_null;

	quote! {
		ArgumentDesc {
			arg_type: #arg_type,
			interface: #interface,
			allow_null: #allow_null,
		}
	}
}

fn generate_arg_arrays(interface: &InterfaceDesc, side: MessageSide) -> TokenStream {
	let mut requests_iter = interface.requests.iter().map(|request| &request.message);
	let mut events_iter = interface.events.iter().map(|event| &event.message);
	let messages_iter: &mut dyn Iterator<Item=&MessageDesc> = match side {
		MessageSide::Request => &mut requests_iter,
		MessageSide::Event => &mut events_iter,
	};
	let arg_arrays_iter = messages_iter.map(|message| {
		let arg_array_iter = message.arguments.iter().map(|argument| {
			generate_wire_arg_desc(argument)
		});
		quote! {
			#(#arg_array_iter,)*
		}
	});
	quote! {
		&[#(&[#arg_arrays_iter],)*]
	}
}

fn generate_from_args_fn(interface: &InterfaceDesc, side: MessageSide) -> TokenStream {
	let mut requests_iter = interface.requests.iter().map(|request| &request.message);
	let mut events_iter = interface.events.iter().map(|event| &event.message);
	let messages_iter: &mut dyn Iterator<Item=&MessageDesc> = match side {
		MessageSide::Request => &mut requests_iter,
		MessageSide::Event => &mut events_iter,
	};
	let message_parser_match_body = messages_iter.enumerate().map(|(i, message)| {
		let message_parser = generate_message_parser(interface, message, side);
		let opcode = Literal::u16_unsuffixed(i.try_into().unwrap());
		quote! {
			#opcode => {
				#message_parser
			}
		}
	});

	quote! {
		fn from_args(client_map: Self::ClientMap, opcode: u16, args: Vec<DynArgument>) -> Result<Self, FromArgsError> {
			let mut reader = DynArgumentReader::from_args(args);
			Ok(match opcode {
				#(#message_parser_match_body,)*
				_ => return Err(FromArgsError::UnknownOpcode(opcode)),
			})
		}
	}
}

fn generate_opcode_fn(interface: &InterfaceDesc, side: MessageSide) -> TokenStream {
	let mut requests_iter = interface.requests.iter().map(|request| &request.message);
	let mut events_iter = interface.events.iter().map(|event| &event.message);
	let messages_iter: &mut dyn Iterator<Item=&MessageDesc> = match side {
		MessageSide::Request => &mut requests_iter,
		MessageSide::Event => &mut events_iter,
	};
	let opcode_match_arms = messages_iter.enumerate().map(|(i, message)| {
		let variant = Ident::new(&sanitize_enum_variant_name(&snake_to_camel(&message.name)), Span::call_site());
		let contents = if message.arguments.is_empty() { quote!() } else { quote!((_)) };
		let opcode = Literal::u16_unsuffixed(i.try_into().unwrap());
		quote!(Self::#variant#contents => #opcode)
	});

	quote! {
		fn opcode(&self) -> u16 {
			match *self {
				#(#opcode_match_arms,)*
			}
		}
	}
}

fn generate_message_parser(interface: &InterfaceDesc, message: &MessageDesc, side: MessageSide) -> TokenStream {
	let steps = message.arguments.iter().enumerate().map(|(i, arg)| {
		let val = format_ident!("val{}", i);
		let val_create = match arg.arg_type {
			ArgumentType::Int => quote!(let #val = reader.next_int()?;),
			ArgumentType::Uint => quote!(let #val = reader.next_uint()?;),
			ArgumentType::Fixed => quote!(let #val = reader.next_fixed()?;),
			ArgumentType::String => {
				if arg.allow_null {
					quote!(let #val = reader.next_string()?;)
				} else {
					quote!(let #val = reader.next_string()?.ok_or(FromArgsError::NullArgument)?;)
				}
			},
			ArgumentType::Object => {
				let create_object_opt = if arg.interface.is_some() {
					quote! {
						let #val = reader.next_object()?.map(|id| client_map.try_get_object(id).ok_or(FromArgsError::ResourceDoesntExist)).transpose()?;
					}
				} else {
					quote! {
						let #val = reader.next_object()?.map(|id| client_map.try_get_object_untyped(id).ok_or(FromArgsError::ResourceDoesntExist)).transpose()?;
					}
				};
				let null_check = if arg.allow_null {
					quote!()
				} else {
					quote!(let #val = #val.ok_or(FromArgsError::NullArgument)?;)
				};
				quote! {
					#create_object_opt
					#null_check
				}
			},
			ArgumentType::NewId => {
				if arg.interface.is_some() {
					quote! {
						let #val = reader.next_new_id()?.0;
						let #val = client_map.add_new_id(#val);
					}
				} else {
					quote! {
						let #val = reader.next_new_id()?.0;
						let #val = client_map.add_new_id_untyped(#val);
					}
				}
			},
			ArgumentType::Array => quote!(let #val = reader.next_array()?;),
			ArgumentType::Fd => quote!(let #val = reader.next_fd()?;),
		};
		let enum_cast = if let Some((ref ns , ref enum_type)) = arg.enum_type {
			let enum_type = Ident::new(&sanitize_enum_variant_name(&snake_to_camel(enum_type)), Span::call_site());
			if let Some(ref ns) = ns {
				let ns = Ident::new(ns, Span::call_site());
				quote!(let #val = super::#ns::#enum_type::try_from(#val)?;)
			} else {
				quote!(let #val = #enum_type::try_from(#val)?;)
			}
		} else {
			quote!()
		};
		quote! {
			#val_create
			#enum_cast
		}
	});

	let enum_name = format_ident!("{}{}", snake_to_camel(&interface.name), side.as_str());
	let variant_name = Ident::new(&snake_to_camel(&sanitize_enum_variant_name(&message.name)), Span::call_site());
	let contents = if message.arguments.is_empty() {
		quote!()
	} else {
		let fields = message.arguments.iter().enumerate().map(|(i, argument)| {
			let val = format_ident!("val{}", i);
			let field_name = Ident::new(&argument.name, Span::call_site());
			quote!(#field_name: #val)
		});
		let contents_name = format_ident!("{}{}", snake_to_camel(&message.name), side.as_str());
		quote! {
			(#contents_name {
				#(#fields,)*
			})
		}
	};
	let variant_construction = quote! {
		#enum_name::#variant_name#contents
	};

	quote! {
		#(#steps)*
		#variant_construction
	}
}

fn generate_into_args_fn(interface: &InterfaceDesc, side: MessageSide) -> TokenStream {
	let mut requests_iter = interface.requests.iter().map(|request| &request.message);
	let mut events_iter = interface.events.iter().map(|event| &event.message);
	let messages_iter: &mut dyn Iterator<Item=&MessageDesc> = match side {
		MessageSide::Request => &mut requests_iter,
		MessageSide::Event => &mut events_iter,
	};

	let match_body_arms = messages_iter.map(|message| {
		let variant_name = Ident::new(&snake_to_camel(&message.name), Span::call_site());
		let variant_contents = if message.arguments.is_empty() { quote!() } else { quote!((ref data)) };
		let message_writer = generate_message_writer(message);

		quote! {
			Self::#variant_name#variant_contents => {
				#message_writer
			}
		}
	});

	quote! {
		fn into_args(&self, client_map: Self::ClientMap) -> Result<(u16, Vec<DynArgument>), IntoArgsError> {
			let opcode = self.opcode();
			let mut args = Vec::new();
			match *self {
				#(#match_body_arms,)*
			}
			Ok((opcode, args))
		}
	}
}

fn generate_message_writer(message: &MessageDesc) -> TokenStream {
	let steps = message.arguments.iter().map(|arg| {
		let field = Ident::new(&arg.name, Span::call_site());
		match arg.arg_type {
			ArgumentType::Int => quote!(args.push(DynArgument::Int(data.#field.into()));),
			ArgumentType::Uint => quote!(args.push(DynArgument::Uint(data.#field.into()));),
			ArgumentType::Fixed => quote!(args.push(DynArgument::Fixed(data.#field));),
			ArgumentType::String => {
				if arg.allow_null {
					quote!(args.push(DynArgument::String(data.#field.clone()));)
				} else {
					quote!(args.push(DynArgument::String(Some(data.#field.clone())));)
				}
			},
			ArgumentType::Object => {
				if arg.allow_null {
					quote!(args.push(DynArgument::Object(data.#field.as_ref().map(|object| client_map.try_get_id(object.clone())).transpose()?));)
				} else {
					quote!(args.push(DynArgument::Object(Some(client_map.try_get_id(data.#field.clone())?)));)
				}
			},
			ArgumentType::NewId => {
				let interface = if arg.interface.is_none() {
					quote!(Some(title))
				} else {
					quote!(None)
				};
				quote! {
					let (id, title) = client_map.try_get_new_id(&data.#field)?;
					args.push(DynArgument::NewId(id, #interface));
				}
			},
			ArgumentType::Array => quote!(args.push(DynArgument::Array(data.#field.clone()));),
			ArgumentType::Fd => quote!(args.push(DynArgument::Fd(data.#field));),
		}
	});

	quote! {
		#(#steps)*
	}
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