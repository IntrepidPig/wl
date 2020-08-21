use std::{
	fmt,
	io::{Write},
};

use byteorder::{WriteBytesExt, NativeEndian};
use loaner::{ResourceHandle};

use wl_common::{
	wire::{DynMessage},
	interface::{Interface, InterfaceDebug, Message},
};

use crate::{
	client::{Client, ClientMap},
	object::{Object, ObjectImplementation, Dispatcher}, server::SendEventError,
};

// TODO: rename to WlResource to avoid confusion? or make more confusion...
#[derive(Clone)]
pub struct Resource<I> {
	client: ResourceHandle<Client>,
	object: ResourceHandle<Object>,
	interface: I,
}

#[derive(Debug, Clone)]
pub struct Untyped;

impl<I> Resource<I> {
	pub fn interface(&self) -> &I {
		&self.interface
	}

	pub fn client(&self) -> ResourceHandle<Client> {
		self.client.clone()
	}

	pub fn object(&self) -> ResourceHandle<Object> {
		self.object.clone()
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
	pub(crate) fn new(client: ResourceHandle<Client>, object: ResourceHandle<Object>) -> Self {
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
	pub(crate) fn new_untyped(client: ResourceHandle<Client>, object: ResourceHandle<Object>) -> Self {
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

pub struct NewResource<I> {
	pub(crate) inner: Resource<I>,
}

impl<I> NewResource<I> {
	pub(crate) fn new(resource: Resource<I>) -> Self {
		Self {
			inner: resource,
		}
	}
}

impl<I, R> NewResource<I> where R: Message<ClientMap=ClientMap>, I: Interface<Request=R> + 'static {
	pub fn register<Impl: ObjectImplementation<I> + 'static>(self, implementation: Impl) -> Resource<I> {
		self.inner.set_implementation(implementation);
		self.inner
	}
}

impl<I: InterfaceDebug> fmt::Debug for NewResource<I> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("NewResource")
			.field("inner", &self.inner)
			.finish()
	}
}

impl fmt::Debug for NewResource<Untyped> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("NewResource")
			.field("inner", &self.inner)
			.finish()
	}
}