use std::{
	fmt,
	cell::{RefCell},
};

use loaner::{Owner, Handle};
use thiserror::{Error};

use wl_common::{
	interface::{Interface, DynInterface},
};

use crate::{
	resource::{NewResource, Untyped},
	client::{ClientManager},
};

#[derive(Debug)]
pub(crate) struct GlobalManager {
	client_manager: Handle<RefCell<ClientManager>>,
	pub(crate) globals: Vec<Owner<Global>>,
	next_name: u32,
}

impl GlobalManager {
	pub(crate) fn new(client_manager: Handle<RefCell<ClientManager>>) -> Self {
		Self {
			client_manager,
			globals: Vec::new(),
			next_name: 1,
		}
	}

	pub(crate) fn next_name(&mut self) -> u32 {
		let name = self.next_name;
		self.next_name = self.next_name.checked_add(1).expect("Global names exhausted");
		name
	}

	// TODO: debate the return type of this
	pub fn add_global<I: Interface + 'static, Impl: GlobalImplementation<I> + 'static>(&mut self, global_implementation: Impl) -> Handle<Global> {
		let name = self.next_name();
		let global = Global::new(name, global_implementation);
		let client_manager = self.client_manager.get().expect("Client manager destroyed");
		let client_manager = client_manager.borrow_mut();
		for client in &client_manager.clients {
			client.advertise_global::<I>(name);
		}
		let owner = Owner::new(global);
		let handle = owner.handle();
		self.globals.push(owner);
		handle
	}

	pub(crate) fn bind_global(&self, name: u32, this: NewResource<Untyped>) {
		if let Some(global) = self.globals.iter().find(|global| global.name == name) {
			match global.dispatcher.borrow_mut().dispatch(this) {
				Ok(_) => {},
				Err(e) => {
					log::error!("Failed to bind global: {}", e);
				}
			}
		} else {
			log::error!("Attempted to bind global that doesn't exist");
		}
	}

	pub(crate) fn globals(&self) -> impl Iterator<Item=Handle<Global>> + '_ {
		self.globals.iter().map(|owner| owner.handle())
	}
}

#[derive(Debug)]
pub struct Global {
	pub(crate) name: u32,
	// I don't think this field is even necessary because there are no message schemas
	pub(crate) interface: DynInterface,
	pub(crate) dispatcher: RefCell<GlobalDispatcher>,
}

impl Global {
	pub fn new<I: Interface + 'static, Impl: GlobalImplementation<I> + 'static>(name: u32, global_implementation: Impl) -> Self {
		Self {
			name,
			interface: I::as_dyn(),
			dispatcher: RefCell::new(GlobalDispatcher::new(global_implementation)),
		}
	}
}

pub(crate) struct GlobalDispatcher {
	pub implementation: Box<dyn RawGlobalImplementation>,
}

impl GlobalDispatcher {
	pub fn new<I: Interface + 'static, Impl: GlobalImplementation<I> + 'static>(implementation: Impl) -> Self {
		Self {
			implementation: Box::new(RawGlobalImplementationConcrete {
				typed_implementation: Box::new(implementation),
				_phantom: std::marker::PhantomData,
			}),
		}
	}

	pub fn dispatch(&mut self, this: NewResource<Untyped>) -> Result<(), GlobalDispatchError> {
		self.implementation.dispatch(this)
	}
}

impl fmt::Debug for GlobalDispatcher {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("GlobalDispatcher")
			.field("implementation", &"opaque")
			.finish()
    }
}

pub trait GlobalImplementation<I: Interface> {
	fn handle(&mut self, this: NewResource<I>);
}

impl<I: Interface, F: FnMut(NewResource<I>)> GlobalImplementation<I> for F {
    fn handle(&mut self, this: NewResource<I>) {
        (self)(this)
    }
}

pub trait RawGlobalImplementation {
	fn dispatch(&mut self, this: NewResource<Untyped>) -> Result<(), GlobalDispatchError>;
}

pub struct RawGlobalImplementationConcrete<I: Interface> {
	typed_implementation: Box<dyn GlobalImplementation<I>>,
	_phantom: std::marker::PhantomData<I>,
}

impl<I: Interface> RawGlobalImplementation for RawGlobalImplementationConcrete<I> {
    fn dispatch(&mut self, this: NewResource<Untyped>) -> Result<(), GlobalDispatchError> {
		let typed = this.downcast::<I>().ok_or(GlobalDispatchError::TypeMismatch)?;
		self.typed_implementation.handle(typed);
		Ok(())
    }
}

#[derive(Debug, Error)]
pub enum GlobalDispatchError {
	#[error("Attempted to dispatch a request to an object with the wrong type")]
	TypeMismatch,
}