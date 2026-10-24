//! modlobby's helper inside the sandbox, on Linux: it takes the rules it is
//! handed, restricts itself with them, and becomes the command after them.
//! The same binary is the Wayland proxy outside the sandbox and the probe
//! inside it, so one small program -- no GTK, no WebKit -- is all the sandbox
//! has to load.

use std::ffi::OsString;
use std::path::PathBuf;

/// The argument that makes the helper restrict itself and exec a command.
pub const EXEC: &str = "--sandbox-exec";

/// The argument that makes the helper the Wayland proxy, outside the
/// sandbox, for as long as a game uses it.
pub const PROXY: &str = "--wayland-proxy";

/// What the process inside may do with the files and the network, as
/// Landlock is told it: a second layer under bwrap's mount namespace, which
/// already holds nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rules {
	/// Read only.
	pub read: Vec<PathBuf>,
	/// Read and execute.
	pub exec: Vec<PathBuf>,
	/// Everything.
	pub write: Vec<PathBuf>,
	/// Whether the game is online. TCP is refused either way.
	pub online: bool,
}

/// `rules` as the helper's arguments, after [`EXEC`].
pub fn rules_args(rules: &Rules) -> Vec<OsString> {
	let mut out = Vec::new();
	for (flag, paths) in [
		("--read", &rules.read),
		("--exec", &rules.exec),
		("--write", &rules.write),
	] {
		for path in paths {
			out.push(flag.into());
			out.push(path.as_os_str().to_owned());
		}
	}
	out.push("--net".into());
	out.push(if rules.online { "online" } else { "none" }.into());
	out
}

/// What the helper is asked to be, by its first argument alone: whatever
/// follows `--sandbox-exec`'s separator is the command it becomes, however
/// that command's own arguments read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
	Exec,
	Proxy,
	Probe,
	Unknown,
}

pub fn role(args: &[OsString]) -> Role {
	match args.get(1).and_then(|first| first.to_str()) {
		Some(EXEC) => Role::Exec,
		Some(PROXY) => Role::Proxy,
		Some(crate::probe::ARG) => Role::Probe,
		_ => Role::Unknown,
	}
}

/// The rules and the command from a command line that is ours: [`EXEC`]
/// first, the rules, `--`, and a command to become.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Option<(Rules, Vec<OsString>)> {
	let mut args = args.into_iter().skip(1);
	if args.next()? != EXEC {
		return None;
	}
	let mut rules = Rules::default();
	loop {
		let flag = args.next()?;
		let flag = flag.to_str()?;
		if flag == "--" {
			break;
		}
		let value = args.next()?;
		match flag {
			"--read" => rules.read.push(value.into()),
			"--exec" => rules.exec.push(value.into()),
			"--write" => rules.write.push(value.into()),
			"--net" => rules.online = value == "online",
			_ => return None,
		}
	}
	let command: Vec<OsString> = args.collect();
	(!command.is_empty()).then_some((rules, command))
}

/// The proxy's socket and the compositor's, from a command line that is
/// ours: [`PROXY`] first, `<listen> <upstream>`.
pub fn proxy_args(args: impl IntoIterator<Item = OsString>) -> Option<(PathBuf, PathBuf)> {
	let mut args = args.into_iter().skip(1);
	if args.next()? != PROXY {
		return None;
	}
	Some((args.next()?.into(), args.next()?.into()))
}

/// The helper's whole life, from its command line: the probe, a restricted
/// exec, or the proxy. Never returns.
pub fn main(args: Vec<OsString>) -> ! {
	match role(&args) {
		Role::Probe => {
			if let Some(probe) = crate::probe::arg(args.clone()) {
				crate::probe::run_inside(&probe);
				std::process::exit(0);
			}
		}
		#[cfg(target_os = "linux")]
		Role::Exec => {
			if let Some((rules, command)) = parse(args.clone()) {
				exec(rules, command);
			}
		}
		#[cfg(target_os = "linux")]
		Role::Proxy => {
			if let Some((listen, upstream)) = proxy_args(args.clone()) {
				match crate::wayland::serve(&listen, &upstream) {
					Ok(()) => std::process::exit(0),
					Err(err) => {
						eprintln!("the Wayland proxy: {err}");
						std::process::exit(1);
					}
				}
			}
		}
		_ => {}
	}
	eprintln!(
		"usage: {} <report> <canary> <outside> <port> [roots…] | {EXEC} <rules> -- <command…> | {PROXY} <listen> <upstream>",
		crate::probe::ARG
	);
	std::process::exit(2)
}

/// What [`restrict`] could hold on this kernel.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enforced {
	/// Landlock's own word on the ruleset: fully, partly or not enforced.
	pub landlock: String,
}

/// Restricts this process, and whatever it becomes, by `rules`: Landlock as
/// far as the kernel has it, then a seccomp filter that refuses TCP sockets,
/// new namespaces, mounts, tracing, the keyrings, BPF, module loading and
/// typing into the terminal. Landlock is best effort -- bwrap's mount
/// namespace already holds nothing else -- and the filter is not: `Err`
/// when it could not be installed.
#[cfg(target_os = "linux")]
pub fn restrict(rules: &Rules) -> Result<Enforced, String> {
	let landlock = landlock(rules).map_err(|err| format!("Landlock: {err}"))?;
	seccomp().map_err(|err| format!("seccomp: {err}"))?;
	Ok(Enforced { landlock })
}

#[cfg(target_os = "linux")]
fn landlock(rules: &Rules) -> Result<String, landlock::RulesetError> {
	use landlock::{
		ABI, Access, AccessFs, AccessNet, Ruleset, RulesetAttr, RulesetCreatedAttr, Scope,
		path_beneath_rules,
	};
	let abi = ABI::V6;
	let existing = |paths: &[PathBuf]| -> Vec<PathBuf> {
		paths.iter().filter(|path| path.exists()).cloned().collect()
	};
	let read_only = AccessFs::from_read(abi) & !AccessFs::Execute;
	let status = Ruleset::default()
		.handle_access(AccessFs::from_all(abi))?
		// No rule allows a port: no TCP bind or connect, where the kernel
		// has Landlock's network rights.
		.handle_access(AccessNet::from_all(abi))?
		.scope(Scope::AbstractUnixSocket | Scope::Signal)?
		.create()?
		.add_rules(path_beneath_rules(existing(&rules.read), read_only))?
		.add_rules(path_beneath_rules(
			existing(&rules.exec),
			AccessFs::from_read(abi),
		))?
		.add_rules(path_beneath_rules(
			existing(&rules.write),
			AccessFs::from_all(abi),
		))?
		.restrict_self()?;
	Ok(format!("{:?}", status.ruleset))
}

#[cfg(target_os = "linux")]
fn seccomp() -> Result<(), seccompiler::Error> {
	use std::collections::BTreeMap;

	use seccompiler::{
		SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
		SeccompRule, TargetArch,
	};

	let arch = TargetArch::try_from(std::env::consts::ARCH)?;
	let mut denied: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
	for call in [
		libc::SYS_unshare,
		libc::SYS_setns,
		libc::SYS_mount,
		libc::SYS_umount2,
		libc::SYS_pivot_root,
		libc::SYS_chroot,
		libc::SYS_open_tree,
		libc::SYS_move_mount,
		libc::SYS_fsopen,
		libc::SYS_fsconfig,
		libc::SYS_fsmount,
		libc::SYS_fspick,
		libc::SYS_ptrace,
		libc::SYS_process_vm_readv,
		libc::SYS_process_vm_writev,
		libc::SYS_keyctl,
		libc::SYS_add_key,
		libc::SYS_request_key,
		libc::SYS_bpf,
		libc::SYS_perf_event_open,
		libc::SYS_userfaultfd,
		libc::SYS_kexec_load,
		libc::SYS_init_module,
		libc::SYS_finit_module,
		libc::SYS_delete_module,
	] {
		denied.insert(call, Vec::new());
	}
	let arg = |index, len, op, value| SeccompCondition::new(index, len, op, value);
	use SeccompCmpArgLen::{Dword, Qword};
	use SeccompCmpOp::{Eq, MaskedEq};
	// TCP, either family: the engine plays over UDP, and Lua's sockets are
	// off in its config besides.
	let tcp = |family: i32| {
		SeccompRule::new(vec![
			arg(0, Dword, Eq, family as u64)?,
			arg(1, Dword, MaskedEq(0xf), libc::SOCK_STREAM as u64)?,
		])
	};
	denied.insert(
		libc::SYS_socket,
		vec![tcp(libc::AF_INET)?, tcp(libc::AF_INET6)?],
	);
	// A new user namespace is the one namespace a process without
	// capabilities may make.
	let new_user = libc::CLONE_NEWUSER as u64;
	denied.insert(
		libc::SYS_clone,
		vec![SeccompRule::new(vec![arg(
			0,
			Qword,
			MaskedEq(new_user),
			new_user,
		)?])?],
	);
	// Typing into the terminal, or the console's own controls.
	denied.insert(
		libc::SYS_ioctl,
		vec![
			SeccompRule::new(vec![arg(1, Dword, Eq, libc::TIOCSTI)?])?,
			SeccompRule::new(vec![arg(1, Dword, Eq, libc::TIOCLINUX)?])?,
		],
	);
	let refuse: seccompiler::BpfProgram = SeccompFilter::new(
		denied,
		SeccompAction::Allow,
		SeccompAction::Errno(libc::EACCES as u32),
		arch,
	)?
	.try_into()?;
	// `clone3` takes its flags behind a pointer no filter can read: it is
	// "not there", and libc falls back to `clone`, which the filter above
	// reads.
	let mut later = BTreeMap::new();
	later.insert(libc::SYS_clone3, Vec::new());
	let no_clone3: seccompiler::BpfProgram = SeccompFilter::new(
		later,
		SeccompAction::Allow,
		SeccompAction::Errno(libc::ENOSYS as u32),
		arch,
	)?
	.try_into()?;
	seccompiler::apply_filter(&refuse)?;
	seccompiler::apply_filter(&no_clone3)
}

/// Restricts this process by `rules` and becomes `command`; never returns.
/// Anything that fails on the way ends it with 126, the shell's word for a
/// command that could not run, and the reason on stderr.
#[cfg(target_os = "linux")]
pub fn exec(rules: Rules, command: Vec<OsString>) -> ! {
	use std::os::unix::process::CommandExt as _;

	if let Err(why) = restrict(&rules) {
		eprintln!("the sandbox could not restrict itself: {why}");
		std::process::exit(126);
	}
	let err = std::process::Command::new(&command[0])
		.args(&command[1..])
		.exec();
	eprintln!("the sandbox could not start {:?}: {err}", command[0]);
	std::process::exit(126)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rules_round_trip_through_their_arguments() {
		let rules = Rules {
			read: vec!["/cfg/mods-view/0".into(), "/data/games".into()],
			exec: vec!["/usr".into(), "/data/engine".into()],
			write: vec!["/cfg/mods-data".into(), "/tmp".into()],
			online: true,
		};
		let mut line: Vec<OsString> = vec!["modlobby-sandbox".into(), EXEC.into()];
		line.extend(rules_args(&rules));
		line.extend([
			"--".into(),
			"/data/engine/spring".into(),
			"--isolation".into(),
		]);
		assert_eq!(
			parse(line),
			Some((
				rules,
				vec!["/data/engine/spring".into(), "--isolation".into()]
			))
		);
	}

	#[test]
	fn a_command_line_without_the_separator_is_not_ours() {
		let line = |parts: &[&str]| parts.iter().map(OsString::from).collect::<Vec<_>>();
		assert_eq!(parse(line(&["x", EXEC, "--read", "/usr"])), None);
		assert_eq!(
			parse(line(&["x", EXEC, "--read", "/usr", "--"])),
			None,
			"no command"
		);
		assert_eq!(
			parse(line(&["x", EXEC, "--bogus", "1", "--", "true"])),
			None
		);
		assert_eq!(parse(line(&["x", "--read", "/usr", "--", "true"])), None);
		assert_eq!(
			proxy_args(line(&["x", PROXY, "/run/a", "/run/b"])),
			Some(("/run/a".into(), "/run/b".into()))
		);
	}

	/// The first argument alone decides what the helper is: a probe named
	/// after `--sandbox-exec`'s separator is the command to restrict, never
	/// a probe to run unrestricted.
	#[test]
	fn the_first_argument_decides_what_the_helper_is() {
		let line = |parts: &[&str]| parts.iter().map(OsString::from).collect::<Vec<_>>();
		let restricted = line(&[
			"modlobby-sandbox",
			EXEC,
			"--net",
			"online",
			"--",
			"/cfg/probe/modlobby-sandbox",
			crate::probe::ARG,
			"/w/report.txt",
			"/canary",
			"/outside",
			"5123",
		]);
		assert_eq!(role(&restricted), Role::Exec);
		assert_eq!(
			role(&line(&["x", crate::probe::ARG, "/r", "/c", "/o", "1"])),
			Role::Probe
		);
		assert_eq!(role(&line(&["x", PROXY, "/a", "/b"])), Role::Proxy);
		assert_eq!(role(&line(&["x", "--", EXEC])), Role::Unknown);
		assert_eq!(
			parse(line(&["x", "--read", "/usr", EXEC, "--", "true"])),
			None
		);
	}
}
