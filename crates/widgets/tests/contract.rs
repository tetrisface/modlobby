//! The published document, as the pipeline actually writes it.
//!
//! The unit tests in the crate use hand-built JSON, which proves the parser but
//! not the contract: the shape is produced by a different repository, in
//! Python, on a weekly schedule. This fixture is a real `usage.json` trimmed to
//! a dozen widgets, so a change on either side that breaks the other fails here
//! rather than in a user's lobby.
//!
//! Regenerate with the resolve stage and trim; do not hand-edit.

use widgets::{AUDIENCES, DEFAULT_AUDIENCE, DEFAULT_WINDOW, InstallKind, Usage, WINDOWS};

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
        assert!(
            WINDOWS.contains(&window.as_str()),
            "unknown window {window}"
        );
    }
}

#[test]
fn every_audience_the_pipeline_publishes_is_one_this_crate_knows() {
    for audience in &published().audiences {
        assert!(
            AUDIENCES.contains(&audience.as_str()),
            "unknown audience {audience}"
        );
    }
}

#[test]
fn the_combined_audience_is_always_published() {
    // The split can be withheld while a decoding change is still migrating
    // days; `all` is complete throughout, which is why it is the default.
    let usage = published();
    assert!(usage.audiences.iter().any(|a| a == DEFAULT_AUDIENCE));
    for widget in &usage.widgets {
        assert!(
            widget.windows.contains_key(DEFAULT_AUDIENCE),
            "{} has no combined view",
            widget.key
        );
    }
}

#[test]
fn the_default_window_is_actually_published() {
    assert!(published().windows.iter().any(|w| w == DEFAULT_WINDOW));
}

#[test]
fn ranking_follows_the_published_rank() {
    // Density is not checked here and cannot be: this fixture is a trimmed
    // dozen widgets chosen to cover every install case, so its ranks are a
    // subset of the real document's. What a subset can prove is that sorting
    // by the published rank is strictly ordered — which is what the page does.
    let usage = published();
    for audience in &usage.audiences {
        let ranked = usage.ranked(audience, DEFAULT_WINDOW);
        assert!(!ranked.is_empty(), "{audience} ranks nothing");
        let ranks: Vec<u32> = ranked
            .iter()
            .map(|widget| {
                widget
                    .window(audience, DEFAULT_WINDOW)
                    .expect("a ranked widget has the window")
                    .rank
            })
            .collect();
        assert!(
            ranks.windows(2).all(|pair| pair[0] < pair[1]),
            "{audience} ranks are not strictly ascending: {ranks:?}"
        );
        assert!(ranks[0] >= 1, "ranks start at one");
    }
}

#[test]
fn active_players_never_exceed_players() {
    for widget in &published().widgets {
        for (audience, windows) in &widget.windows {
            for (window, stats) in windows {
                assert!(
                    stats.players_active <= stats.players,
                    "{} {audience}/{window}: {} active of {} players",
                    widget.name,
                    stats.players_active,
                    stats.players
                );
            }
        }
    }
}

#[test]
fn retention_is_a_fraction() {
    for widget in &published().widgets {
        for stats in widget.windows.values().flat_map(|windows| windows.values()) {
            assert!((0.0..=1.0).contains(&stats.retention), "{}", widget.name);
        }
    }
}

#[test]
fn a_resolved_widget_carries_an_id_and_an_unresolved_one_does_not() {
    for widget in &published().widgets {
        assert_eq!(widget.resolved, !widget.id.is_empty(), "{}", widget.key);
    }
}

#[test]
fn only_a_resolved_widget_can_be_installed() {
    // The converse does not hold, and that is the point of the licence gate: a
    // widget can be traced to a real source and still have no download.
    for widget in &published().widgets {
        if widget.is_installable() {
            assert!(
                widget.resolved,
                "{} offers a download unresolved",
                widget.key
            );
        }
    }
}

#[test]
fn a_widget_with_no_download_always_says_why() {
    // A button that does nothing and explains nothing is worse than no button.
    for widget in &published().widgets {
        if !widget.is_installable() {
            let why = widget.install.unavailable_because();
            assert!(why.is_some_and(|why| !why.is_empty()), "{}", widget.key);
        }
    }
}

#[test]
fn an_unresolved_widget_has_no_source_and_no_link() {
    for widget in &published().widgets {
        if !widget.resolved {
            assert_eq!(widget.install.kind, InstallKind::None, "{}", widget.key);
            assert!(!widget.install.is_linkable(), "{}", widget.key);
        }
    }
}

#[test]
fn a_download_is_always_accompanied_by_something_to_check_it_against() {
    for widget in &published().widgets {
        if !widget.install.is_installable() {
            continue;
        }
        assert!(
            widget.install.permissive,
            "{} serves without a grant",
            widget.key
        );
        for file in &widget.install.files {
            assert!(
                !file.content_hash.is_empty(),
                "{} {}",
                widget.key,
                file.path
            );
            assert!(
                file.install_path().is_some(),
                "{} {}",
                widget.key,
                file.path
            );
        }
    }
}

#[test]
fn a_withheld_widget_keeps_its_link_and_its_hashes() {
    let usage = published();
    let withheld: Vec<_> = usage
        .widgets
        .iter()
        .filter(|w| w.resolved && !w.install.is_installable())
        .collect();
    for widget in &withheld {
        assert!(widget.install.is_linkable(), "{} lost its link", widget.key);
        assert!(
            widget.install.files.iter().all(|f| f.url.is_empty()),
            "{} kept a download url it should not have",
            widget.key
        );
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
    if let Some(year) = widget.window(DEFAULT_AUDIENCE, "365d")
        && let Some(week) = widget.window(DEFAULT_AUDIENCE, "7d")
    {
        assert!(year.coverage <= week.coverage);
    }
}

#[test]
fn every_widget_has_at_least_one_window_to_show() {
    for widget in &published().widgets {
        assert!(
            widget
                .window_or_fallback(DEFAULT_AUDIENCE, DEFAULT_WINDOW)
                .is_some(),
            "{}",
            widget.key
        );
    }
}

#[test]
fn a_picture_is_an_absolute_https_url_or_nothing() {
    // The thumbnail scheme fetches whatever this names, so it has to be a
    // real address -- and a widget without one is drawn with initials instead.
    let usage = published();
    assert!(usage.widgets.iter().any(|widget| !widget.image.is_empty()));
    for widget in &usage.widgets {
        assert!(
            widget.image.is_empty() || widget.image.starts_with("https://"),
            "{}: {}",
            widget.key,
            widget.image
        );
    }
}

#[test]
fn a_discord_link_is_built_from_ids_not_a_thread_title() {
    for widget in &published().widgets {
        if widget.install.kind != InstallKind::Discord {
            continue;
        }
        let tail = widget
            .install
            .page
            .trim_start_matches("https://discord.com/channels/");
        assert!(
            tail.split('/')
                .all(|part| part.chars().all(|c| c.is_ascii_digit())),
            "{}: {}",
            widget.key,
            widget.install.page
        );
    }
}

#[test]
fn the_published_shape_is_the_one_this_crate_reads() {
    // A client that quietly parses an older document shows numbers whose
    // meaning has changed underneath it, which is worse than refusing.
    assert_eq!(published().document_version, widgets::DOCUMENT_VERSION);
}

#[test]
fn every_widget_has_forks_and_names_one_of_them_as_its_main() {
    // The collapsed row's author, picture and install all come from the main
    // fork, so a row whose main names nothing would render blank. A family
    // nobody's file could be traced to is the exception: it has no publisher to
    // be main, only the "other versions" line, and the row falls back to it.
    for widget in &published().widgets {
        assert!(!widget.forks.is_empty(), "{} has no forks", widget.key);
        if widget.main.is_empty() {
            assert!(!widget.resolved, "{} resolved but names no main", widget.key);
            assert!(
                widget.forks.iter().all(|fork| fork.kind == widgets::ForkKind::Other),
                "{} has a publisher but names no main",
                widget.key
            );
            continue;
        }
        assert!(
            widget.forks.iter().any(|fork| fork.key == widget.main),
            "{}: main {} is not among its forks",
            widget.key,
            widget.main
        );
        assert_eq!(
            widget.forks.iter().filter(|fork| fork.main).count(),
            1,
            "{} does not have exactly one main fork",
            widget.key
        );
    }
}

#[test]
fn still_using_is_never_more_than_used_once() {
    // The two metrics the page switches between. A player counted as still
    // using must have been counted as having used it, in every slice, for rows
    // and forks alike -- the toggle would otherwise show a number going up
    // when the reader asks for the looser measure.
    let usage = published();
    for widget in &usage.widgets {
        for (audience, windows) in &widget.windows {
            for (window, stats) in windows {
                assert!(
                    stats.players_still_using <= stats.players_active,
                    "{} {audience}/{window}: {} still using of {} active",
                    widget.name,
                    stats.players_still_using,
                    stats.players_active
                );
            }
        }
        for fork in &widget.forks {
            for (audience, windows) in &fork.windows {
                for (window, stats) in windows {
                    assert!(
                        stats.players_still_using <= stats.players_active
                            && stats.players_active <= stats.players,
                        "{} fork {} {audience}/{window}",
                        widget.name,
                        fork.key
                    );
                }
            }
        }
    }
}

#[test]
fn a_withheld_fork_carries_no_numbers() {
    // Below the anonymity floor the fork is still listed, because its source is
    // public -- but nothing about the players on it may be.
    for widget in &published().widgets {
        for fork in &widget.forks {
            for stats in fork.windows.values().flat_map(|windows| windows.values()) {
                if !stats.withheld {
                    continue;
                }
                assert_eq!(stats.players, 0, "{} {}", widget.name, fork.key);
                assert_eq!(stats.players_active, 0, "{} {}", widget.name, fork.key);
                assert_eq!(stats.players_still_using, 0, "{} {}", widget.name, fork.key);
                assert_eq!(stats.sightings, 0, "{} {}", widget.name, fork.key);
            }
        }
    }
}

#[test]
fn an_other_versions_fork_never_offers_a_download() {
    // "Other versions" is whatever players ran that matches no known source,
    // including their own edits. There is nothing to install and nobody to name.
    for widget in &published().widgets {
        for fork in widget.forks.iter().filter(|f| f.kind == widgets::ForkKind::Other) {
            assert!(!fork.install.is_installable(), "{}", widget.key);
            assert!(!fork.main, "{}: other versions cannot be the main fork", widget.key);
        }
    }
}

#[test]
fn every_fork_file_lands_somewhere_safe() {
    // The install path comes from another repository and is joined onto the
    // player's data directory, so the crate validates it rather than trusting it.
    for widget in &published().widgets {
        for fork in &widget.forks {
            for file in &fork.install.files {
                assert!(
                    file.install_path().is_some(),
                    "{} {} refuses {}",
                    widget.key,
                    fork.key,
                    file.path
                );
            }
        }
    }
}

#[test]
fn a_helper_file_is_never_published_as_a_widget() {
    // BAR registers a GetInfo-less .lua under its own file name with no author.
    // Those are parts of widgets, not widgets, and the pipeline drops them.
    for widget in &published().widgets {
        assert!(
            !(widget.name.to_lowercase().ends_with(".lua") && widget.author == "unknown"),
            "{} is a helper file",
            widget.name
        );
    }
}
