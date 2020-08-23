use std::{
	fmt,
	marker::PhantomData,
};

use loaner::{Owner, Handle, Ref};

use wl_common::{
	interface::{Interface, InterfaceDebug, Message},
};

use crate::{
	server::{State},
	client::{Client, ClientMap},
	object::{Object, ObjectImplementation, Dispatcher}, server::SendEventError,
};

// TODO: rename to WlResource to avoid confusion? or make more confusion...
#[derive(Clone)]
pub struct Resource<I> {
	client: Handle<Client>,
	object: Handle<Object>,
	interface: I,
}

#[derive(Debug, Clone)]
pub struct Untyped;

impl<I> Resource<I> {
	pub fn interface(&self) -> &I {
		&self.interface
	}

	pub fn client(&self) -> Handle<Client> {
		self.client.clone()
	}

	pub fn object(&self) -> Handle<Object> {
		self.object.clone()
	}

	pub fn is(&self, other: &Resource<I>) -> bool {
		self.object.is(&other.object)
	}

	pub fn destroy(&self) {
		if let Some(object) = self.object.get() {
			object.destroy.set(true);
		}
	}

	pub fn get_data<'a, T: 'static>(&'a self) -> Option<Ref<'a, T>> {
		self.object.get()?.data.borrow().downcast_ref::<Owner<T>>().map(|owner| owner.custom_ref())
	}

	pub fn with<T, F: FnOnce(Ref<Object>) -> T>(&self, f: F) -> Option<T> {
		self.object.get().map(f)
	}

	pub fn to_untyped(&self) -> Resource<Untyped> {
		Resource {
			client: self.client.clone(),
			object: self.object.clone(),
			interface: Untyped,
		}
	}
}

impl<I: Interface> Resource<I> {
	pub(crate) fn new(client: Handle<Client>, object: Handle<Object>) -> Self {
		Self {
			client,
			object,
			interface: I::new(),
		}
	}
}

impl<I: Interface + 'static> Resource<I> where I::Request: Message<ClientMap=ClientMap> + fmt::Debug {
	pub fn set_implementation<Impl: ObjectImplementation<I> + 'static>(&self, implementation: Impl) {
		let dispatcher = Dispatcher::new(implementation);
		if let Some(object) = self.object.get() {
			*object.dispatcher.borrow_mut() = Some(dispatcher);
		};
	}
}

impl<I: Interface> Resource<I> where I::Event: Message<ClientMap=ClientMap> + fmt::Debug {
	pub fn send_event(&self, event: I::Event) {
		match self.try_send_event(event) {
			Ok(_) => {},
			Err(e) => {
				log::error!("Sending event failed: {}", e);
			}
		}
	}

	pub fn try_send_event(&self, event: I::Event) -> Result<(), SendEventError> {
		// TODO: control with WAYLAND_DEBUG, as well is for requests received.
		/* log::trace!(
			" -> {interface_name}.{interface_version}@{object_id} {event:?}",
			interface_name=object.interface.name,
			interface_version=object.interface.version,
			object_id=object.id,
			event=event
		); */
		
		let client = self.client.get().ok_or(SendEventError::ClientMissing)?;
		client.try_send_event::<I>(self.object.clone(), event)?;

		Ok(())
	}
}

impl Resource<Untyped> {
	pub(crate) fn new_untyped(client: Handle<Client>, object: Handle<Object>) -> Self {
		Resource {
			client,
			object,
			interface: Untyped,
		}
	}

	pub fn downcast<I: Interface>(&self) -> Option<Resource<I>> {
		let object = self.object.get()?;
		// TODO: version/subset checking too?
		if I::as_dyn() == object.interface.get() {
			Some(self.downcast_unchecked())
		} else {
			None
		}
	}
	
	fn downcast_unchecked<I: Interface>(&self) -> Resource<I> {
		Resource {
			client: self.client.clone(),
			object: self.object.clone(),
			interface: I::new(),
		}
	}
}

impl<I: InterfaceDebug> fmt::Debug for Resource<I> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Resource<{}:{}>@{}", self.interface.name(), self.interface.version(), if let Some(object) = self.object.get() {
			format!("{}", object.id)
		} else {
			format!("<dead>")
		})
	}
}

/* impl fmt::Debug for Resource<DynInterface> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Resource<(dyn){}:{}>@({:?}.{:?})", self.interface.name, self.interface.version, self.client.0, self.object.0)
	}
} */

impl fmt::Debug for Resource<Untyped> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Resource<(untyped)>@{}", if let Some(object) = self.object.get() {
			format!("{}", object.id)
		} else {
			format!("<dead>")
		})
	}
}

// This is close to a resource owner. Exactly one gets created for every protocol object, unlike `Resource`.
#[derive(Debug)]
pub struct NewResource<I> {
	pub(crate) client: Handle<Client>,
	pub(crate) object: Handle<Object>,
	_phantom: PhantomData<I>,
}

impl<I> NewResource<I> {
	pub(crate) fn new(client: Handle<Client>, object: Handle<Object>) -> Self {
		Self {
			client,
			object,
			_phantom: PhantomData,
		}
	}
}

impl NewResource<Untyped> {
	pub(crate) fn downcast<I: Interface>(self) -> Option<NewResource<I>> {
		let object = self.object.get()?;
		// TODO: version/subset checking too?
		if I::as_dyn() == object.interface.get() {
			Some(NewResource {
				client: self.client,
				object: self.object,
				_phantom: PhantomData,
			})
		} else {
			None
		}
	}
}

impl<I: Interface + 'static> NewResource<I> where I::Request: Message<ClientMap=ClientMap> + fmt::Debug {
	pub fn register<Impl: ObjectImplementation<I> + 'static, T: 'static>(self, data: T, implementation: Impl) -> Resource<I> {
		if let Some(object) = self.object.get() {
			let dispatcher = Dispatcher::new(implementation);
			*object.dispatcher.borrow_mut() = Some(dispatcher);
			object.set_data(data);
		}
		Resource::new(self.client, self.object)
	}

	pub fn register_fn<T: 'static, F, D>(self, data: T, handler: F, destructor: D) -> Resource<I> where F: FnMut(&mut State, Resource<I>, I::Request) + 'static, D: FnMut(&mut State, Resource<I>) + 'static {
		let implementation = ObjectImplementationFn {
			handler,
			destructor,
			_phantom: PhantomData,
		};
		self.register(data, implementation)
	}
}

struct ObjectImplementationFn<I: Interface, F, D> where F: FnMut(&mut State, Resource<I>, I::Request) + 'static, D: FnMut(&mut State, Resource<I>) + 'static {
	handler: F,
	destructor: D,
	_phantom: PhantomData<I>,
}

impl<I: Interface, F, D> ObjectImplementation<I> for ObjectImplementationFn<I, F, D> where F: FnMut(&mut State, Resource<I>, I::Request) + 'static, D: FnMut(&mut State, Resource<I>) + 'static {
	fn handle(&mut self, state: &mut State, this: Resource<I>, request: I::Request) {
        (self.handler)(state, this, request)
	}
	
	fn handle_destructor(&mut self, state: &mut State, this: Resource<I>) {
        (self.destructor)(state, this)
    }
}
