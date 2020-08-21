pub mod server;
pub mod protocol;
pub mod client;
pub mod resource;
pub mod global;
pub mod object;
pub mod net;

pub use crate::{
	server::{Server},
	client::{Client},
	resource::{Resource, NewResource, Untyped},
	global::{Global},
	object::{ObjectImplementation},
};

pub use loaner::{ResourceOwner as Owner, ResourceHandle as Handle, ResourceRef as Ref};