use std::{
	borrow::{Cow},
};

use crate::{
	wire::{ArgumentDesc, DynArgument, ArgumentError},
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

	fn as_dyn() -> DynInterface {
		DynInterface {
			name: Self::NAME,
			version: Self::VERSION,
			requests: Self::REQUESTS,
			events: Self::EVENTS,
		}
	}

	fn title() -> InterfaceTitle {
		InterfaceTitle::new(Self::NAME, Self::VERSION)
	}
}

pub trait InterfaceDebug {
	fn name(&self) -> &str;
	fn version(&self) -> u32;
}

impl<I: Interface> InterfaceDebug for I {
    fn name(&self) -> &str {
        I::NAME
    }
    fn version(&self) -> u32 {
        I::VERSION
    }
	
}

pub const ANONYMOUS_NAME: &'static str = "anonymous";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynInterface {
	pub name: &'static str,
	pub version: u32,
	pub requests: MessagesDesc,
	pub events: MessagesDesc,
}

impl InterfaceDebug for DynInterface {
    fn name(&self) -> &str {
        self.name.as_ref()
    }
    fn version(&self) -> u32 {
        self.version
    }
}

impl DynInterface {
	// TODO: change to accept InterfaceTitle
	pub fn new(name: &'static str, version: u32, requests: MessagesDesc, events: MessagesDesc) -> Self {
		Self {
			name,
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

#[derive(Debug, Error)]
pub enum AddObjectError {
	#[error("Tried to add an object to a client that doesn't exist")]
	ClientDoesntExist,
	#[error("Tried to add an object to a client but the id was already taken")]
	IdAlreadyTaken,
	#[error("Another object with the same already already exists with a different interface")]
	InterfaceMismatch,
}

pub trait Message {
	type ClientMap;

	fn opcode(&self) -> u16;

	fn from_args(client_map: Self::ClientMap, opcode: u16, args: Vec<DynArgument>) -> Result<Self, FromArgsError> where Self: Sized;

	fn into_args(&self, client_map: Self::ClientMap) -> Result<(u16, Vec<DynArgument>), IntoArgsError>;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Got an invalid enum value")]
pub struct InvalidEnumValue;
