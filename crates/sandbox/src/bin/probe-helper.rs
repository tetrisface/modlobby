//! The probe as a binary of its own, for the crate's tests: modlobby itself
//! answers `--sandbox-probe` the same way.

fn main() {
	match sandbox::probe::arg(std::env::args_os()) {
		Some(args) => sandbox::probe::run_inside(&args),
		None => {
			eprintln!(
				"usage: probe-helper {} <report> <canary> <outside> <loopback-port>",
				sandbox::probe::ARG
			);
			std::process::exit(2);
		}
	}
}
