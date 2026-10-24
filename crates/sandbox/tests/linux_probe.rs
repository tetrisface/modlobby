//! The real bubblewrap on the machine running the tests, under its Wayland
//! session: the probe must hold, and the helper must restrict what it
//! becomes.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use sandbox::{Confinement, Network, Policy};

/// A directory under the home, as modlobby's own data is: a temporary one
/// under `/tmp` would sit in the sandbox's own writable `/tmp`.
fn scratch() -> tempfile::TempDir {
	let home = std::env::var_os("HOME").expect("HOME");
	tempfile::Builder::new()
		.prefix(".sandbox-test-")
		.tempdir_in(home)
		.unwrap()
}

fn helper() -> PathBuf {
	PathBuf::from(env!("CARGO_BIN_EXE_probe-helper"))
}

fn policy_in(dir: &Path, network: Network) -> Policy {
	let bin = dir.join("bin");
	std::fs::create_dir_all(&bin).unwrap();
	let write = dir.join("mods");
	std::fs::create_dir_all(&write).unwrap();
	Policy {
		exe_dirs: vec![bin],
		roots: Vec::new(),
		write,
		network,
		env: sandbox::scrubbed_env(std::env::vars_os()),
		grants_file: dir.join("grants.json"),
	}
}

async fn holds(network: Network) {
	let dir = scratch();
	let bubblewrap: Arc<dyn Confinement> = Arc::new(sandbox::linux::Bubblewrap::new(helper()));
	assert_eq!(
		bubblewrap.unavailable(),
		None,
		"this machine has a Wayland session and bwrap"
	);
	let policy = policy_in(dir.path(), network);
	let cache = dir.path().join("probe.json");
	let held =
		sandbox::probe::ensure(&bubblewrap, &policy, &helper(), &cache, "test", "0.0.0").await;
	let report = std::fs::read_to_string(policy.write.join("probe-report.txt")).unwrap_or_default();
	assert_eq!(held, Ok(()), "the report:\n{report}");
	assert!(cache.exists(), "a verdict that holds is cached");
}

#[tokio::test]
async fn the_real_bubblewrap_holds_online() {
	holds(Network::Outbound).await;
}

#[tokio::test]
async fn the_real_bubblewrap_holds_offline() {
	holds(Network::None).await;
}

/// The helper restricts itself before it becomes the command: Landlock
/// keeps writes to the write directory, and seccomp refuses TCP sockets and
/// new user namespaces, while UDP stays.
#[test]
fn the_helper_restricts_what_it_becomes() {
	let dir = scratch();
	let inside = dir.path().join("inside");
	std::fs::create_dir_all(&inside).unwrap();
	let outside = dir.path().join("outside.txt");
	let script = format!(
		"echo x > {inside}/ok; \
		 (echo y > {outside} && echo wrote-outside) || echo refused-outside; \
		 python3 -c 'import socket; socket.socket(socket.AF_INET, socket.SOCK_STREAM)' 2>/dev/null && echo tcp-open || echo tcp-refused; \
		 python3 -c 'import socket; socket.socket(socket.AF_INET, socket.SOCK_DGRAM)' 2>/dev/null && echo udp-open || echo udp-refused; \
		 unshare -U true 2>/dev/null && echo userns-made || echo userns-refused",
		inside = inside.display(),
		outside = outside.display(),
	);
	let out = Command::new(helper())
		.arg(sandbox::inside::EXEC)
		.args(sandbox::inside::rules_args(&sandbox::inside::Rules {
			read: vec!["/etc".into(), "/proc".into(), "/dev".into()],
			exec: vec!["/usr".into()],
			write: vec![inside.clone(), "/dev/null".into()],
			online: true,
		}))
		.args(["--", "/bin/sh", "-c", &script])
		.output()
		.unwrap();
	let said = String::from_utf8_lossy(&out.stdout);
	let complained = String::from_utf8_lossy(&out.stderr);
	assert!(out.status.success(), "{said}\n{complained}");
	assert!(inside.join("ok").is_file(), "{said}\n{complained}");
	assert!(said.contains("refused-outside"), "{said}");
	assert!(!outside.exists());
	assert!(said.contains("tcp-refused"), "{said}");
	assert!(said.contains("udp-open"), "{said}");
	assert!(said.contains("userns-refused"), "{said}");
}
