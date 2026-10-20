#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
	// Inside the sandbox, modlobby is the probe that proves the boundary
	// before an engine is started in it; nothing of the app comes up for that.
	if let Some(args) = sandbox::probe::arg(std::env::args_os()) {
		sandbox::probe::run_inside(&args);
		return;
	}
	modlobby_app_lib::run()
}
