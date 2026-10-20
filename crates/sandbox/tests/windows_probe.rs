//! The real container on the machine running the tests.
#![cfg(windows)]

use std::path::PathBuf;

use sandbox::{Network, Policy};

#[tokio::test]
async fn the_real_container_holds_on_this_machine() {
	let container: std::sync::Arc<dyn sandbox::Confinement> =
		std::sync::Arc::new(sandbox::windows::AppContainer::new().unwrap());
	let dir = tempfile::tempdir().unwrap();
	// Its own copy of the helper, so the grant covers one small directory
	// rather than the whole target tree.
	let bin = dir.path().join("bin");
	std::fs::create_dir(&bin).unwrap();
	let helper = bin.join("probe-helper.exe");
	std::fs::copy(env!("CARGO_BIN_EXE_probe-helper"), &helper).unwrap();
	let write = dir.path().join("mods");
	std::fs::create_dir(&write).unwrap();
	let policy = Policy {
		exe_dirs: vec![bin],
		roots: Vec::new(),
		write,
		network: Network::None,
		env: sandbox::scrubbed_env(std::env::vars_os()),
		grants_file: dir.path().join("grants.json"),
	};
	let cache = dir.path().join("probe.json");

	sandbox::probe::ensure(&container, &policy, &helper, &cache, "test", "0.0.0")
		.await
		.unwrap();

	assert!(cache.exists(), "a verdict that holds is cached");
	let _ = PathBuf::new();
}
