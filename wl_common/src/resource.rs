use std::{
	borrow::{Cow},
	fmt,
};

use graph_storage::{GraphStorage, Key};
use thiserror::Error;

use crate::{
	wire::{ArgumentDesc, ArgumentType},
	protocol::{self, Interface, DynInterface, MessagesDesc, InterfaceTitle},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientHandle(Key);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectHandle(Key);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalHandle(Key);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenerHandle(Key);

#[derive(Debug)]
pub struct Client {
	objects: GraphStorage<ObjectInfo>,
}

impl Client {
	pub fn new() -> Self {
		let mut objects = GraphStorage::new();
		objects.add(ObjectInfo {
		    id: 1,
		    interface: DynInterface::new(
				"wl_display",
				1,
				&[
            &[ArgumentDesc {
                arg_type: ArgumentType::NewId,
                interface: Some("wl_callback"),
                allow_null: false,
            }],
            &[ArgumentDesc {
                arg_type: ArgumentType::NewId,
                interface: Some("wl_registry"),
                allow_null: false,
            }],
        ],
		&[
            &[
                ArgumentDesc {
                    arg_type: ArgumentType::Object,
                    interface: None,
                    allow_null: false,
                },
                ArgumentDesc {
                    arg_type: ArgumentType::Uint,
                    interface: None,
                    allow_null: false,
                },
                ArgumentDesc {
                    arg_type: ArgumentType::String,
                    interface: None,
                    allow_null: false,
                },
            ],
            &[ArgumentDesc {
                arg_type: ArgumentType::Uint,
                interface: None,
                allow_null: false,
            }],
        ],
			), // This is hardcoded because wl_common cannot depend on the generated protocol
		});
		Self {
			objects,
		}
	}

	pub fn find_object_handle<F: FnMut(&ObjectInfo) -> bool>(&self, predicate: F) -> Option<ObjectHandle> {
		self.objects.find_key(predicate).map(ObjectHandle)
	}
}

#[derive(Debug, Clone)]
pub struct ObjectInfo {
	pub id: u32,
	pub interface: DynInterface,
}

#[derive(Debug, Clone)]
pub struct GlobalInfo {
	pub name: u32,
	pub interface: DynInterface,
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

#[derive(Debug)]
pub struct ResourceManager {
	pub clients: GraphStorage<Client>,
	pub globals: GraphStorage<GlobalInfo>,
	pub next_global_name: u32,
}

impl ResourceManager {
	pub fn new() -> Self {
		Self {
			clients: GraphStorage::new(),
			globals: GraphStorage::new(),
			next_global_name: 1,
		}
	}
	
	pub fn set_resource_interface<I: Interface>(&mut self, resource: &Resource<Untyped>) -> Option<Resource<I>> {
		if let Some(object_info) = self.get_object_info_untyped_mut(&resource) {
			object_info.interface = I::as_dyn();
			Some(resource.downcast_unchecked())
		} else {
			None
		}
	}

	pub fn set_resource_interface_untyped(&mut self, resource: &Resource<Untyped>, interface: DynInterface) {
		log::debug!("Setting interface of resource {:?} to {:?}", resource, interface);
		if let Some(object_info) = self.get_object_info_untyped_mut(&resource) {
			object_info.interface = interface;
		} else {
			log::warn!("Failed to set resource interface");
		}
		dbg!(&self);
	}

	pub fn update_resource_interface_to<I: Interface>(&mut self, resource: &Resource<I>) {
		log::debug!("Updating interface for {:?}", resource);
		if let Some(object_info) = self.get_object_info_untyped_mut(&resource.to_untyped()) {
			object_info.interface = I::as_dyn();
		} else {
			log::warn!("Failed to update the interface of a resource that doesn't exist ({:?})", resource);
		}
	}

	pub fn add_client(&mut self, client: Client) -> ClientHandle {
		ClientHandle(self.clients.add(client))
	}

	pub fn get_client(&self, client_handle: ClientHandle) -> Option<&Client> {
		self.clients.get(client_handle.0)
	}

 	/*pub fn insert_object<I: Interface>(&mut self, client_handle: ClientHandle, id: u32) -> Result<Resource<I>, AddObjectError> {
		dbg!(I::NAME, client_handle, id);
		self.clients.get_mut(client_handle.0).ok_or(AddObjectError::ClientDoesntExist).and_then(|client| {
			if let Some(object_key) = client.objects.find_key(|object| object.id == id) {
				let object = client.objects.get_mut(object_key).unwrap();
				object.interface = Cow::Borrowed(I::NAME);
				object.version = I::VERSION;
				object.requests = I::REQUESTS;
				object.events = I::EVENTS;
				Ok(Resource::new(client_handle, ObjectHandle(object_key)))
			} else {
				let object_handle = ObjectHandle(client.objects.add(ObjectInfo {
					id,
					interface: Cow::Borrowed(I::NAME),
					version: I::VERSION,
					requests: I::REQUESTS,
					events: I::EVENTS,
				}));
				let resource = Resource::new(client_handle, object_handle);
				Ok(resource)
			}
		})
	} */

	/* pub fn insert_object_untyped(&mut self, client_handle: ClientHandle, id: u32, interface: DynInterface) -> Result<Resource<DynInterface>, AddObjectError> {
		dbg!(&interface, client_handle, id);
		self.clients.get_mut(client_handle.0).ok_or(AddObjectError::ClientDoesntExist).and_then(|client| {
			if let Some(object_key) = client.objects.find_key(|object| object.id == id) {
				let object = client.objects.get_mut(object_key).unwrap();
				object.interface = interface.name.clone();
				object.version = interface.version;
				object.requests = interface.requests;
				object.events = interface.events;
				Ok(Resource::new(client_handle, ObjectHandle(object_key)))
			} else {
				let object_handle = ObjectHandle(client.objects.add(ObjectInfo {
					id,
					interface: interface.name.clone(),
					version: interface.version,
					requests: interface.requests,
					events: interface.events,
				}));
				let resource = Resource::new(client_handle, object_handle);
				Ok(resource)
			}
		})
	} */

	pub fn get_or_add_object<I: Interface>(&mut self, client_handle: ClientHandle, id: u32) -> Result<Resource<I>, AddObjectError> {
		self.clients.get_mut(client_handle.0).ok_or(AddObjectError::ClientDoesntExist).and_then(|client| {
			if let Some(object_key) = client.objects.find_key(|object| object.id == id) {
				let object = client.objects.get(object_key).unwrap().clone();
				if object.interface == I::as_dyn() {
					Ok(Resource::<I>::new(client_handle, ObjectHandle(object_key)))
				} else {
					Err(AddObjectError::InterfaceMismatch)
				}
			} else {
				let object_handle = ObjectHandle(client.objects.add(ObjectInfo {
					id,
					interface: I::as_dyn(),
				}));
				let resource = Resource::<I>::new(client_handle, object_handle);
				Ok(resource)
			}
		})
	}

	pub fn get_or_add_object_untyped(&mut self, client_handle: ClientHandle, id: u32, interface: DynInterface) -> Result<Resource<DynInterface>, AddObjectError> {
		self.clients.get_mut(client_handle.0).ok_or(AddObjectError::ClientDoesntExist).and_then(|client| {
			if let Some(object_key) = client.objects.find_key(|object| object.id == id) {
				let object = client.objects.get(object_key).unwrap().clone();
				if object.interface == interface {
					Ok(Resource::new_with(client_handle, ObjectHandle(object_key), interface))
				} else {
					Err(AddObjectError::InterfaceMismatch)
				}
			} else {
				let object_handle = ObjectHandle(client.objects.add(ObjectInfo {
					id,
					interface: interface.clone(),
				}));
				let resource = Resource::new_with(client_handle, object_handle, interface);
				Ok(resource)
			}
		})
	}

	pub fn add_object<I: Interface>(&mut self, client_handle: ClientHandle, id: u32) -> Result<Resource<I>, AddObjectError> {
		self.clients.get_mut(client_handle.0).ok_or(AddObjectError::ClientDoesntExist).and_then(|client| {
			if client.objects.find_key(|object| object.id == id).is_some() {
				Err(AddObjectError::IdAlreadyTaken)
			} else {
				let object_handle = ObjectHandle(client.objects.add(ObjectInfo {
					id,
					interface: I::as_dyn(),
				}));
				let resource = Resource::<I>::new(client_handle, object_handle);
				Ok(resource)
			}
		})
	}

	pub fn add_object_dyn(&mut self, client_handle: ClientHandle, id: u32, interface: DynInterface) -> Result<Resource<DynInterface>, AddObjectError> {
		self.clients.get_mut(client_handle.0).ok_or(AddObjectError::ClientDoesntExist).and_then(|client| {
			if client.objects.find_key(|object| object.id == id).is_some() {
				Err(AddObjectError::IdAlreadyTaken)
			} else {
				let object_handle = ObjectHandle(client.objects.add(ObjectInfo {
					id,
					interface: interface.clone(),
				}));
				let resource = Resource::new_with(client_handle, object_handle, interface);
				Ok(resource)
			}
		})
	}

	pub fn add_object_untyped(&mut self, client_handle: ClientHandle, id: u32) -> Result<Resource<Untyped>, AddObjectError> {
		self.clients.get_mut(client_handle.0).ok_or(AddObjectError::ClientDoesntExist).and_then(|client| {
			if client.objects.find_key(|object| object.id == id).is_some() {
				Err(AddObjectError::IdAlreadyTaken)
			} else {
				let object_handle = ObjectHandle(client.objects.add(ObjectInfo {
					id,
					interface: DynInterface::new_anonymous(),
				}));
				let resource = Resource::new_untyped(client_handle, object_handle);
				Ok(resource)
			}
		})
	}

	pub fn add_global<I: Interface>(&mut self) -> GlobalHandle {
		let name = self.next_global_name;
		self.next_global_name += 1;
		let key = self.globals.add(GlobalInfo {
		    name,
		    interface: I::as_dyn(),
		});
		GlobalHandle(key)
	}

	pub fn get_global_info<I: Interface>(&self, handle: GlobalHandle) -> Option<&GlobalInfo> {
		self.get_global_info_untyped(handle)
			.filter(|info| info.interface == I::as_dyn())
	}

	pub fn get_global_info_untyped(&self, handle: GlobalHandle) -> Option<&GlobalInfo> {
		self.globals.get(handle.0)
	}

	pub fn find_global_handle<I: Interface, F: FnMut(&GlobalInfo) -> bool>(&self, mut predicate: F) -> Option<GlobalHandle> {
		self.globals
			.kv_iter()
			.filter(|(_key, info)| info.interface == I::as_dyn())
			.find(|(_key, info)| predicate(info))
			.map(|(key, _info)| GlobalHandle(key))
	}

	pub fn find_global_handle_untyped<F: FnMut(&GlobalInfo) -> bool>(&self, predicate: F) -> Option<GlobalHandle> {
		self.globals.find_key(predicate).map(GlobalHandle)
	}

	pub fn get_object_info<I: Interface>(&self, resource: &Resource<I>) -> Option<&ObjectInfo> {
		self.get_object_info_untyped(&resource.to_untyped())
			.filter(|info| info.interface == I::as_dyn())
	}

	pub fn get_object_info_mut<I: Interface>(&mut self, resource: &Resource<I>) -> Option<&mut ObjectInfo> {
		self.get_object_info_untyped_mut(&resource.to_untyped())
			.filter(|info| info.interface == I::as_dyn())
	}

	pub fn get_object_info_untyped(&self, resource: &Resource<Untyped>) -> Option<&ObjectInfo> {
		self.clients.get(resource.client().0)
			.and_then(|client| client.objects.get(resource.object().0))
	}

	pub fn get_object_info_untyped_mut(&mut self, resource: &Resource<Untyped>) -> Option<&mut ObjectInfo> {
		self.clients.get_mut(resource.client().0)
			.and_then(|client| client.objects.get_mut(resource.object().0))
	}

	pub fn get_resource_dyn(&self, resource: Resource<Untyped>) -> Option<Resource<DynInterface>> {
		self.get_object_info_untyped(&resource).map(|object_info| Resource::new_with(resource.client(), resource.object(), object_info.interface.clone()))
	}

	pub fn find_resource_dyn(&self, client_handle: ClientHandle, object_id: u32) -> Option<Resource<DynInterface>> {
		self.find_object_handle(client_handle, object_id)
			.and_then(|object_handle| self.get_object_info_untyped(&Resource::new_untyped(client_handle, object_handle)).map(|object_info| (object_handle, object_info)))
			.map(|(object_handle, object_info)| Resource::<DynInterface>::new_with(client_handle, object_handle, object_info.interface.clone()))
	}

	pub fn find_resource<I: Interface>(&self, client_handle: ClientHandle, object_id: u32) -> Option<Resource<I>> {
		self.find_resource_dyn(client_handle, object_id)
			.and_then(|untyped| untyped.downcast().ok())
	}

	pub fn find_object_handle(&self, client_handle: ClientHandle, object_id: u32) -> Option<ObjectHandle> {
		self.clients.get(client_handle.0).and_then(|client| {
			client.objects.find_key(|object| object.id == object_id).map(ObjectHandle)
		})
	}

	pub fn remove_client(&mut self, client_handle: ClientHandle) -> Option<Client> {
		self.clients.remove(client_handle.0)
	}
}

#[derive(Copy, Clone)]
pub struct Resource<I> {
	client: ClientHandle,
	object: ObjectHandle,
	interface: I,
}

#[derive(Debug, Clone)]
pub struct Untyped;

impl<I> Resource<I> {
	pub fn interface(&self) -> &I {
		&self.interface
	}

	pub fn client(&self) -> ClientHandle {
		self.client.clone()
	}

	pub fn object(&self) -> ObjectHandle {
		self.object.clone()
	}

	pub fn to_untyped(&self) -> Resource<Untyped> {
		Resource {
			client: self.client,
			object: self.object,
			interface: Untyped,
		}
	}
}

impl<I: Interface> Resource<I> {
	pub fn new(client: ClientHandle, object: ObjectHandle) -> Self {
		Self {
			client,
			object,
			interface: I::new(),
		}
	}

	pub fn to_dyn(&self) -> Resource<DynInterface> {
		Resource {
			client: self.client,
			object: self.object,
			interface: DynInterface {
				name: Cow::Borrowed(I::NAME),
				version: I::VERSION,
				requests: I::REQUESTS,
				events: I::EVENTS,
			}
		}
	}
}

impl Resource<DynInterface> {
	pub fn new_with(client: ClientHandle, object: ObjectHandle, interface: DynInterface) -> Self {
		Self {
			client,
			object,
			interface,
		}
	}

	pub fn downcast<I: Interface>(&self) -> Result<Resource<I>, ()> {
		// TODO subset testing? I sure hope not
		if I::as_dyn() == self.interface {
			Ok(Resource {
				client: self.client,
				object: self.object,
				interface: I::new(),
			})
		} else {
			Err(())
		}
	}
}

impl Resource<Untyped> {
	pub fn new_untyped(client: ClientHandle, object: ObjectHandle) -> Self {
		Resource {
			client,
			object,
			interface: Untyped,
		}
	}

	pub fn downcast<I: Interface>(&self, resources: &ResourceManager) -> Option<Resource<I>> {
		if let Some(object_info) = resources.get_object_info_untyped(self) {
			if I::as_dyn() == object_info.interface {
				Some(self.downcast_unchecked())
			} else {
				None
			}
		} else {
			None
		}
	}

	// TODO: not pub?
	pub fn downcast_unchecked<I: Interface>(&self) -> Resource<I> {
		Resource {
			client: self.client,
			object: self.object,
			interface: I::new(),
		}
	}
}

impl<I: Interface> fmt::Debug for Resource<I> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Resource<{}:{}>@({:?}.{:?})", I::NAME, I::VERSION, self.client.0, self.object.0)
	}
}

impl fmt::Debug for Resource<DynInterface> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Resource<(dyn){}:{}>@({:?}.{:?})", self.interface.name, self.interface.version, self.client.0, self.object.0)
	}
}

impl fmt::Debug for Resource<Untyped> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Resource<(untyped)>@({:?}.{:?})", self.client.0, self.object.0)
	}
}