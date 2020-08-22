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

use loaner::{Owner, Handle};
use thiserror::{Error};

use wl_common::{
	wire::{MessageHeader, RawMessage, RawMessageReader, SerializeRawError},
	interface::{Interface, IntoArgsError},
};

use crate::{
	client::{Client, ClientManager},
	global::{GlobalImplementation, GlobalManager, Global},
};

pub enum ServerMessage {
	NewClient(UnixStream),
}

#[derive(Debug, Error)]
pub enum ServerError {
	#[error("Failed to create wayland server")]
	SocketBind(#[from] ServerCreateError),
	#[error("Failed to accept connection from client")]
	AcceptError(#[source] io::Error),
	#[error("Received a message in an invalid format")]
	InvalidMessage,
	#[error("An unknown IO error occurred")]
	UnknownIoError(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum ServerCreateError {
	#[error("Failed to bind wayland server socket")]
	SocketBind(#[source] io::Error),
	#[error("An unknown IO error occurred")]
	UnknownIoError(#[from] io::Error),
}

const MAX_MESSAGE_SIZE: usize = 4096;
const MAX_FDS: usize = 16;

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
	listener: UnixListener,
	client_manager: Owner<RefCell<ClientManager>>,
	global_manager: Owner<RefCell<GlobalManager>>,
	msg_buf: Box<[u8; MAX_MESSAGE_SIZE]>,
	next_serial: u32,
}

impl Server {
	pub fn new<S: 'static>(state: S) -> Result<Self, ServerCreateError> {
		let listener = UnixListener::bind("/run/user/1000/wayland-0")
			.map_err(|e| ServerCreateError::SocketBind(e))?;
		listener.set_nonblocking(true)?;

		let client_manager = Owner::new(RefCell::new(ClientManager::new()));
		let global_manager = Owner::new(RefCell::new(GlobalManager::new(client_manager.handle())));
		client_manager.borrow_mut().set_global_manager(global_manager.handle());
		client_manager.borrow_mut().set_this(client_manager.handle());

		let state = State::new(state);

		Ok(Self {
			state,
			listener,
			client_manager,
			global_manager,
			msg_buf: Box::new([0u8; MAX_MESSAGE_SIZE]),
			next_serial: 1,
		})
	}

	// TODO!: accept and propagate version number
	pub fn register_global<I: Interface + 'static, Impl: GlobalImplementation<I> + 'static>(&mut self, global_implementation: Impl) -> Handle<Global> {
		self.global_manager.borrow_mut().add_global(global_implementation)
	}

	pub fn run<S: 'static, F: FnMut(Handle<Client>) -> S>(&mut self, mut client_state_creator: F) -> Result<(), ServerError> {
		loop {
			match self.try_accept(&mut client_state_creator) {
				Ok(Some(_)) => log::info!("Client connected"),
				Ok(None) => {},
				Err(e) => log::error!("Client connection error: {:?}", e),
			}
			
			if let Some((client_handle, raw_message)) = self.try_next_raw_message()? {
				let client = client_handle.get().expect("Client was destroyed");
				let resource = match client.find_by_id_untyped(raw_message.header.sender) {
					Some(resource) => resource,
					None => {
						log::warn!("A client sent an event to a resource that no longer exists; ignoring...");
						continue;
					}
				};
				let object_handle = resource.object();
				let object = object_handle.get().unwrap();

				let reader = RawMessageReader::new(&raw_message);
				let opcode = raw_message.header.opcode;
				let args = match wl_common::wire::DynMessage::parse_dyn_args(object.interface.requests[raw_message.header.opcode as usize], reader) {
					Ok(args) => args,
					Err(e) => {
						log::error!("Failed to parse client message: {}", e);
						continue;
					}
				};

				// wtf
				if false {} else {
					if let Some(dispatcher) = &mut *object.dispatcher.borrow_mut() {
						match dispatcher.dispatch(&mut self.state, resource.to_untyped(), opcode, args) {
							Ok(_) => {},
							Err(e) => {
								log::error!("{}", e);
							}
						}
					} else {
						log::error!("Received a request for an object with no associated dispatcher");
					}
				}
			}
		}
	}

	pub fn try_accept<S: 'static, F: FnOnce(Handle<Client>) -> S>(&mut self, state_creator: F) -> Result<Option<Handle<Client>>, ServerError> {
		if let Some(stream) = self.try_accept_stream()? {
			let handle = self.client_manager.borrow_mut().create_client(stream, ());
			handle.get().unwrap().set_state(state_creator(handle.clone()));
			Ok(Some(handle))
		} else {
			Ok(None)
		}
	}

	fn try_accept_stream(&mut self) -> Result<Option<UnixStream>, ServerError> {
		match self.listener.accept() {
			Ok((stream, _addr)) => {
				Ok(Some(stream))
			},
			Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
				Ok(None)
			},
			Err(e) => {
				Err(ServerError::AcceptError(e))
			},
		}
	}

	fn try_next_raw_message(&mut self) -> Result<Option<(Handle<Client>, RawMessage)>, ServerError> {
		use nix::{
			sys::{socket, uio::IoVec},
			poll,
		};

		let poll_targets = self.client_manager.borrow().clients
			.iter()
			.map(|client| {
				(client.handle(), client.stream.borrow().as_raw_fd())
			})
			.collect::<Vec<_>>();
		let mut pollfds = poll_targets.iter().map(|t| poll::PollFd::new(t.1, poll::PollFlags::POLLIN)).collect::<Vec<_>>();
		poll::poll(&mut pollfds, 0).map_err(|e| {
			log::error!("Polling fds failed: {}", e);
		}).unwrap();

		let mut cmsg_buffer = nix::cmsg_space!([RawFd; MAX_FDS]);

		for (i, (client_handle, _)) in poll_targets.iter().enumerate() {
			let pollfd = &pollfds[i];
			if pollfd.revents().map(|revents| !(revents & poll::PollFlags::POLLIN).is_empty()).unwrap_or(false) {
				if !(pollfd.revents().unwrap() & poll::PollFlags::POLLHUP).is_empty() {
					// TODO: destroy client, and watch for borrow_mut panics
					self.client_manager.borrow_mut().destroy_client(client_handle.clone());
					log::trace!("Client {:?} disconnected", client_handle);
					continue;
				}

				let fd = pollfd.fd();
				cmsg_buffer.clear();
				
				let mut header_buf = [0u8; 8];
				let iovec = IoVec::from_mut_slice(&mut header_buf);
				let flags = socket::MsgFlags::MSG_PEEK | socket::MsgFlags::MSG_DONTWAIT;
				let recv = socket::recvmsg(fd, &[iovec], None, flags).unwrap();
				if recv.bytes != 8 {
					log::error!("Header read returned {} bytes instead of 8", recv.bytes);
					return Err(ServerError::InvalidMessage)
				}
				let msg_header = MessageHeader::from_bytes(&header_buf).unwrap();

				let iovec = IoVec::from_mut_slice(&mut self.msg_buf[..msg_header.msg_size as usize]);
				let flags = socket::MsgFlags::MSG_CMSG_CLOEXEC | socket::MsgFlags::MSG_DONTWAIT;
				let recv = socket::recvmsg(fd, &[iovec], Some(&mut cmsg_buffer), flags).unwrap();
				let buf = &self.msg_buf[..recv.bytes];
				let mut fds = Vec::new();
				let _ = recv.cmsgs().map(|cmsg| match cmsg {
					socket::ControlMessageOwned::ScmRights(fds_) => fds.extend_from_slice(&fds_),
					_ => {},
				}).collect::<()>();
				let raw = match RawMessage::from_data(buf, fds).map_err(|_| ServerError::InvalidMessage) {
					Ok(raw) => raw,
					Err(e) => {
						log::error!("Failed to read message from client: {}", e);
						continue;
					}
				};

				return Ok(Some((client_handle.clone(), raw)));
			}
		}

		Ok(None)
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
	Io(#[from] io::Error),
}
