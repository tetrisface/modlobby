//! Machine identity as Chobby reports it: the `hardware:*` telemetry
//! properties, the LOGIN `lobby_hash` (`"<macAddrHash> <sysInfoHash[..16]>"`)
//! and the telemetry `machine_hash`.

use spring_protocol::hash;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, RefreshKind, System};

#[derive(Debug, Clone)]
pub struct Hardware {
    pub properties: Vec<(String, String)>,
    pub lobby_hash: String,
    pub machine_hash: String,
}

impl Hardware {
    /// Fixed values for tests and headless runs.
    pub fn stub() -> Self {
        Self {
            properties: vec![("hardware:osinfo".into(), "test".into())],
            lobby_hash: "test test".into(),
            machine_hash: "test".into(),
        }
    }
}

pub fn detect() -> Hardware {
    // Only the CPU list and the RAM total are read. `new_all` would also walk
    // every process, disk and interface, which is what made this the slowest
    // step of opening the app.
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing())
            .with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    let os = format!(
        "{} {}",
        System::name().unwrap_or_default(),
        System::os_version().unwrap_or_default()
    );
    let cpu = system
        .cpus()
        .first()
        .map(|c| format!("{} x{}", c.brand().trim(), system.cpus().len()))
        .unwrap_or_default();
    let ram = format!("{} MB", system.total_memory() / (1024 * 1024));
    let mac_hash = hash::md5_base64(&primary_mac().unwrap_or_default());
    let sysinfo_hash = hash::md5_hex(&format!("{os}|{cpu}|{ram}"));
    let lobby_hash = format!("{mac_hash} {}", &sysinfo_hash[..16]);

    Hardware {
        properties: vec![
            ("hardware:osinfo".into(), os),
            ("hardware:cpuinfo".into(), cpu),
            ("hardware:raminfo".into(), ram),
            ("hardware:sysInfoHash".into(), sysinfo_hash),
            ("hardware:macAddrHash".into(), mac_hash),
        ],
        machine_hash: hash::md5_base64(&lobby_hash),
        lobby_hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hash is what the server tells one machine from another by, and
    /// it is Chobby's inputs — OS, CPU brand and count, RAM — so a cheaper
    /// refresh must read the same values a full one does. An empty brand or
    /// a zero RAM would quietly make every install look like a new machine.
    #[test]
    fn the_cheap_refresh_reads_what_the_full_one_reads() {
        let started = std::time::Instant::now();
        let full = System::new_all();
        let full_took = started.elapsed();
        let cpu = full
            .cpus()
            .first()
            .map(|c| format!("{} x{}", c.brand().trim(), full.cpus().len()))
            .unwrap_or_default();
        let ram = format!("{} MB", full.total_memory() / (1024 * 1024));

        let started = std::time::Instant::now();
        let detected = detect();
        // With `--nocapture`: how much the cheaper refresh saves on this box.
        println!("full refresh {full_took:?}, detect {:?}", started.elapsed());
        let property = |name: &str| -> &str {
            detected
                .properties
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
                .unwrap_or_default()
        };
        assert_eq!(property("hardware:cpuinfo"), cpu);
        assert_eq!(property("hardware:raminfo"), ram);
        assert!(!cpu.starts_with(" x"), "a CPU brand was read: {cpu:?}");
        assert!(!cpu.ends_with("x0"), "cores were counted: {cpu:?}");
        assert_ne!(ram, "0 MB", "RAM was read");
        assert!(!property("hardware:osinfo").trim().is_empty());
    }
}

/// First interface with a non-zero MAC, in name order so the choice is stable across runs.
fn primary_mac() -> Option<String> {
    let networks = Networks::new_with_refreshed_list();
    let mut candidates: Vec<(&String, String)> = networks
        .iter()
        .map(|(name, data)| (name, data.mac_address().to_string()))
        .filter(|(_, mac)| mac != "00:00:00:00:00:00")
        .collect();
    candidates.sort();
    candidates.into_iter().next().map(|(_, mac)| mac)
}
