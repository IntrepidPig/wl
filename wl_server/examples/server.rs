use wl_server::{
	Server, NewResource,
	protocol::*,
};

fn main() {
	setup_logging();

	log::info!("Starting server...");

	let state = State::new();
	let mut server = Server::new(state).unwrap();
	server.register_global::<WlCompositor, _>(|new_resource: NewResource<WlCompositor>| {
		new_resource.register(|wl_compositor, request| {
			dbg!(wl_compositor);
			dbg!(request);
		});
	});
	server.register_global::<WlShm, _>(|new_resource: NewResource<WlShm>| {
		new_resource.register(|_wl_shm, request| {
			match request {
				WlShmRequest::CreatePool(create_pool) => {
					dbg!(create_pool);
				},
			}
		});
	});
	server.run().unwrap();
}

pub struct State {

}

impl State {
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
