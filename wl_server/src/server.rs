#[allow(unused)]
use std::{
	io::{self, Read, Write},
	thread,
	sync::{
		mpsc::{self, Sender, Receiver},
	},
	os::unix::{
		io::{RawFd, AsRawFd},
		net::{UnixListener, UnixStream},
	},
	ffi::{CString},
	collections::{HashMap, VecDeque},
	cell::{RefCell},
	any::{Any},
	fmt,
};

use loaner::{Owner, Handle, Ref};
use thiserror::{Error};

use wl_common::{
	wire::{RawMessageReader, SerializeRawError, ParseDynError, RawMessage},
	interface::{Interface, IntoArgsError},
};

use crate::{
	net::{NetServer, NetError, ClientEvent, ClientEventPayload},
	client::{Client, ClientManager},
	global::{GlobalImplementation, GlobalManager, Global}, object::Object, Resource,
};

pub struct State {
	inner: Box<dyn Any>,
}

impl fmt::Debug for State {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("State")
			.field("inner", &"<opaque>")
			.finish()
	}
}

impl State {
	pub fn new<S: 'static>(state: S) -> Self {
		Self {
			inner: Box::new(state),
		}
	}

	pub fn get<S: 'static>(&self) -> &S {
		self.inner.downcast_ref().expect("State type mismatch")
	}

	pub fn get_mut<S: 'static>(&mut self) -> &mut S {
		self.inner.downcast_mut().expect("State type mismatch")
	}
}

pub struct Server {
	pub state: State,
	net: NetServer,
	client_manager: Owner<RefCell<ClientManager>>,
	global_manager: Owner<RefCell<GlobalManager>>,
	next_serial: u32,
}

impl Server {
	pub fn new<S: 'static>(state: S) -> Result<Self, ServerCreateError> {
		let net = NetServer::new()?;

		let client_manager = Owner::new(RefCell::new(ClientManager::new()));
		let global_manager = Owner::new(RefCell::new(GlobalManager::new(client_manager.handle())));
		client_manager.borrow_mut().set_global_manager(global_manager.handle());
		client_manager.borrow_mut().set_this(client_manager.handle());

		let state = State::new(state);

		Ok(Self {
			state,
			net,
			client_manager,
			global_manager,
			next_serial: 1,
		})
	}

	// TODO!: accept and propagate version number
	pub fn register_global<I: Interface + 'static, Impl: GlobalImplementation<I> + 'static>(&mut self, global_implementation: Impl) -> Handle<Global> {
		self.global_manager.borrow_mut().add_global(global_implementation)
	}

	pub fn run<S: 'static, F: FnMut(Handle<Client>) -> S>(&mut self, mut client_state_creator: F) -> Result<(), ServerError> {
		loop {
			match self.dispatch(&mut client_state_creator) {
				Ok(()) => {},
				Err(e) => log::error!("{}", e),
			}
		}
	}

	pub fn dispatch<S: 'static, F: FnMut(Handle<Client>) -> S>(&mut self, mut client_state_creator: F) -> Result<(), ServerError> {
		self.client_manager.borrow().flush_clients()?;

		match self.try_accept(&mut client_state_creator) {
			Ok(Some(_)) => log::info!("Client connected"),
			Ok(None) => {},
			Err(e) => log::error!("Client connection error: {:?}", e),
		}
		
		let client_event = self.net.poll_clients(&mut *self.client_manager.borrow_mut())?;
		if let Some(ClientEvent {
			client,
			payload,
		}) = client_event {
			let client = client.get().expect("Client doesn't exist");
			match payload {
				ClientEventPayload::ClientDisconnected => self.handle_client_disconnect(client)?,
				ClientEventPayload::Message(msg) => self.handle_client_message(client, msg)?,
			}
		}

		Ok(())
	}

	pub fn handle_client_disconnect(&mut self, client: Ref<Client>) -> Result<(), ServerError> {
		self.cleanup_client(client)?;

		Ok(())
	}

	pub fn handle_client_message(&mut self, client: Ref<Client>, raw_message: RawMessage) -> Result<(), ServerError> {
		let resource = match client.find_by_id_untyped(raw_message.header.sender) {
			Some(resource) => resource,
			None => return Err(ServerError::RequestReceiverDoesntExist),
		};
		let object_handle = resource.object();
		// This will fail if the client has sent a request before learning of the object's destruction
		let object = object_handle.get().ok_or(ServerError::RequestReceiverDoesntExist)?;

		let reader = RawMessageReader::new(&raw_message);
		let opcode = raw_message.header.opcode;
		let args = wl_common::wire::DynMessage::parse_dyn_args(object.interface.get().requests[raw_message.header.opcode as usize], reader)?;

		// wtf
		if false {} else {
			if let Some(dispatcher) = &mut *object.dispatcher.borrow_mut() {
				match dispatcher.dispatch(&mut self.state, resource.clone(), opcode, args) {
					Ok(_) => {},
					Err(e) => {
						log::error!("Failed to dispatch object request: {}", e);
					}
				}
			} else {
				log::error!("Received a request for an object with no associated dispatcher");
			}

			if object.destroy.get() {
				self.destroy_object(client, object);
			}
		}
		
		Ok(())
	}
	
	pub(crate) fn cleanup_client(&mut self, client: Ref<Client>) -> Result<(), ServerError> {
		while let Some(object) = client.objects.borrow_mut().remove_any() {
			self.run_object_destructor(client.clone(), object.custom_ref());
		}

		let _ = self.client_manager.borrow_mut().remove_client(client.handle());
		
		Ok(())
	}

	pub(crate) fn destroy_object(&mut self, client: Ref<Client>, object: Ref<Object>) {
		self.run_object_destructor(client.clone(), object.clone());
		let _ = client.remove_object(object);
	}

	fn run_object_destructor(&mut self, client: Ref<Client>, object: Ref<Object>) {
		if let Some(ref mut dispatcher) = *object.dispatcher.borrow_mut() {
			let resource = Resource::new_untyped(client.handle(), object.handle());
			match dispatcher.dispatch_destructor(&mut self.state, resource) {
				Ok(()) => {},
				Err(e) => {
					log::error!("Failed to run object destructor: {}", e);
				}
			}
		}
	}

	pub fn try_accept<S: 'static, F: FnOnce(Handle<Client>) -> S>(&mut self, state_creator: F) -> Result<Option<Handle<Client>>, ServerError> {
		if let Some(net) = self.net.try_accept()? {
			let handle = self.client_manager.borrow_mut().create_client(net, ());
			handle.get().unwrap().set_state(state_creator(handle.clone()));
			Ok(Some(handle))
		} else {
			Ok(None)
		}
	}
	
	// TODO: wonder about serials
	pub fn next_serial(&mut self) -> u32 {
		let serial = self.next_serial;
		// How should we handle serial exhaustion
		self.next_serial = self.next_serial.checked_add(1).expect("Serials exhausted");
		serial
	}

	pub fn print_debug_info(&self) {
		log::debug!("Debugging too difficult lmao");
	}
}

#[derive(Debug, Error)]
pub enum ServerError {
	#[error("Failed to create wayland server\n\t{0}")]
	CreateError(#[from] ServerCreateError),
	#[error(transparent)]
	NetError(#[from] NetError),
	#[error("Could not convert message arguments to a request\n\t{0}")]
	InvalidArguments(#[from] ParseDynError),
	#[error("An unknown IO error occurred\n\t{0}")]
	UnknownIoError(#[from] io::Error),
	#[error("A client sent a request to an object that doesn't exist")]
	RequestReceiverDoesntExist,
}

#[derive(Debug, Error)]
pub enum ServerCreateError {
	#[error(transparent)]
	NetError(#[from] NetError),
	#[error("An unknown IO error occurred")]
	UnknownIoError(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum SendEventError {
	#[error(transparent)]
	IntoArgsError(#[from] IntoArgsError),
	#[error(transparent)]
	SerializeRawError(#[from] SerializeRawError),
	#[error("The client referred to does not exist")]
	ClientMissing,
	#[error("The sender referred to does not exist")]
	SenderMissing,
	#[error(transparent)]
	Net(#[from] NetError),
}
