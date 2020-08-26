use wl_server::{
	Server, Resource, NewResource,
	protocol::*,
};

fn main() {
	setup_logging();

	log::info!("Starting server...");

	let state = State::new();
	let mut server = Server::new(state).unwrap();
	server.register_global::<WlCompositor, _>(|new_resource: NewResource<WlCompositor>| {
		new_resource.register_fn(
			(),
			|_state, compositor, request| {
				dbg!(compositor);
				dbg!(request);
			},
			|_, _| {
				log::info!("Compositor destroyed");
			}
		);
	});
	// TODO: these required closure argument type annotations can be mitigated by adding a `register_fn` function
	server.register_global::<WlShm, _>(|new_resource: NewResource<WlShm>| {
		new_resource.register_fn(
			ShmData { },
			|_state, shm: Resource<WlShm, ShmData>, request| {
				let _shm_data = shm.get_data();
				match request {
					WlShmRequest::CreatePool(create_pool) => {
						dbg!(create_pool);
					},
				}
			},
			|_, _| {
				log::info!("Shm destroyed");
			}
		);
	});
	server.run(|_this| ClientState::new()).unwrap();
}

pub struct ShmData {

}

pub struct State {

}

impl State {
	pub fn new() -> Self {
		Self {

		}
	}
}

pub struct ClientState {

}

impl ClientState {
	pub fn new() -> Self {
		Self {

		}
	}
}

fn setup_logging() {
	let colors = Box::new(fern::colors::ColoredLevelConfig::new())
		.info(fern::colors::Color::Blue)
		.warn(fern::colors::Color::Yellow)
		.error(fern::colors::Color::Red)
		.debug(fern::colors::Color::BrightGreen);
	fern::Dispatch::new()
		.format(move |out, message, record| out.finish(format_args!("[{}] {}", colors.color(record.level()), message)))
		.level(log::LevelFilter::Trace)
		.chain(std::io::stderr())
		.apply()
		.expect("Failed to setup logging dispatch");
}
