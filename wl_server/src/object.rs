use std::{
	any::{Any},
	cell::{Cell, RefCell},
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
	client::{ClientMap},
	resource::{Resource, Untyped},
};

#[derive(Debug)]
pub struct ObjectMap {
	pub(crate) objects: Vec<Owner<Object>>,
}

impl ObjectMap {
	pub(crate) fn new() -> Self {
		Self {
			objects: Vec::new(),
		}
	}

	pub fn add(&mut self, object: Owner<Object>) {
		self.objects.push(object);
	}

	pub fn remove(&mut self, handle: Handle<Object>) -> Option<Owner<Object>> {
		if let Some(i) = self.objects.iter().position(|object| object.handle().is(&handle)) {
			Some(self.objects.remove(i))
		} else {
			None
		}
	}

	pub fn remove_any(&mut self) -> Option<Owner<Object>> {
		self.objects.pop()
	}

	pub fn find<F: Fn(&Owner<Object>) -> bool>(&self, f: F) -> Option<Ref<Object>> {
		self.objects.iter().find_map(|object| {
			if f(object) {
				Some(object.custom_ref())
			} else {
				None
			}
		})
	}
}

#[derive(Debug)]
pub struct Object {
	pub(crate) id: u32,
	pub(crate) interface: Cell<DynInterface>,
	pub(crate) dispatcher: RefCell<Option<Dispatcher>>, 
	pub(crate) data: RefCell<Box<dyn Any>>,
	pub(crate) destroy: Cell<bool>,
}

impl Object {
	pub fn new<I, R>(id: u32) -> Self where R: Message<ClientMap=ClientMap> + fmt::Debug, I: Interface<Request=R> + fmt::Debug + 'static {
		Self {
			id,
			interface: Cell::new(I::as_dyn()),
			dispatcher: RefCell::new(Some(Dispatcher::null::<I, R>())),
			data: RefCell::new(Box::new(())),
			destroy: Cell::new(false),
		}
	}

	// This is dangerous because if any request or event is sent to this object before it leaves it's untyped state, errors will happen
	pub fn new_untyped(id: u32) -> Self {
		Self {
			id,
			interface: Cell::new(DynInterface::new_anonymous()),
			dispatcher: RefCell::new(None),
			data: RefCell::new(Box::new(())),
			destroy: Cell::new(false),
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

impl Drop for Object {
	fn drop(&mut self) {
		if let Some(ref dispatcher) = &*self.dispatcher.borrow() {
			if !dispatcher.destroyed {
				log::warn!("Object {} was dropped without running its destructor; Resource leaks may occur", self.id);
			}
		}
	}
}

pub(crate) struct Dispatcher {
	pub implementation: Box<dyn RawObjectImplementation>,
	pub destroyed: bool,
}

impl Dispatcher {
	pub fn new<I: Interface + 'static, T: ObjectImplementation<I> + 'static>(implementation: T) -> Self where I::Request: Message<ClientMap=ClientMap> + fmt::Debug {
		let raw_obj_implementation: Box<dyn RawObjectImplementation> = Box::new(RawObjectImplementationConcrete::<I> {
			_phantom: std::marker::PhantomData,
			typed_implementation: Box::new(implementation),
		});
		Self {
			implementation: raw_obj_implementation,
			destroyed: false,
		}
	}

	pub fn null<I, R>() -> Self where R: Message<ClientMap=ClientMap> + fmt::Debug, I: Interface<Request=R> + 'static {
		#[derive(Debug)]
		struct NullImpl;

		impl<M, I: Interface<Request=M>> ObjectImplementation<I> for NullImpl where M: Message + fmt::Debug {
			fn handle(&mut self, _state: &mut State, this: Resource<I>, request: I::Request) {
				log::debug!("Got unhandled request for {:?}: {:?}", this, request);
			}

			fn handle_destructor(&mut self, _state: &mut State, this: Resource<I>) {
				log::debug!("Got unhandled destructor ron for {:?}", this);
			}
		}

		let implementation = Box::new(RawObjectImplementationConcrete::<I> {
			_phantom: std::marker::PhantomData,
			typed_implementation: Box::new(NullImpl),
		});
		
		Self {
			implementation,
			destroyed: false,
		}
	}

	pub fn dispatch(&mut self, state: &mut State, this: Resource<Untyped>, opcode: u16, args: Vec<DynArgument>) -> Result<(), DispatchError> {
		if self.destroyed {
			return Err(DispatchError::ObjectDestroyed)
		}
		self.implementation.dispatch(state, this, opcode, args)
	}

	pub fn dispatch_destructor(&mut self, state: &mut State, this: Resource<Untyped>) -> Result<(), DispatchError> {
		if self.destroyed {
			return Err(DispatchError::ObjectDestroyed)
		}
		let result = self.implementation.dispatch_destructor(state, this);
		self.destroyed = true;
		result
	}
}

impl fmt::Debug for Dispatcher {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("Dispatcher")
			.field("implementation", &"<opaque>")
			.finish()
	}
}

// TODO: consider passing associated object data in a typed manner to the handler here. Would be nice...
pub trait ObjectImplementation<I: Interface> {
	fn handle(&mut self, state: &mut State, this: Resource<I>, request: I::Request);

	fn handle_destructor(&mut self, state: &mut State, this: Resource<I>);
}

pub trait RawObjectImplementation {
	fn dispatch(&mut self, state: &mut State, this: Resource<Untyped>, opcode: u16, args: Vec<DynArgument>) -> Result<(), DispatchError>;

	fn dispatch_destructor(&mut self, state: &mut State, this: Resource<Untyped>) -> Result<(), DispatchError>;
}

pub struct RawObjectImplementationConcrete<I> {
	_phantom: std::marker::PhantomData<I>,
	typed_implementation: Box<dyn ObjectImplementation<I>>,
}

impl<I: Interface> RawObjectImplementation for RawObjectImplementationConcrete<I> where I::Request: Message<ClientMap=ClientMap> + fmt::Debug {
	fn dispatch(&mut self, state: &mut State, this: Resource<Untyped>, opcode: u16, args: Vec<DynArgument>) -> Result<(), DispatchError> {
		let typed_resource = this.downcast::<I>().ok_or(DispatchError::TypeMismatch)?;
		let client_map = this.client().get().unwrap().client_map();
		let request = I::Request::from_args(client_map, opcode, args)?;
		self.typed_implementation.handle(state, typed_resource, request);
		Ok(())
	}

	fn dispatch_destructor(&mut self, state: &mut State, this: Resource<Untyped>) -> Result<(), DispatchError> {
		let typed_resource = this.downcast::<I>().ok_or(DispatchError::TypeMismatch)?;
		self.typed_implementation.handle_destructor(state, typed_resource);
		Ok(())
	}
}

#[derive(Debug, Error)]
pub enum DispatchError {
	#[error("Attempted to dispatch a request to an object with the wrong type")]
	TypeMismatch,
	#[error("Attempted to dispatch to an object that was destroyed")]
	ObjectDestroyed,
	#[error(transparent)]
	ArgumentError(#[from] FromArgsError),
}