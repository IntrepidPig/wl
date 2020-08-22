pub mod server;
pub mod protocol;
pub mod client;
pub mod resource;
pub mod global;
pub mod object;
pub mod net;
pub use loaner;

pub use crate::{
	server::{Server},
	client::{Client},
	resource::{Resource, NewResource, Untyped},
	global::{Global},
	object::{ObjectImplementation},
	loaner::{Owner, Handle},
};

// TODO: implement custom versions