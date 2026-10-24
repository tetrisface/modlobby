//! modlobby's sandbox helper, shipped beside the app: the probe inside the
//! sandbox, the restricted exec of the engine on Linux, and the Wayland
//! proxy. It links none of the app's libraries, so it loads inside a
//! sandbox that holds only the system's.

fn main() {
	sandbox::inside::main(std::env::args_os().collect())
}
