use std::{
	any::{Any},
	cell::{RefCell},
	fmt,
};

use thiserror::{Error};

use loaner::{
	Owner, Handle, Ref,
};

use wl_common::{
	interface::{Interface, DynInterface, Message, FromArgsError},
	wire::{DynArgument},
};

use crate::{
	server::{State},
	client::{Client, ClientMap},
	resource::{Resource, Untyped},
};

#[derive(Debug)]
pub struct ObjectMap {
	client: Option<Handle<Client>>, // TODO: ensure necessary
	objects: Vec<Owner<Object>>,
}

impl ObjectMap {
	pub(crate) fn new() -> Self {
		Self {
			client: None,
			objects: Vec::new(),
		}
	}

	pub fn add(&mut self, object: Owner<Object>) {
		self.objects.push(object);
	}

	pub fn find<F: Fn(&Owner<Object>) -> bool>(&self, f: F) -> Option<Handle<Object>> {
		self.objects.iter().find_map(|object| {
			if f(object) {
				Some(object.handle())
			} else {
				None
			}
		})
	}
}

#[derive(Debug)]
pub struct Object {
	pub(crate) id: u32,
	pub(crate) interface: DynInterface,
	pub(crate) dispatcher: RefCell<Option<Dispatcher>>, 
	pub(crate) data: RefCell<Box<dyn Any>>,
}

impl Object {
	pub fn new<I, R>(id: u32) -> Self where R: Message<ClientMap=ClientMap> + fmt::Debug, I: Interface<Request=R> + fmt::Debug + 'static {
		Self {
			id,
			interface: I::as_dyn(),
			dispatcher: RefCell::new(Some(Dispatcher::null::<I, R>())),
			data: RefCell::new(Box::new(())),
		}
	}

	// This is dangerous because if any request or event is sent to this object before it leaves it's untyped state, errors will happen
	pub fn new_untyped(id: u32) -> Self {
		Self {
			id,
			interface: DynInterface::new_anonymous(),
			dispatcher: RefCell::new(None),
			data: RefCell::new(Box::new(())),
		}
	}

	pub fn set_data<T: 'static>(&self, data: T) -> Handle<T> {
		let owner = Owner::new(data);
		let handle = owner.handle();
		*self.data.borrow_mut() = Box::new(owner);
		handle
	}

	pub fn get_data<'a, T: 'static>(&'a self) -> Option<Ref<'a, T>> {
		self.data.borrow().downcast_ref::<Owner<T>>().map(|owner| owner.custom_ref())
	}
}

pub(crate) struct Dispatcher {
	pub implementation: Box<dyn RawObjectImplementation>,
}

impl Dispatcher {
	pub fn new<I, R, T>(implementation: T) -> Self where R: Message<ClientMap=ClientMap>, I: Interface<Request=R> + 'static, T: ObjectImplementation<I> + 'static {
		let raw_obj_implementation: Box<dyn RawObjectImplementation> = Box::new(RawObjectImplementationConcrete::<I> {
			_phantom: std::marker::PhantomData,
			typed_implementation: Box::new(implementation),
		});
		Self {
			implementation: raw_obj_implementation,
		}
	}

	pub fn null<I, R>() -> Self where R: Message<ClientMap=ClientMap> + fmt::Debug, I: Interface<Request=R> + 'static {
		#[derive(Debug)]
		struct NullImpl;

		impl<M, I: Interface<Request=M>> ObjectImplementation<I> for NullImpl where M: Message + fmt::Debug {
			fn handle(&mut self, _state: &mut State, this: Resource<I>, request: I::Request) {
				log::debug!("Got unhandled request for {:?}: {:?}", this, request);
			}
		}

		let implementation = Box::new(RawObjectImplementationConcrete::<I> {
			_phantom: std::marker::PhantomData,
			typed_implementation: Box::new(NullImpl),
		});
		
		Self {
			implementation,
		}
	}

	pub fn dispatch(&mut self, state: &mut State, this: Resource<Untyped>, opcode: u16, args: Vec<DynArgument>) -> Result<(), DispatchError> {
		self.implementation.dispatch(state, this, opcode, args)
	}
}

impl fmt::Debug for Dispatcher {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("Dispatcher")
			.field("implementation", &"<opaque>")
			.finish()
	}
}

pub trait ObjectImplementation<I: Interface> {
	fn handle(&mut self, state: &mut State, this: Resource<I>, request: I::Request);
}

impl<I: Interface, F: FnMut(&mut State, Resource<I>, I::Request)> ObjectImplementation<I> for F {
	fn handle(&mut self, state: &mut State, this: Resource<I>, request: I::Request) {
        (self)(state, this, request)
    }
}


pub trait RawObjectImplementation {
	fn dispatch(&mut self, state: &mut State, this: Resource<Untyped>, opcode: u16, args: Vec<DynArgument>) -> Result<(), DispatchError>;
}

pub struct RawObjectImplementationConcrete<I> {
	_phantom: std::marker::PhantomData<I>,
	typed_implementation: Box<dyn ObjectImplementation<I>>,
}

impl<R, I> RawObjectImplementation for RawObjectImplementationConcrete<I> where R: Message<ClientMap=ClientMap>, I: Interface<Request=R> {
	fn dispatch(&mut self, state: &mut State, this: Resource<Untyped>, opcode: u16, args: Vec<DynArgument>) -> Result<(), DispatchError> {
		let typed_resource = this.downcast::<I>().ok_or(DispatchError::TypeMismatch)?;
		let client_map = this.client().get().unwrap().client_map();
		let request = I::Request::from_args(client_map, opcode, args)?;
		self.typed_implementation.handle(state, typed_resource, request);
		Ok(())
	}
}

#[derive(Debug, Error)]
pub enum DispatchError {
	#[error("Attempted to dispatch a request to an object with the wrong type")]
	TypeMismatch,
	#[error(transparent)]
	ArgumentError(#[from] FromArgsError),
}