//! The helper as a binary of its own, for the crate's tests: modlobby ships
//! the same entry point as `modlobby-sandbox`.

fn main() {
	sandbox::inside::main(std::env::args_os().collect())
}
