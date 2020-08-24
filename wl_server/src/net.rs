use std::{
	os::unix::{net::{UnixListener,  UnixStream}, io::{RawFd, AsRawFd}},
	io,
};

use nix::{
	poll,
	errno::Errno,
	sys::{socket, uio::{IoVec}},
};
use thiserror::{Error};
use loaner::{Handle};

use wl_common::{
	wire::{RawMessage, MessageHeader, ArgumentType},
};

use crate::{
	client::{Client, ClientManager},
};
use byteorder::{WriteBytesExt, NativeEndian};

/// Maximum amount of bytes that can be buffered 
const DATA_BUFFER_SIZE: usize = 1024 * 16; // 16 KiB
/// Maximum amount of file descriptors that can be buffered
const FD_BUFFER_SIZE: usize = 16;
/// Maximum amount of file descriptors that can be received in a single message
const MAX_FDS: usize = 8;
/// How many times to try receiving data after a header before returning an error
const RECV_TRIES: u32 = 2;
/// How many times to try sending data before returning an error
const FLUSH_TRIES: u32 = 2;

pub(crate) struct ClientEvent {
	pub client: Handle<Client>,
	pub payload: ClientEventPayload,
}

pub(crate) enum ClientEventPayload {
	ClientDisconnected,
	Message(RawMessage),
}


#[derive(Debug)]
pub struct NetServer {
	listener: UnixListener,
}

impl NetServer {
	pub fn new() -> Result<Self, NetError> {
		let listener = UnixListener::bind("/run/user/1000/wayland-0")
			.map_err(NetError::SocketBind)?;
		listener.set_nonblocking(true).expect("Failed to set listener as non-blocking");

		Ok(Self {
			listener,
		})
	}

	pub fn try_accept(&mut self) -> Result<Option<NetClient>, NetError> {
		match self.listener.accept() {
			Ok((stream, _addr)) => {
				Ok(Some(NetClient::new(stream)))
			},
			Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
				Ok(None)
			},
			Err(e) => {
				Err(NetError::AcceptError(e))
			},
		}
	}

	pub(crate) fn poll_clients(&mut self, client_manager: &mut ClientManager) -> Result<Option<ClientEvent>, NetError> {
		let poll_targets = client_manager.clients
			.iter()
			.map(|client| {
				(client.handle(), client.net.borrow().stream.as_raw_fd())
			})
			.collect::<Vec<_>>();
		let mut pollfds = poll_targets.iter().map(|(_client, fd)| poll::PollFd::new(*fd, poll::PollFlags::POLLIN)).collect::<Vec<_>>();

		poll::poll(&mut pollfds, 0).map_err(NetError::PollError)?;

		for (i, (client_handle, _fd)) in poll_targets.iter().enumerate() {
			let pollfd = &pollfds[i];
			if pollfd.revents().map(|revents| !(revents & poll::PollFlags::POLLIN).is_empty()).unwrap_or(false) {
				if !(pollfd.revents().unwrap() & poll::PollFlags::POLLHUP).is_empty() {
					return Ok(Some(ClientEvent {
						client: client_handle.clone(),
						payload: ClientEventPayload::ClientDisconnected,
					}))
				}
			}

			let client = client_handle.get().unwrap();
			let mut net_client = client.net.borrow_mut();

			match net_client.try_read_message(&*client) {
				Ok(Some(msg)) => return Ok(Some(ClientEvent {
					client: client_handle.clone(),
					payload: ClientEventPayload::Message(msg),
				})),
				Ok(None) => {},//log::error!("Received no event from client after poll"),
				Err(e) => return Err(e),
			}
		}

		Ok(None)
	}
}

#[derive(Debug)]
pub struct NetClient {
	stream: UnixStream,
	in_buffer: MessageBuffer,
	out_buffer: MessageBuffer,
}

impl NetClient {
	pub fn new(stream: UnixStream) -> Self {
		Self {
			stream,
			in_buffer: MessageBuffer::new(),
			out_buffer: MessageBuffer::new(),
		}
	}

	pub fn try_read_message(&mut self, client: &Client) -> Result<Option<RawMessage>, NetError> {
		// Read at least a message header
		if !self.try_fill_buffer_until(8, 0, RECV_TRIES)? {
			return Ok(None);
		};

		let header = MessageHeader::from_bytes(&self.in_buffer.data[..8]).unwrap();

		let objects = client.objects.borrow();
		let object = objects.find(|object| object.id == header.sender).ok_or(NetError::InvalidMessage)?;
		let expected_fds = object.interface.get().requests[header.opcode as usize].iter().filter(|arg| arg.arg_type == ArgumentType::Fd).count();

		// Read the rest of the message
		if !self.try_fill_buffer_until(header.msg_size as usize, expected_fds, RECV_TRIES)? {
			return Err(NetError::InsufficientData);
		}

		let (data, fds) = self.in_buffer.advance(header.msg_size as usize, expected_fds);
		let raw = RawMessage {
			header,
			data: data[8..].to_owned(),
			fds,
		};

		Ok(Some(raw))
	}

	pub fn try_send_message(&mut self, message: RawMessage) -> Result<bool, NetError> {
		let mut data = Vec::with_capacity(message.header.msg_size as usize);
		data.write_u32::<NativeEndian>(message.header.sender).unwrap();
		data.write_u16::<NativeEndian>(message.header.opcode).unwrap();
		data.write_u16::<NativeEndian>(message.header.msg_size).unwrap();
		data.extend_from_slice(&message.data);

		if self.flush()? {
			self.try_send_data(data, message.fds)
		} else {
			self.out_buffer.append(&data, &message.fds)?;
			Ok(false)
		}
	}

	fn try_fill_buffer(&mut self) -> Result<bool, NetError> {
		let fd = self.stream.as_raw_fd();
		let mut cmsg_buf = nix::cmsg_space!([RawFd; MAX_FDS]);
		let iovec = IoVec::from_mut_slice(&mut self.in_buffer.data[self.in_buffer.data_len..]);
		let flags = socket::MsgFlags::MSG_CMSG_CLOEXEC | socket::MsgFlags::MSG_DONTWAIT;

		let recv = match socket::recvmsg(fd, &[iovec], Some(&mut cmsg_buf), flags) {
			Ok(recv) => recv,
			Err(nix::Error::Sys(Errno::EAGAIN)) => return Ok(false),
			Err(e) => return Err(NetError::RecvError(e)),
		};
		for cmsg in recv.cmsgs() {
			match cmsg {
				socket::ControlMessageOwned::ScmRights(fds_) => self.in_buffer.fds.extend_from_slice(&fds_),
				_ => {},
			}
		}

		self.in_buffer.data_len += recv.bytes;

		Ok(true)
	}

	fn try_fill_buffer_until(&mut self, data_len: usize, fd_count: usize, tries: u32) -> Result<bool, NetError> {
		for _ in 0..tries {
			self.try_fill_buffer()?;
			if self.in_buffer.data_len >= data_len && self.in_buffer.fds.len() >= fd_count {
				return Ok(true);
			}
		}
		Ok(self.in_buffer.data_len >= data_len && self.in_buffer.fds.len() >= fd_count)
	}

	fn try_send_data(&mut self, data: Vec<u8>, fds: Vec<RawFd>) -> Result<bool, NetError> {
		let fd = self.stream.as_raw_fd();
		let iovec = IoVec::from_slice(&data);
		let cmsg = socket::ControlMessage::ScmRights(&fds);
		let flags = socket::MsgFlags::MSG_DONTWAIT;

		Ok(match socket::sendmsg(fd, &[iovec], &[cmsg], flags, None) {
			Ok(n) => {
				if n > 0 {
					self.out_buffer.append(&data[n..], &[])?;
					false
				} else {
					true
				}
			},
			Err(nix::Error::Sys(Errno::EAGAIN)) => {
				self.out_buffer.append(&data, &fds)?;
				false
			},
			Err(e) => return Err(NetError::SendError(e)),
		})
	}

	pub fn flush(&mut self) -> Result<bool, NetError> {
		if self.out_buffer.is_empty() {
			return Ok(true);
		}

		for _ in 0..FLUSH_TRIES {
			let (data, fds) = self.out_buffer.advance_all();
			if self.try_send_data(data, fds)? {
				return Ok(true);
			};
		}

		Ok(false)
	}
}

#[derive(Debug)]
struct MessageBuffer {
	data: Vec<u8>,
	data_len: usize,
	fds: Vec<RawFd>,
}

impl MessageBuffer {
	fn new() -> Self {
		Self {
			data: vec![0u8; DATA_BUFFER_SIZE],
			data_len: 0,
			fds: Vec::with_capacity(FD_BUFFER_SIZE),
		}
	}

	fn is_empty(&self) -> bool {
		self.data_len == 0 && self.fds.is_empty()
	}

	fn advance(&mut self, data_len: usize, fd_count: usize) -> (Vec<u8>, Vec<RawFd>) {
		let data_left = self.data.split_off(data_len);
		let data = std::mem::replace(&mut self.data, data_left);
		self.data_len -= data_len;
		if self.data.len() < DATA_BUFFER_SIZE {
			self.data.resize(DATA_BUFFER_SIZE, 0u8);
		}

		let fds_left = self.fds.split_off(fd_count);
		let fds = std::mem::replace(&mut self.fds, fds_left);

		(data, fds)
	}

	fn advance_all(&mut self) -> (Vec<u8>, Vec<RawFd>) {
		self.advance(self.data_len, self.fds.len())
	}

	fn append(&mut self, data: &[u8], fds: &[RawFd]) -> Result<(), NetError> {
		if self.data_len + data.len() > DATA_BUFFER_SIZE {
			return Err(NetError::BufferFull);
		}
		if self.fds.len() + fds.len() > FD_BUFFER_SIZE {
			return Err(NetError::BufferFull);
		}

		self.data.resize(self.data_len, 0u8);
		self.data.extend_from_slice(data);
		self.data_len += data.len();

		self.fds.extend_from_slice(fds);

		Ok(())
	}
}

#[derive(Debug, Error)]
pub enum NetError {
	#[error("Failed to bind socket\n\t{0}")]
	SocketBind(#[source] io::Error),
	#[error("Failed to accept connection from client\n\t{0}")]
	AcceptError(#[source] io::Error),
	#[error("Failed to poll clients\n\t{0}")]
	PollError(#[source] nix::Error),
	#[error("Failed to read socket\n\t{0}")]
	RecvError(#[source] nix::Error),
	#[error("Failed to write to socket\n\t{0}")]
	WriteError(#[source] io::Error),
	#[error("Failed to send message on socket\n\t{0}")]
	SendError(#[source] nix::Error),
	#[error("The client did not send a full message")]
	InsufficientData,
	#[error("Message buffers grew too large")]
	BufferFull,
	#[error("Failed to parse data as a message")]
	InvalidMessage,
}
