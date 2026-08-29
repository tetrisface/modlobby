//! Machine identity as Chobby reports it: the `hardware:*` telemetry
//! properties, the LOGIN `lobby_hash` (`"<macAddrHash> <sysInfoHash[..16]>"`)
//! and the telemetry `machine_hash`.

use spring_protocol::hash;
use sysinfo::{Networks, System};

#[derive(Debug, Clone)]
pub struct Hardware {
    pub properties: Vec<(String, String)>,
    pub lobby_hash: String,
    pub machine_hash: String,
}

pub fn detect() -> Hardware {
    let system = System::new_all();
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
