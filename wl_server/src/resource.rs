use std::{
	fmt,
	marker::PhantomData,
};

use loaner::{Owner, Handle, Ref};

use wl_common::{
	interface::{Interface, Message},
};

use crate::{
	server::{State},
	client::{Client, ClientMap},
	object::{Object, ObjectImplementation, Dispatcher}, server::SendEventError,
};

#[derive(Debug)]
pub enum Anonymous {}

#[derive(Debug)]
pub enum Untyped {}

// TODO: rename to WlResource to avoid confusion? or make more confusion...
pub struct Resource<I, T> {
	client: Handle<Client>,
	object: Handle<Object>,
	_phantom: PhantomData<(I, T)>,
}

impl<I, T> Resource<I, T> {
	pub fn client(&self) -> Handle<Client> {
		self.client.clone()
	}

	pub fn object(&self) -> Handle<Object> {
		self.object.clone()
	}

	pub fn is(&self, other: &Resource<I, T>) -> bool {
		self.object.is(&other.object)
	}

	pub fn destroy(&self) {
		if let Some(object) = self.object.get() {
			object.destroy.set(true);
		}
	}

	pub fn to_anonymous(&self) -> Resource<Anonymous, T> {
		Resource {
			client: self.client.clone(),
			object: self.object.clone(),
			_phantom: PhantomData,
		}
	}

	pub fn to_untyped(&self) -> Resource<I, Untyped> {
		Resource {
			client: self.client.clone(),
			object: self.object.clone(),
			_phantom: PhantomData,
		}
	}

	fn downcast_unchecked<I2, T2>(&self) -> Resource<I2, T2> {
		Resource {
			client: self.client.clone(),
			object: self.object.clone(),
			_phantom: PhantomData,
		}
	}
}

impl<I, T: 'static> Resource<I, T> {
	pub fn try_get_data(&self) -> Option<Ref<T>> {
		self.object.get()?.data.borrow().downcast_ref::<Owner<T>>().map(|owner| owner.custom_ref())
	}

	pub fn get_data(&self) -> Ref<T> {
		self.object.get().expect("Object was destroyed").data.borrow().downcast_ref::<Owner<T>>().map(|owner| owner.custom_ref()).expect("Data type mismatch")
	}

	pub fn with<U, F: FnOnce(Ref<Object>, Ref<T>) -> U>(&self, f: F) -> Option<U> {
		self.object.get()
			.and_then(|object| {
				let data = object.data.borrow();
				Some((object.clone(), data.downcast_ref::<Owner<T>>().map(|owner| owner.custom_ref())?))
			})
			.map(|(object, data)| f(object, data))
	}
}

impl<T> Resource<Anonymous, T> {
	pub(crate) fn new_anonymous(client: Handle<Client>, object: Handle<Object>) -> Self {
		Resource {
			client,
			object,
			_phantom: PhantomData,
		}
	}

	pub fn downcast_interface<I: Interface>(&self) -> Option<Resource<I, T>> {
		let object = self.object.get()?;
		// TODO: version/subset checking too?
		if I::as_dyn() == object.interface.get() {
			Some(self.downcast_unchecked())
		} else {
			None
		}
	}
}

impl<I> Resource<I, Untyped> {
	pub fn downcast_data<T: 'static>(&self) -> Option<Resource<I, T>> {
		let object = self.object.get()?;
		if object.get_data::<T>().is_some() {
			Some(self.downcast_unchecked())
		} else {
			None
		}
	}
}

impl Resource<Anonymous, Untyped> {
	pub fn downcast_both<I: Interface, T: 'static>(&self) -> Option<Resource<I, T>> {
		self.downcast_interface()?.downcast_data()
	}
}

impl<I: Interface, T> Resource<I, T> {
	pub(crate) fn new(client: Handle<Client>, object: Handle<Object>) -> Self {
		Self {
			client,
			object,
			_phantom: PhantomData,
		}
	}
}

impl<I: Interface + 'static, T: 'static> Resource<I, T> where I::Request: Message<ClientMap=ClientMap> + fmt::Debug {
	pub(crate) fn set_implementation<Impl: ObjectImplementation<I, T> + 'static>(&self, implementation: Impl) {
		let dispatcher = Dispatcher::new(implementation);
		if let Some(object) = self.object.get() {
			*object.dispatcher.borrow_mut() = Some(dispatcher);
		};
	}
}

impl<I: Interface, T> Resource<I, T> where I::Event: Message<ClientMap=ClientMap> + fmt::Debug {
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

impl<I, T> fmt::Debug for Resource<I, T> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match (self.client.get(), self.object.get()) {
			(Some(client), Some(object)) => {
				let interface = object.interface.get();
				write!(f, "Resource@{}({}@{})", client.id(), interface.name, object.id)
			},
			(None, Some(object)) => {
				let interface = object.interface.get();
				write!(f, "Resource@<dead>({}@{})", interface.name, object.id)
			},
			(Some(client), None) => {
				write!(f, "Resource@{}(<dead>)", client.id())
			},
			(None, None) => {
				write!(f, "Resource@<dead>(<dead>)")
			}
		}
	}
}

impl<I, T> Clone for Resource<I, T> {
	fn clone(&self) -> Self {
		Resource {
			client: self.client.clone(),
			object: self.object.clone(),
			_phantom: PhantomData,
		}
	}
}

// This is close to a resource owner. Exactly one gets created for every protocol object, unlike `Resource`.
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

impl NewResource<Anonymous> {
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
	pub fn register<Impl: ObjectImplementation<I, T> + 'static, T: 'static>(self, data: T, implementation: Impl) -> Resource<I, T> {
		if let Some(object) = self.object.get() {
			let dispatcher = Dispatcher::new(implementation);
			*object.dispatcher.borrow_mut() = Some(dispatcher);
			object.set_data(data);
		}
		Resource::new(self.client, self.object)
	}

	pub fn register_fn<T: 'static, F, D>(self, data: T, handler: F, destructor: D) -> Resource<I, T> where F: FnMut(&mut State, Resource<I, T>, I::Request) + 'static, D: FnMut(&mut State, Resource<I, T>) + 'static {
		let implementation = ObjectImplementationFn {
			handler,
			destructor,
			_phantom: PhantomData,
		};
		self.register(data, implementation)
	}
}

impl<I> fmt::Debug for NewResource<I> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match (self.client.get(), self.object.get()) {
			(Some(client), Some(object)) => {
				let interface = object.interface.get();
				write!(f, "NewResource@{}({}@{})", client.id(), interface.name, object.id)
			},
			(None, Some(object)) => {
				let interface = object.interface.get();
				write!(f, "NewResource@<dead>({}@{})", interface.name, object.id)
			},
			(Some(client), None) => {
				write!(f, "NewResource@{}(<dead>)", client.id())
			},
			(None, None) => {
				write!(f, "NewResource@<dead>(<dead>)")
			}
		}
	}
}

struct ObjectImplementationFn<I: Interface, T, F, D> where F: FnMut(&mut State, Resource<I, T>, I::Request) + 'static, D: FnMut(&mut State, Resource<I, T>) + 'static {
	handler: F,
	destructor: D,
	_phantom: PhantomData<(I, T)>,
}

impl<I: Interface, T, F, D> ObjectImplementation<I, T> for ObjectImplementationFn<I, T, F, D> where F: FnMut(&mut State, Resource<I, T>, I::Request) + 'static, D: FnMut(&mut State, Resource<I, T>) + 'static {
	fn handle(&mut self, state: &mut State, this: Resource<I, T>, request: I::Request) {
        (self.handler)(state, this, request)
	}
	
	fn handle_destructor(&mut self, state: &mut State, this: Resource<I, T>) {
        (self.destructor)(state, this)
    }
}
