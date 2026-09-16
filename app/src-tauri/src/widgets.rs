//! What BAR players actually run, for the widget list.
//!
//! The numbers come from pve.bar's weekly projection over public replays —
//! aggregates over distinct players, never named, withheld below a
//! k-anonymity floor. See the `widgets` crate for what is published and why.

use tauri::State;

use crate::state::App;

/// The published usage document, or `None` when it could not be read.
///
/// Deliberately not a `Result`: usage decorates a widget list rather than
/// carrying it, so a page that renders without the numbers is a better outcome
/// than one that refuses to render. The state layer logs the reason.
#[tauri::command]
pub async fn widget_usage(app: State<'_, App>) -> Result<Option<widgets::Usage>, ()> {
    Ok(app.widget_usage().await)
}
