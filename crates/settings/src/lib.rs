//! User settings the way VS Code does them: one JSONC file the user may edit
//! by hand (comments survive the app's own writes), reloaded live, described
//! by a JSON Schema for editor completion. Credentials never touch the file —
//! they live in the OS keyring behind [`CredentialStore`].

pub mod credentials;
pub mod file;
pub mod model;
pub mod watch;

pub use credentials::{CredentialError, CredentialStore, KeyringStore, MemoryStore};
pub use file::{Error, Store, config_dir};
pub use model::Settings;
pub use watch::SettingsEvent;
