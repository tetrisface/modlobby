//! Client-property telemetry, mirroring Chobby's `api_analytics.lua`.
//!
//! teiserver's automod grants `app_status: :accepted` — which unlocks
//! `s.battle.queue_status` and `s.battle.extra_data` — once a client has
//! uploaded `hardware:cpuinfo`; the other properties feed the smurf keys.

use base64::prelude::*;

/// Property names Chobby uploads once per session (`LateHWInfo`).
pub const HARDWARE_PROPERTIES: [&str; 6] = [
    "hardware:osinfo",
    "hardware:cpuinfo",
    "hardware:gpuinfo",
    "hardware:raminfo",
    "hardware:sysInfoHash",
    "hardware:macAddrHash",
];

/// `c.telemetry.update_client_property <name> <base64(value)> <machine_hash>`.
pub fn update_client_property(name: &str, value: &str, machine_hash: &str) -> String {
    let name = name.replace(' ', "_");
    format!(
        "c.telemetry.update_client_property {name} {} {machine_hash}",
        BASE64_STANDARD.encode(value)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_value_as_base64() {
        assert_eq!(
            update_client_property("hardware:cpuinfo", "AMD Ryzen", "MH=="),
            "c.telemetry.update_client_property hardware:cpuinfo QU1EIFJ5emVu MH=="
        );
    }
}
