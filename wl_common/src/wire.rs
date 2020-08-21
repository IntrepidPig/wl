use std::{
	os::unix::io::RawFd,
	convert::{TryFrom},
};

use byteorder::{NativeEndian, ReadBytesExt, WriteBytesExt};
use thiserror::Error;

use crate::{
	interface::{Message, InterfaceTitle},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixed(pub u32);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MessageHeader {
	pub sender: u32,
	pub opcode: u16,
	pub msg_size: u16,
}

impl MessageHeader {
	pub fn from_bytes(bytes: &[u8]) -> Result<Self, ()> {
		if bytes.len() != 8 {
			return Err(());
		}

		let mut cursor = std::io::Cursor::new(bytes);
		let sender = cursor.read_u32::<NativeEndian>().map_err(|_| ())?;
		let opcode = cursor.read_u16::<NativeEndian>().map_err(|_| ())?;
		let msg_size = cursor.read_u16::<NativeEndian>().map_err(|_| ())?;
		Ok(Self {
			sender,
			opcode,
			msg_size,
		})
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMessage {
	pub header: MessageHeader,
	pub data: Vec<u8>,
	pub fds: Vec<RawFd>, // TODO leave this as Vec<u8>
}

impl RawMessage {
	pub fn from_data(bytes: &[u8], fds: Vec<RawFd>) -> Result<Self, ()> {
		if bytes.len() < 8 {
			return Err(());
		}
		let header = MessageHeader::from_bytes(&bytes[..8])?;
		let data = bytes[8..].to_vec();
		Ok(Self {
			header,
			data,
			fds,
		})
	}

	pub fn from_data_without_header(header: MessageHeader, bytes: Vec<u8>, fds: Vec<RawFd>) -> Self {
		Self {
			header,
			data: bytes,
			fds,
		}
	}
}

#[derive(Debug, Clone)]
pub struct RawMessageReader<'a, 'b> {
	pub header: MessageHeader,
	data: std::io::Cursor<&'a [u8]>,
	fds: &'b [RawFd],
	next_fd: usize,
}

impl<'a> RawMessageReader<'a, 'a> {
	pub fn new(raw: &'a RawMessage) -> Self {
		Self {
			header: raw.header,
			data: std::io::Cursor::new(&raw.data),
			fds: &raw.fds,
			next_fd: 0,
		}
	}
}

impl<'a, 'b> RawMessageReader<'a, 'b> {
	pub fn next_int(&mut self) -> Result<i32, ParseRawError> {
		self.data.read_i32::<NativeEndian>().map_err(From::from)
	}

	pub fn next_uint(&mut self) -> Result<u32, ParseRawError> {
		self.data.read_u32::<NativeEndian>().map_err(From::from)
	}

	pub fn next_fixed(&mut self) -> Result<Fixed, ParseRawError> {
		self.next_uint().map(Fixed)
	}

	// TODO convert to CString maybe (required trailing nul concerns say maybe not)
	pub fn next_string(&mut self) -> Result<Option<Vec<u8>>, ParseRawError> {
		let array = self.next_array()?;
		if array.is_empty() {
			Ok(None)
		} else {
			Ok(Some(array))
		}
	}
	
	pub fn next_object(&mut self) -> Result<Option<u32>, ParseRawError> {
		let id = self.next_uint()?;
		if id == 0 {
			Ok(None)
		} else {
			Ok(Some(id))
		}
	}

	pub fn next_new_id(&mut self) -> Result<u32, ParseRawError> {
		self.next_uint()
	}

	pub fn next_new_id_anonymous(&mut self) -> Result<(u32, InterfaceTitle), ParseRawError> {
		let name = String::from_utf8(self.next_string()?.unwrap()).unwrap(); // TODO treat non-utf8 properly, i.e., use CString instead
		let version = self.next_uint()?;
		let id = self.next_uint()?;
		Ok((id, InterfaceTitle::new(name, version)))
	}

	pub fn next_array(&mut self) -> Result<Vec<u8>, ParseRawError> {
		let len = self.next_uint()?;
		let mut buf = Vec::new();
		for _ in 0..len {
			let b = self.data.read_u8().map_err(ParseRawError::IoError)?;
			buf.push(b);
		}
		// Read padding to the next 32 bit alignment position
		for _ in 0..((4 - len % 4) % 4) {
			self.data.read_u8().map_err(ParseRawError::IoError)?;
		}
		Ok(buf)
	}

	pub fn next_fd(&mut self) -> Result<RawFd, ParseRawError> {
		if self.next_fd < self.fds.len() {
			let fd = self.fds[self.next_fd];
			self.next_fd += 1;
			Ok(fd)
		} else {
			Err(ParseRawError::InsufficientFds)
		}
	}
}

#[derive(Debug, Clone)]
pub struct DynMessage {
	pub sender: u32,
	pub opcode: u16,
	pub arguments: Vec<DynArgument>,
}

impl DynMessage {
	pub fn new(sender: u32, opcode: u16, arguments: Vec<DynArgument>) -> Self {
		Self {
			sender,
			opcode,
			arguments,
		}
	}

	pub fn from_raw(args_desc: &[ArgumentDesc], reader: RawMessageReader) -> Result<Self, ParseRawError> {
		Ok(Self {
			sender: reader.header.sender,
			opcode: reader.header.opcode,
			arguments: Self::parse_dyn_args(args_desc, reader)?,
		})
	}

	pub fn into_raw(&self) -> Result<RawMessage, SerializeRawError> {
		let (data, fds) = Self::serialize_raw_args(&self.arguments)?;
		Ok(RawMessage {
		    header: MessageHeader {
		        sender: self.sender,
		        opcode: self.opcode,
		        msg_size: u16::try_from(data.len() + 8).map_err(|_| SerializeRawError::MessageTooLong)?,
			},
		    data,
		    fds,
		})
	}

	pub fn serialize_raw_args(args: &[DynArgument]) -> Result<(Vec<u8>, Vec<RawFd>), SerializeRawError> {
		let mut buf = Vec::new();
		let mut fds = Vec::new();

		// Writes an array of bytes as is to a buffer, including the length, contents, and padding
		fn write_array(buf: &mut Vec<u8>, array: &[u8]) -> Result<(), SerializeRawError> {
			let len = u32::try_from(array.len()).map_err(|_| SerializeRawError::ArrayTooLong)?;
			buf.write_u32::<NativeEndian>(len).unwrap();
			buf.extend_from_slice(array);
			let padding = (4 - (len % 4)) % 4;
			for _ in 0..padding {
				buf.push(0u8);
			}
			Ok(())
		}

		for arg in args {
			match *arg {
			    DynArgument::Int(v) => buf.write_i32::<NativeEndian>(v).unwrap(),
			    DynArgument::Uint(v) => buf.write_u32::<NativeEndian>(v).unwrap(),
			    DynArgument::Fixed(v) => buf.write_u32::<NativeEndian>(v.0).unwrap(),
			    DynArgument::String(ref v) => if let Some(v) = v {
					// TODO worry about interior nul bytes (likely by making this a CString)
					write_array(&mut buf, v)?;
				} else {
					// Zero-length string means null probably because a non-null string would have
					// a length of at least 1 due to the null terminator
					buf.write_u32::<NativeEndian>(0u32).unwrap();
				}
			    DynArgument::Object(v) => if let Some(v) = v {
					buf.write_u32::<NativeEndian>(v).unwrap();
				} else {
					buf.write_u32::<NativeEndian>(0).unwrap();
				}
			    DynArgument::NewId(v, ref interface) => {
					if let Some(interface) = interface {
						let c_name = std::ffi::CString::new(interface.name.as_bytes()).unwrap();
						write_array(&mut buf, c_name.as_bytes_with_nul())?;
					}
					buf.write_u32::<NativeEndian>(v).unwrap();
				}
			    DynArgument::Array(ref v) => write_array(&mut buf, v)?,
			    DynArgument::Fd(v) => fds.push(v),
			}
		}
		Ok((buf, fds))
	}

	pub fn parse_dyn_args(args_desc: &[ArgumentDesc], mut reader: RawMessageReader) -> Result<Vec<DynArgument>, ParseRawError> {
		let mut args = Vec::new();
		for arg_desc in args_desc {
			match arg_desc.arg_type {
			    ArgumentType::Int => args.push(DynArgument::Int(reader.next_int()?)),
			    ArgumentType::Uint => args.push(DynArgument::Uint(reader.next_uint()?)),
			    ArgumentType::Fixed => args.push(DynArgument::Fixed(reader.next_fixed()?)),
			    ArgumentType::String => args.push(DynArgument::String(reader.next_string()?)),
			    ArgumentType::Object => {
					let next_object = reader.next_object()?;
					args.push(DynArgument::Object(next_object))
				},
			    ArgumentType::NewId => {
					if arg_desc.interface.is_some() {
						let id = reader.next_new_id()?;
						args.push(DynArgument::NewId(id, None));
					} else {
						let (id, title) = reader.next_new_id_anonymous()?;
						args.push(DynArgument::NewId(id, Some(title)));
					}
				}
			    ArgumentType::Array => args.push(DynArgument::Array(reader.next_array()?)),
			    ArgumentType::Fd => args.push(DynArgument::Fd(reader.next_fd()?)),
			}
		}
		Ok(args)
	}
}

#[derive(Debug, Error)]
pub enum ParseRawError {
	#[error("An error occurred while parsing a message; the likely cause is insufficient data sent")]
	IoError(#[from] std::io::Error),
	#[error("The message did not contain the expected amount of file descriptors")]
	InsufficientFds,
	#[error("The message referenced an object id that does not exist")]
	ObjectDoesntExist,
}

#[derive(Debug, Error)]
pub enum SerializeRawError {
	#[error("Tried to serialize a message with an array whose length exceeds a 32 bit size")]
	ArrayTooLong,
	#[error("The size of the message after serializing exceeded a 16 bit size")]
	MessageTooLong,
	#[error("The message referenced an object id that does not exist")]
	ObjectDoesntExist,
}

#[derive(Debug, Error)]
pub enum ArgumentError {
	#[error("Not enough arguments were passed")]
	InsufficientArguments,
	#[error("Arguments with incorrect types were passed")]
	IncorrectArguments,
}

#[derive(Debug, Clone)]
pub enum DynArgument {
	Int(i32),
	Uint(u32),
	Fixed(Fixed),
	String(Option<Vec<u8>>), // TODO wrap this Vec in a newtype to make helper functions easily available and allow more type inference shenanigans
	Object(Option<u32>),
	NewId(u32, Option<InterfaceTitle>),
	Array(Vec<u8>),
	Fd(RawFd),
}

#[derive(Debug, Clone)]
pub struct DynArgumentReader {
	args: Vec<DynArgument>,
}

// TODO better solution than remove(0)
impl DynArgumentReader {
	pub fn from_args(args: Vec<DynArgument>) -> Self {
		Self {
			args
		}
	}

	pub fn next_arg(&mut self) -> Option<DynArgument> {
		if self.args.is_empty() { None } else { Some(self.args.remove(0)) }
	}

	pub fn next_int(&mut self) -> Result<i32, ArgumentError> {
		if let DynArgument::Int(v) = self.next_arg().ok_or(ArgumentError::InsufficientArguments)? { Ok(v) } else { Err(ArgumentError::IncorrectArguments) }
	}

	pub fn next_uint(&mut self) -> Result<u32, ArgumentError> {
		if let DynArgument::Uint(v) = self.next_arg().ok_or(ArgumentError::InsufficientArguments)? { Ok(v) } else { Err(ArgumentError::IncorrectArguments) }
	}

	pub fn next_fixed(&mut self) -> Result<Fixed, ArgumentError> {
		if let DynArgument::Fixed(v) = self.next_arg().ok_or(ArgumentError::InsufficientArguments)? { Ok(v) } else { Err(ArgumentError::IncorrectArguments) }
	}

	pub fn next_string(&mut self) -> Result<Option<Vec<u8>>, ArgumentError> {
		if let DynArgument::String(v) = self.next_arg().ok_or(ArgumentError::InsufficientArguments)? { Ok(v) } else { Err(ArgumentError::IncorrectArguments) }
	}

	pub fn next_object(&mut self) -> Result<Option<u32>, ArgumentError> {
		if let DynArgument::Object(v) = self.next_arg().ok_or(ArgumentError::InsufficientArguments)? { Ok(v) } else { Err(ArgumentError::IncorrectArguments) }
	}

	pub fn next_new_id(&mut self) -> Result<(u32, Option<InterfaceTitle>), ArgumentError> {
		if let DynArgument::NewId(v, interface) = self.next_arg().ok_or(ArgumentError::InsufficientArguments)? { Ok((v, interface)) } else { Err(ArgumentError::IncorrectArguments) }
	}

	pub fn next_array(&mut self) -> Result<Vec<u8>, ArgumentError> {
		if let DynArgument::Array(v) = self.next_arg().ok_or(ArgumentError::InsufficientArguments)? { Ok(v) } else { Err(ArgumentError::IncorrectArguments) }
	}

	pub fn next_fd(&mut self) -> Result<RawFd, ArgumentError> {
		if let DynArgument::Fd(v) = self.next_arg().ok_or(ArgumentError::InsufficientArguments)? { Ok(v) } else { Err(ArgumentError::IncorrectArguments) }
	}
}

#[derive(Debug)]
pub struct MessageData<M: Message> { // TODO remove
	sender: u32,
	message: M,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentType {
	Int,
	Uint,
	Fixed,
	String,
	Object,
	NewId,
	Array,
	Fd,
}

impl ArgumentType {
	pub fn to_string(self) -> &'static str {
		match self {
			ArgumentType::Int => "int",
		    ArgumentType::Uint => "uint",
		    ArgumentType::Fixed => "fixed",
		    ArgumentType::String => "string",
		    ArgumentType::Object => "object",
		    ArgumentType::NewId => "new_id",
		    ArgumentType::Array => "array",
		    ArgumentType::Fd => "fd",
		}
	}

	pub fn from_str(s: &str) -> Option<Self> {
		Self::from_bytes(s.as_bytes())
	}

	pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
		Some(match bytes {
			b"int" => ArgumentType::Int,
			b"uint" => ArgumentType::Uint,
			b"fixed" => ArgumentType::Fixed,
			b"string" => ArgumentType::String,
			b"object" => ArgumentType::Object,
			b"new_id" => ArgumentType::NewId,
			b"array" => ArgumentType::Array,
			b"fd" => ArgumentType::Fd,
			_ => return None,
		})
	}
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentDesc {
	pub arg_type: ArgumentType,
	pub interface: Option<&'static str>,
	pub allow_null: bool,
}
