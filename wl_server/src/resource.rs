use std::{
	fmt,
	io::{Write}, marker::PhantomData,
};

use byteorder::{WriteBytesExt, NativeEndian};
use loaner::{Handle};

use wl_common::{
	wire::{DynMessage},
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

	// TODO: returning a handle is not ideal because it does not convey that there is
	// guaranteed to be a resource behind the handle, and an unwrap will usually follow.
	pub fn get_data<T: 'static>(&self) -> Option<Handle<T>> {
		self.object.get()?.get_data()
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

impl<I, R> Resource<I> where R: Message<ClientMap=ClientMap>, I: Interface<Request=R> + 'static {
	pub fn set_implementation<Impl: ObjectImplementation<I> + 'static>(&self, implementation: Impl) {
		let dispatcher = Dispatcher::new(implementation);
		if let Some(object) = self.object.get() {
			*object.dispatcher.borrow_mut() = Some(dispatcher);
		};
	}
}

impl<I, E> Resource<I> where E: Message<ClientMap=ClientMap> + fmt::Debug, I: Interface<Event=E> {
	pub fn send_event(&self, event: I::Event) {
		match self.try_send_event(event) {
			Ok(_) => {},
			Err(e) => {
				log::error!("Sending event failed: {}", e);
			}
		}
	}

	pub fn try_send_event(&self, event: I::Event) -> Result<(), SendEventError> {
		// TODO: move logic to net module

		let client = self.client.get().ok_or(SendEventError::ClientMissing)?;
		let object = self.object.get().ok_or(SendEventError::SenderMissing)?;

		let client_map = client.client_map();
		let args = event.into_args(client_map)?;

		let dyn_msg = DynMessage::new(object.id, args.0, args.1);
		let raw = dyn_msg.into_raw()?;
		let mut data = Vec::with_capacity(raw.header.msg_size as usize);
		data.write_u32::<NativeEndian>(raw.header.sender).unwrap();
		data.write_u16::<NativeEndian>(raw.header.opcode).unwrap();
		data.write_u16::<NativeEndian>(raw.header.msg_size).unwrap();
		data.extend_from_slice(&raw.data);

		// TODO: control with WAYLAND_DEBUG, as well is for requests received.
		/* log::trace!(
			" -> {interface_name}.{interface_version}@{object_id} {event:?}",
			interface_name=object.interface.name,
			interface_version=object.interface.version,
			object_id=object.id,
			event=event
		); */

		client.stream.borrow_mut().write_all(&data)?;

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
		if I::as_dyn() == object.interface {
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
		if I::as_dyn() == object.interface {
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

impl<I, R> NewResource<I> where R: Message<ClientMap=ClientMap>, I: Interface<Request=R> + 'static {
	pub fn register<Impl: ObjectImplementation<I> + 'static, T: 'static>(self, data: T, implementation: Impl) -> Resource<I> {
		if let Some(object) = self.object.get() {
			let dispatcher = Dispatcher::new(implementation);
			*object.dispatcher.borrow_mut() = Some(dispatcher);
			object.set_data(data);
		}
		Resource::new(self.client, self.object)
	}

	pub fn register_fn<T: 'static, F: FnMut(&mut State, Resource<I>, I::Request) + 'static>(self, data: T, f: F) -> Resource<I> {
		let implementation = ObjectImplementationFn {
			f,
			_phantom: PhantomData,
		};
		self.register(data, implementation)
	}
}

struct ObjectImplementationFn<I: Interface, F: FnMut(&mut State, Resource<I>, I::Request)> {
	f: F,
	_phantom: PhantomData<I>,
}

impl<I: Interface, F: FnMut(&mut State, Resource<I>, I::Request)> ObjectImplementation<I> for ObjectImplementationFn<I, F> {
	fn handle(&mut self, state: &mut State, this: Resource<I>, request: I::Request) {
        (self.f)(state, this, request)
    }
}
