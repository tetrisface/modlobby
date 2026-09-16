//! The published document, as the pipeline actually writes it.
//!
//! The unit tests in the crate use hand-built JSON, which proves the parser but
//! not the contract: the shape is produced by a different repository, in
//! Python, on a weekly schedule. This fixture is a real `usage.json` trimmed to
//! a dozen widgets, so a change on either side that breaks the other fails here
//! rather than in a user's lobby.
//!
//! Regenerate with the resolve stage and trim; do not hand-edit.

use widgets::{Usage, DEFAULT_WINDOW, WINDOWS};

fn published() -> Usage {
    let raw = include_str!("usage.sample.json");
    serde_json::from_str(raw).expect("the published document parses")
}

#[test]
fn the_real_document_parses() {
    let usage = published();
    assert!(!usage.widgets.is_empty());
    assert!(!usage.generated_at.is_empty());
    assert!(!usage.policy_version.is_empty());
}

#[test]
fn every_window_the_pipeline_publishes_is_one_this_crate_knows() {
    for window in &published().windows {
        assert!(WINDOWS.contains(&window.as_str()), "unknown window {window}");
    }
}

#[test]
fn the_default_window_is_actually_published() {
    assert!(published().windows.iter().any(|w| w == DEFAULT_WINDOW));
}

#[test]
fn ranking_is_dense_and_starts_at_one() {
    let usage = published();
    let ranked = usage.ranked(DEFAULT_WINDOW);
    assert!(!ranked.is_empty());
    for (index, widget) in ranked.iter().enumerate() {
        let stats = widget.window(DEFAULT_WINDOW).expect("a ranked widget has the window");
        assert_eq!(stats.rank as usize, index + 1);
    }
}

#[test]
fn active_players_never_exceed_players() {
    for widget in &published().widgets {
        for (window, stats) in &widget.windows {
            assert!(
                stats.players_active <= stats.players,
                "{} {window}: {} active of {} players",
                widget.name,
                stats.players_active,
                stats.players
            );
        }
    }
}

#[test]
fn retention_is_a_fraction() {
    for widget in &published().widgets {
        for stats in widget.windows.values() {
            assert!((0.0..=1.0).contains(&stats.retention), "{}", widget.name);
        }
    }
}

#[test]
fn a_resolved_widget_carries_an_id_and_an_unresolved_one_does_not() {
    for widget in &published().widgets {
        assert_eq!(widget.resolved, !widget.id.is_empty(), "{}", widget.key);
        assert_eq!(widget.is_installable(), widget.resolved, "{}", widget.key);
    }
}

#[test]
fn every_widget_is_named_whether_or_not_it_resolved() {
    // The name comes from the widget's own GetInfo either way, which is what
    // lets the long tail appear on a page at all.
    for widget in &published().widgets {
        assert!(!widget.name.is_empty(), "{}", widget.key);
        assert!(!widget.author.is_empty(), "{}", widget.key);
    }
}

#[test]
fn a_partial_window_reports_partial_coverage() {
    // The pipeline backfills history a slice at a time. On a fortnight of data
    // the year window is honest about holding a fortnight.
    let usage = published();
    let widget = &usage.widgets[0];
    if let Some(year) = widget.window("365d") {
        if let Some(week) = widget.window("7d") {
            assert!(year.coverage <= week.coverage);
        }
    }
}

#[test]
fn every_widget_has_at_least_one_window_to_show() {
    for widget in &published().widgets {
        assert!(widget.window_or_fallback(DEFAULT_WINDOW).is_some(), "{}", widget.key);
    }
}
