use std::{
	borrow::{Cow},
	ffi::{CStr, CString}
};

use crate::{
	wire::{RawMessage, RawMessageReader, ArgumentType, ArgumentDesc, DynArgument, DynArgumentReader, DynMessage, ArgumentError},
	resource::{ResourceManager, Resource, ClientHandle, AddObjectError},
};

use thiserror::Error;

pub type MessagesDesc = &'static [&'static [ArgumentDesc]];

pub trait Interface {
	type Request: Message;
	type Event: Message;

	const NAME: &'static str;
	const VERSION: u32;
	const REQUESTS: MessagesDesc;
	const EVENTS: MessagesDesc;

	fn new() -> Self where Self: Sized;

	// TODO Move to Ext trait?
	fn as_dyn() -> DynInterface {
		DynInterface {
			name: Cow::Borrowed(Self::NAME),
			version: Self::VERSION,
			requests: Self::REQUESTS,
			events: Self::EVENTS,
		}
	}

	fn title() -> InterfaceTitle {
		InterfaceTitle::new(Self::NAME, Self::VERSION)
	}
}

pub const ANONYMOUS_NAME: &'static str = "anonymous";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynInterface {
	pub name: Cow<'static, str>, // This is a Cow because interface names are sometimes sent over the wire, but usually not
	pub version: u32,
	pub requests: MessagesDesc,
	pub events: MessagesDesc,
}

impl DynInterface {
	// TODO: change to accept InterfaceTitle
	pub fn new<N: Into<Cow<'static, str>>>(name: N, version: u32, requests: MessagesDesc, events: MessagesDesc) -> Self {
		Self {
			name: name.into(),
			version,
			requests,
			events,
		}
	}
	
	// TODO: consider disallowing this and dealing with wl_registry.bind some other way
	pub fn new_anonymous() -> Self {
		Self {
			name: ANONYMOUS_NAME.into(),
			version: 0,
			requests: &[],
			events: &[],
		}
	}

	pub fn title(&self) -> InterfaceTitle {
		InterfaceTitle::new(self.name.clone(), self.version)
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceTitle {
	pub name: Cow<'static, str>,
	pub version: u32,
}

impl InterfaceTitle {
	pub fn new<N: Into<Cow<'static, str>>>(name: N, version: u32) -> Self {
		Self {
			name: name.into(),
			version,
		}
	}
}

pub trait Message {
	fn opcode(&self) -> u16;
	fn from_args(resources: &mut ResourceManager, client_handle: ClientHandle, opcode: u16, args: Vec<DynArgument>) -> Result<Self, FromArgsError> where Self: Sized;
	fn into_args(&self, resources: &ResourceManager, client_handle: ClientHandle) -> Result<(u16, Vec<DynArgument>), IntoArgsError>;
}

#[derive(Debug, Error)]
pub enum FromArgsError {
	#[error(transparent)]
	AddObjectError(#[from] AddObjectError),
	#[error("Unknown opcode: {0}")]
	UnknownOpcode(u16),
	#[error("A non-nullable argument was null")]
	NullArgument,
	#[error("An argument referenced a resource that does not exist")]
	ResourceDoesntExist,
	#[error(transparent)]
	ArgumentError(#[from] ArgumentError),
	#[error(transparent)]
	InvalidEnumValue(#[from] InvalidEnumValue),
	#[error("An unknown error occurred while reading the argument list: {0}")]
	Other(String)
}

#[derive(Debug, Error)]
pub enum IntoArgsError {
	#[error("This message referenced a resource that does not exist")]
	ResourceDoesntExist,
	#[error("An unknwon error occurred while converting to an argument list: {0}")]
	Other(String),
}

pub enum NoMessage { }
impl Message for NoMessage {
	fn opcode(&self) -> u16 {
		panic!("Cannot get NoMessage opcode");
	}

	fn from_args(_resources: &mut ResourceManager, _client_handle: ClientHandle, _opcode: u16, _args: Vec<DynArgument>) -> Result<Self, FromArgsError> {
		panic!("Cannot read NoMessage");
	}

	fn into_args(&self, _resources: &ResourceManager, _client_handle: ClientHandle) -> Result<(u16, Vec<DynArgument>), IntoArgsError> {
		panic!("Cannot convert NoMessage to arguments");
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Got an invalid enum value")]
pub struct InvalidEnumValue;

pub struct ProtocolRegistry {
	interfaces: Vec<DynInterface>,
}

impl ProtocolRegistry {
	pub fn new() -> Self {
		Self {
			interfaces: Vec::new(),
		}
	}

	pub fn register_protocol(&mut self, protocol: &[DynInterface]) {
		for interface in protocol {
			self.register_interface(interface.clone())
		}
	}

	pub fn register_interface(&mut self, interface: DynInterface) {
		self.interfaces.push(interface);
	}

	pub fn find_interface(&self, title: InterfaceTitle) -> Option<DynInterface> {
		self.interfaces.iter().find(|interface| interface.name == title.name && interface.version == title.version).cloned()
	}
}