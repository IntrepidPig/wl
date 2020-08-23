use std::{
	ffi::{CString},
	cell::{RefCell},
	fmt,
};

use loaner::{
	Owner, Handle, Ref,
};

use wl_common::{
	interface::{Interface, Message, IntoArgsError, InterfaceTitle}, wire::{DynMessage},
};

use crate::{
	server::{State, SendEventError},
	net::{NetClient, NetError},
	resource::{Resource, Untyped, NewResource},
	object::{Object, ObjectMap, ObjectImplementation},
	global::{GlobalManager},
	protocol::*,
};

#[derive(Debug)]
pub struct ClientManager {
	pub(crate) this: Option<Handle<RefCell<ClientManager>>>,
	pub(crate) global_manager: Option<Handle<RefCell<GlobalManager>>>,
	pub(crate) clients: Vec<Owner<Client>>,
}

impl ClientManager {
	pub(crate) fn new() -> Self {
		Self {
			this: None,
			global_manager: None,
			clients: Vec::new(),
		}
	}

	pub(crate) fn set_this(&mut self, client_manager: Handle<RefCell<ClientManager>>) {
		self.this = Some(client_manager);
	}

	pub(crate) fn this(&self) -> Handle<RefCell<ClientManager>> {
		self.this.clone().expect("Client manager self-reference not set")
	}

	pub(crate) fn set_global_manager(&mut self, global_manager: Handle<RefCell<GlobalManager>>) {
		self.global_manager = Some(global_manager);
	}

	pub(crate) fn global_manager(&self) -> Handle<RefCell<GlobalManager>> {
		self.global_manager.clone().expect("Global manager not set")
	}

	pub fn create_client<S: 'static>(&mut self, net: NetClient, state: S) -> Handle<Client> {
		let client = Client::new(self.this(), self.global_manager(), net, state);
		let handle = client.handle();
		self.clients.push(client);
		handle
	}

	pub fn remove_client(&mut self, handle: Handle<Client>) -> Option<Owner<Client>> {
		self.clients.iter().position(|owner| owner.handle().is(&handle)).map(|position| self.clients.remove(position))
	}

	pub fn flush_clients(&self) -> Result<bool, NetError> {
		let mut flushed = true;
		for client in &self.clients {
			flushed = flushed && client.net.borrow_mut().flush()?;
		}
		Ok(flushed)
	}
}

// TODO: allow the user to associate dynamic data with a client as they do with objects
#[derive(Debug)]
pub struct Client {
	this: RefCell<Option<Handle<Client>>>, // TODO: ensure necessary
	client_manager: Handle<RefCell<ClientManager>>,
	global_manager: Handle<RefCell<GlobalManager>>,
	
	pub(crate) net: RefCell<NetClient>,
	pub(crate) objects: Owner<RefCell<ObjectMap>>, // TODO: remove from Owner,
	pub(crate) state: RefCell<State>,

	pub(crate) display: RefCell<Option<Resource<WlDisplay>>>,
	pub(crate) registry: RefCell<Option<Resource<WlRegistry>>>,
}

impl Client {
	pub(crate) fn new<S: 'static>(client_manager: Handle<RefCell<ClientManager>>, global_manager: Handle<RefCell<GlobalManager>>, net: NetClient, state: S) -> Owner<Self> {
		let mut objects = ObjectMap::new();
		objects.add(Owner::new(Object::new::<WlDisplay, _>(1)));
		let objects = Owner::new(RefCell::new(objects));
		let state = RefCell::new(State::new(Owner::new(state)));

		let partial = Owner::new(Self {
			this: RefCell::new(None),
			client_manager,
			global_manager,
			net: RefCell::new(net),
			objects,
			state,
			display: RefCell::new(None),
			registry: RefCell::new(None),
		});
		let handle = partial.handle();
		*partial.this.borrow_mut() = Some(handle.clone());

		let display = partial.find_by_id::<WlDisplay>(1).unwrap();
		display.set_implementation(WlDisplayImplementation);
		*partial.display.borrow_mut() = Some(display);

		partial
	}

	pub fn set_state<S: 'static>(&self, state: S) {
		*self.state.borrow_mut() = State::new(Owner::new(state));
	}

	pub fn state<'a, S: 'static>(&'a self) -> Ref<'a, S> {
		self.state.borrow().get::<Owner<S>>().custom_ref()
	}

	fn handle(&self) -> Handle<Client> {
		self.this.borrow().clone().expect("Handle not set")
	}

	pub(crate) fn advertise_current_globals(&self) {
		let global_manager = self.global_manager.get().unwrap();
		let global_manager = global_manager.borrow();
		for global in global_manager.globals() {
			let global = global.get().unwrap();
			self.advertise_global_dyn(global.name, global.interface.title())
		}
	}

	pub(crate) fn advertise_global<I: Interface>(&self, name: u32) {
		self.advertise_global_dyn(name, I::as_dyn().title())
	}

	pub(crate) fn advertise_global_dyn(&self, name: u32, title: InterfaceTitle) {
		if let Some(registry) = &*self.registry.borrow() {
			match registry.try_send_event(WlRegistryEvent::Global(wl_registry::GlobalEvent {
				name,
				interface: CString::new(title.name.as_bytes()).unwrap().into_bytes_with_nul(),
				version: title.version,
			})) {
				Ok(_) => {},
				Err(e) => {
					log::error!("Failed to advertise global to client: {}", e);
				}
			};
		} else {
			log::error!("Tried to add global before client's registry was initialized");
		}
	}

	// TODO: change all of the client_map-specific function signatures to look like this
	pub fn try_send_event<I: Interface>(&self, object: Handle<Object>, event: I::Event) -> Result<(), SendEventError> where I::Event: Message<ClientMap=ClientMap> + fmt::Debug {
		let object = object.get().ok_or(SendEventError::SenderMissing)?;

		let client_map = self.client_map();
		let (opcode, args) = event.into_args(client_map)?;
		let dyn_msg = DynMessage::new(object.id, opcode, args);
		let raw = dyn_msg.into_raw()?;

		self.net.borrow_mut().try_send_message(raw)?;

		Ok(())
	}

	pub(crate) fn remove_object(&self, object: Ref<Object>) -> Option<Owner<Object>> {
		let owner = self.objects.borrow_mut().remove(object.handle());
		let display = self.display.borrow().clone().expect("Client display not set");
		let delete_id_event = wl_display::DeleteIdEvent {
			id: object.id,
		};
		display.send_event(WlDisplayEvent::DeleteId(delete_id_event));
		owner
	}

	pub fn find<I: Interface, F: Fn(Resource<I>) -> bool>(&self, f: F) -> Option<Resource<I>> {
		// FUNKTIONAL (and scary)
		self.find_untyped(|resource| {
			resource.downcast().map(|resource| f(resource)).unwrap_or(false)
		}).and_then(|resource| resource.downcast())
	}

	pub fn find_untyped<F: Fn(Resource<Untyped>) -> bool>(&self, f: F) -> Option<Resource<Untyped>> {
		self.objects.borrow().find(|object| {
			let resource = Resource::new_untyped(self.handle(), object.handle());
			f(resource)
		}).map(|object| {
			Resource::new_untyped(self.handle(), object.handle())
		})
	}

	pub fn find_by_id<I: Interface>(&self, id: u32) -> Option<Resource<I>> {
		self.find(|resource| {
			resource.object().get().unwrap().id == id
		})
	}

	pub fn find_by_id_untyped(&self, id: u32) -> Option<Resource<Untyped>> {
		self.find_untyped(|resource| {
			resource.object().get().unwrap().id == id
		})
	}

	pub(crate) fn client_map(&self) -> ClientMap {
		ClientMap {
			handle: self.handle(),
		}
	}
}

// TODO: rename this to something that more clearly means "a reference to a client's map of objects"
// Right now the name seems like it means "a map of clients"
pub struct ClientMap {
	handle: Handle<Client>,
}

// TODO: review possibilites of the handle being null
impl ClientMap {
	pub fn try_get_object<I: Interface>(&self, id: u32) -> Option<Resource<I>> {
		let client = self.handle.get().expect("Client was destroyed");
		client.find_by_id(id)
	}

	pub fn try_get_object_untyped(&self, id: u32) -> Option<Resource<Untyped>> {
		let client = self.handle.get().expect("Client was destroyed");
		client.find_by_id_untyped(id)
	}

	pub fn try_get_id<I>(&self, resource: Resource<I>) -> Result<u32, IntoArgsError> {
		let untyped = resource.to_untyped();
		untyped.object().get().map(|object| object.id).ok_or(IntoArgsError::ResourceDoesntExist)
	}

	pub fn add_new_id<I, R>(&self, id: u32) -> NewResource<I> where R: Message<ClientMap=ClientMap> + fmt::Debug, I: Interface<Request=R> + fmt::Debug + 'static {
		let client = self.handle.get().expect("Client was destroyed");
		let object = Object::new::<I, R>(id);
		let object_owner = Owner::new(object);
		let object_handle = object_owner.handle();
		client.objects.borrow_mut().add(object_owner);
		NewResource::new(self.handle.clone(), object_handle)
	}

	// TODO: accept InterfaceTitle?
	pub fn add_new_id_untyped(&self, id: u32) -> NewResource<Untyped> {
		let client = self.handle.get().expect("Client was destroyed");
		let object = Object::new_untyped(id);
		let object_owner = Owner::new(object);
		let object_handle = object_owner.handle();
		client.objects.borrow_mut().add(object_owner);
		NewResource::new(self.handle.clone(), object_handle)
	}

	pub fn try_get_new_id<I>(&self, new_resource: &NewResource<I>) -> Result<(u32, InterfaceTitle), IntoArgsError> {
		new_resource.object.get().map(|object| (object.id, object.interface.get().title())).ok_or(IntoArgsError::ResourceDoesntExist)
	}
}

pub struct WlDisplayImplementation;

impl ObjectImplementation<WlDisplay> for WlDisplayImplementation {
    fn handle(&mut self, _state: &mut State, this: Resource<WlDisplay>, request: WlDisplayRequest) {
        match request {
			WlDisplayRequest::Sync(sync) => {
				let callback = sync.callback.register_fn((), |_, _, _| { }, |_, _| { });
				callback.send_event(WlCallbackEvent::Done(wl_callback::DoneEvent {
					callback_data: 1, // TODO!: serial
				}));
			},
			WlDisplayRequest::GetRegistry(get_registry) => {
				let registry = get_registry.registry.register((), WlRegistryImplementation);
				let client = this.client();
				let client = client.get().unwrap();
				*client.registry.borrow_mut() = Some(registry.clone());
				client.advertise_current_globals();
			},
		}
	}
	
	fn handle_destructor(&mut self, _state: &mut State, _this: Resource<WlDisplay>) {
		
	}
}

pub struct WlRegistryImplementation;

impl ObjectImplementation<WlRegistry> for WlRegistryImplementation {
    fn handle(&mut self, _state: &mut State, this: Resource<WlRegistry>, request: WlRegistryRequest) {
        match request {
			WlRegistryRequest::Bind(bind) => {
				let client = this.client();
				let client = client.get().unwrap();
				let global_manager = client.global_manager.get().unwrap();
				let global_manager = global_manager.borrow();
				global_manager.bind_global(bind.name, bind.id);
			}
		}
	}
	
	fn handle_destructor(&mut self, _state: &mut State, _this: Resource<WlRegistry>) {
		
	}
}