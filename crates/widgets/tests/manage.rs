//! Install, update and delete against a real server and a real directory.
//!
//! The properties worth holding: bytes are checked against the hash the game
//! itself would compute, a delete removes exactly what was installed and no
//! more, and nothing outside modlobby's own directory is written.

use std::path::Path;

use widgets::config::WidgetConfig;
use widgets::install::{Install, InstallFile, InstallKind};
use widgets::manage::{
    Ledger, ManageError, delete, hash_of, install, is_intact, read_config, residue_of,
    settings_keys, settings_since, write_config,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WIDGET: &[u8] = b"function widget:GetInfo() return { name = \"Ping Wheel\" } end\n";
const INCLUDE: &[u8] = b"return { helper = true }\n";

/// The same client the app uses: reqwest is built with `rustls-no-provider`,
/// so one built any other way panics on the first request.
fn client() -> reqwest::Client {
    content::http::client("test")
}

fn descriptor(base: &str, files: Vec<InstallFile>) -> Install {
    Install {
        kind: InstallKind::Github,
        url: files.first().map(|f| f.url.clone()).unwrap_or_default(),
        archive: false,
        page: format!("{base}/page"),
        license: "MIT".into(),
        permissive: true,
        reason: String::new(),
        files,
    }
}

async fn serve(server: &MockServer, at: &str, body: &'static [u8]) -> String {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(server)
        .await;
    format!("{}{at}", server.uri())
}

#[tokio::test]
async fn a_widget_is_downloaded_verified_and_written() {
    let server = MockServer::start().await;
    let url = serve(&server, "/gui_ping_wheel.lua", WIDGET).await;
    let dir = tempfile::tempdir().unwrap();

    let entry = install(
        &client(),
        "widget:gui_ping_wheel",
        "Ping Wheel",
        &descriptor(
            &server.uri(),
            vec![InstallFile {
                path: "Widgets/community/gui_ping_wheel/gui_ping_wheel.lua".into(),
                content_hash: hash_of(WIDGET),
                url,
            }],
        ),
        dir.path(),
        1000,
    )
    .await
    .expect("installs");

    assert_eq!(entry.files, vec!["LuaUI/Widgets/gui_ping_wheel.lua"]);
    let written = dir.path().join("LuaUI/Widgets/gui_ping_wheel.lua");
    assert_eq!(std::fs::read(&written).unwrap(), WIDGET);
}

#[tokio::test]
async fn the_upstream_directories_are_not_recreated_locally() {
    // BAR loads `LuaUI/Widgets/<name>.lua`. A repository keeps its widgets
    // several directories deep, and reproducing that would install a widget
    // somewhere the game never looks.
    let server = MockServer::start().await;
    let url = serve(&server, "/deep.lua", WIDGET).await;
    let dir = tempfile::tempdir().unwrap();

    let entry = install(
        &client(),
        "k",
        "W",
        &descriptor(
            &server.uri(),
            vec![InstallFile {
                path: "a/b/c/deep.lua".into(),
                content_hash: hash_of(WIDGET),
                url,
            }],
        ),
        dir.path(),
        1,
    )
    .await
    .unwrap();

    assert_eq!(entry.files, vec!["LuaUI/Widgets/deep.lua"]);
    assert!(!dir.path().join("a").exists());
}

#[tokio::test]
async fn a_path_that_tries_to_escape_is_not_written() {
    let server = MockServer::start().await;
    let url = serve(&server, "/evil.lua", WIDGET).await;
    let dir = tempfile::tempdir().unwrap();

    let outcome = install(
        &client(),
        "k",
        "W",
        &descriptor(
            &server.uri(),
            vec![InstallFile {
                path: "../../../../etc/evil.lua".into(),
                content_hash: hash_of(WIDGET),
                url,
            }],
        ),
        dir.path(),
        1,
    )
    .await
    .unwrap();

    // The traversal is stripped, not obeyed: only the file name survives.
    assert_eq!(outcome.files, vec!["LuaUI/Widgets/evil.lua"]);
    assert!(dir.path().join("LuaUI/Widgets/evil.lua").exists());
}

#[tokio::test]
async fn bytes_that_do_not_match_their_hash_are_not_written() {
    // These bytes come from somebody else's repository or a Discord post. The
    // hash is the only thing tying them to the widget whose numbers were read.
    let server = MockServer::start().await;
    let url = serve(&server, "/gui.lua", WIDGET).await;
    let dir = tempfile::tempdir().unwrap();

    let outcome = install(
        &client(),
        "k",
        "Ping Wheel",
        &descriptor(
            &server.uri(),
            vec![InstallFile {
                path: "gui.lua".into(),
                content_hash: hash_of(b"something else entirely"),
                url,
            }],
        ),
        dir.path(),
        1,
    )
    .await;

    assert!(matches!(outcome, Err(ManageError::Corrupt(_))));
    assert!(!dir.path().join("LuaUI/Widgets/gui.lua").exists());
}

#[tokio::test]
async fn a_windows_hashed_widget_still_verifies() {
    // BAR reads with `io.open(f, "r")`, so a Windows client collapses CRLF
    // before hashing while the server holds the raw bytes. Same widget.
    let crlf: Vec<u8> = b"line one\r\nline two\r\n".to_vec();
    let unix: Vec<u8> = b"line one\nline two\n".to_vec();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/crlf.lua"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(crlf.clone()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();

    let entry = install(
        &client(),
        "k",
        "W",
        &descriptor(
            &server.uri(),
            vec![InstallFile {
                path: "crlf.lua".into(),
                content_hash: hash_of(&unix),
                url: format!("{}/crlf.lua", server.uri()),
            }],
        ),
        dir.path(),
        1,
    )
    .await
    .expect("the text-mode hash is the same widget");

    // Stored exactly as served: the hash variant is how it was recognised, not
    // an instruction to rewrite somebody's line endings.
    assert_eq!(
        std::fs::read(dir.path().join(&entry.files[0])).unwrap(),
        crlf
    );
}

#[tokio::test]
async fn a_widget_travels_with_its_includes() {
    let server = MockServer::start().await;
    let main = serve(&server, "/gui_a.lua", WIDGET).await;
    let helper = serve(&server, "/util.lua", INCLUDE).await;
    let dir = tempfile::tempdir().unwrap();

    let entry = install(
        &client(),
        "k",
        "A",
        &descriptor(
            &server.uri(),
            vec![
                InstallFile {
                    path: "gui_a.lua".into(),
                    content_hash: hash_of(WIDGET),
                    url: main,
                },
                InstallFile {
                    path: "util.lua".into(),
                    content_hash: hash_of(INCLUDE),
                    url: helper,
                },
            ],
        ),
        dir.path(),
        1,
    )
    .await
    .unwrap();

    assert_eq!(entry.files.len(), 2);
    assert!(dir.path().join("LuaUI/Widgets/util.lua").exists());
}

#[tokio::test]
async fn a_widget_without_a_download_is_refused_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let withheld = Install {
        kind: InstallKind::Github,
        page: "https://github.com/o/r".into(),
        reason: "licence does not grant redistribution".into(),
        files: vec![InstallFile {
            path: "gui.lua".into(),
            content_hash: "abc".into(),
            url: String::new(),
        }],
        ..Install::default()
    };

    let outcome = install(&client(), "k", "W", &withheld, dir.path(), 1).await;
    let Err(ManageError::NotInstallable(_, why)) = outcome else {
        panic!("expected a refusal with a reason");
    };
    assert!(why.contains("licence"));
}

#[tokio::test]
async fn an_update_is_only_needed_when_the_published_files_change() {
    let server = MockServer::start().await;
    let url = serve(&server, "/gui.lua", WIDGET).await;
    let dir = tempfile::tempdir().unwrap();
    let published = descriptor(
        &server.uri(),
        vec![InstallFile {
            path: "gui.lua".into(),
            content_hash: hash_of(WIDGET),
            url,
        }],
    );

    let entry = install(&client(), "k", "W", &published, dir.path(), 1)
        .await
        .unwrap();
    assert!(!entry.is_outdated(&published));

    let mut moved_on = published.clone();
    moved_on.files[0].content_hash = hash_of(b"a newer revision");
    assert!(entry.is_outdated(&moved_on));
}

#[tokio::test]
async fn deleting_removes_the_files_and_the_config_entries() {
    let server = MockServer::start().await;
    let url = serve(&server, "/gui.lua", WIDGET).await;
    let dir = tempfile::tempdir().unwrap();
    let entry = install(
        &client(),
        "widget:gui",
        "Ping Wheel",
        &descriptor(
            &server.uri(),
            vec![InstallFile {
                path: "gui.lua".into(),
                content_hash: hash_of(WIDGET),
                url,
            }],
        ),
        dir.path(),
        1,
    )
    .await
    .unwrap();

    let mut config = WidgetConfig::parse(config_with("Ping Wheel"));
    let deleted = delete(&entry, dir.path(), &mut config, Vec::new()).unwrap();

    assert_eq!(deleted.files, vec!["LuaUI/Widgets/gui.lua"]);
    assert!(deleted.config.order_entry, "removed from order");
    assert!(deleted.config.settings, "removed its saved settings");
    assert!(!dir.path().join("LuaUI/Widgets/gui.lua").exists());
}

#[tokio::test]
async fn deleting_reports_what_it_deliberately_left_behind() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("springsettings.cfg"),
        "PingWheelSize = 12\nSomethingElse = 1\n",
    )
    .unwrap();

    // Nothing on disk says which widget wrote which key, so this is a guess —
    // which is exactly why it is reported rather than removed.
    let found = residue_of(&entry_named("PingWheel"), dir.path());
    assert!(found.iter().any(|said| said.contains("springsettings.cfg")));
    assert!(dir.path().join("springsettings.cfg").exists());
}

#[test]
fn a_setting_that_appeared_after_the_install_is_a_candidate() {
    // Weaker evidence than a name match, but wider: a widget that writes a key
    // named nothing like itself shows up here and nowhere else.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("springsettings.cfg"), "Existing = 1\n").unwrap();
    let mut entry = entry_named("Ping Wheel");
    entry.settings_before = settings_keys(dir.path());

    std::fs::write(
        dir.path().join("springsettings.cfg"),
        "Existing = 1\nSomethingNew = 4\n",
    )
    .unwrap();

    assert_eq!(settings_since(&entry, dir.path()), vec!["SomethingNew"]);
    assert!(
        residue_of(&entry, dir.path())
            .iter()
            .any(|said| said.contains("SomethingNew"))
    );
}

#[test]
fn a_settings_file_that_did_not_change_leaves_no_residue() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("springsettings.cfg"),
        "# a comment\nExisting = 1\n",
    )
    .unwrap();
    let mut entry = entry_named("Ping Wheel");
    entry.settings_before = settings_keys(dir.path());
    assert_eq!(settings_since(&entry, dir.path()), Vec::<String>::new());
    assert_eq!(residue_of(&entry, dir.path()), Vec::<String>::new());
}

#[tokio::test]
async fn an_install_records_the_settings_that_were_there_first() {
    let server = MockServer::start().await;
    let url = serve(&server, "/gui.lua", WIDGET).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("springsettings.cfg"), "Existing = 1\n").unwrap();

    let entry = install(
        &client(),
        "k",
        "W",
        &descriptor(
            &server.uri(),
            vec![InstallFile {
                path: "gui.lua".into(),
                content_hash: hash_of(WIDGET),
                url,
            }],
        ),
        dir.path(),
        1,
    )
    .await
    .unwrap();

    assert_eq!(entry.settings_before, vec!["Existing"]);
}

fn entry_named(name: &str) -> widgets::manage::InstalledWidget {
    widgets::manage::InstalledWidget {
        key: "k".into(),
        name: name.to_owned(),
        files: Vec::new(),
        hashes: Vec::new(),
        source: String::new(),
        installed_at: 1,
        settings_before: Vec::new(),
    }
}

#[test]
fn the_ledger_survives_a_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = Ledger::default();
    ledger.widgets.insert(
        "widget:gui".into(),
        widgets::manage::InstalledWidget {
            key: "widget:gui".into(),
            name: "Ping Wheel".into(),
            files: vec!["LuaUI/Widgets/gui.lua".into()],
            hashes: vec!["abc".into()],
            source: "https://example.test/gui.lua".into(),
            installed_at: 42,
            settings_before: vec!["Existing".into()],
        },
    );
    ledger.write(dir.path()).unwrap();
    assert_eq!(Ledger::read(dir.path()), ledger);
}

#[test]
fn an_unreadable_ledger_reads_as_empty_rather_than_failing() {
    // Claiming ownership of nothing is recoverable; refusing to render the
    // page is not.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("LuaUI")).unwrap();
    std::fs::write(dir.path().join(widgets::manage::LEDGER_FILE), "{ not json").unwrap();
    assert_eq!(Ledger::read(dir.path()), Ledger::default());
}

#[test]
fn a_widget_whose_files_were_removed_by_hand_is_not_intact() {
    let dir = tempfile::tempdir().unwrap();
    let entry = widgets::manage::InstalledWidget {
        key: "k".into(),
        name: "W".into(),
        files: vec!["LuaUI/Widgets/gone.lua".into()],
        hashes: vec!["abc".into()],
        source: String::new(),
        installed_at: 1,
        settings_before: Vec::new(),
    };
    assert!(!is_intact(&entry, dir.path()));
}

#[test]
fn the_config_is_written_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("LuaUI/Config/BYAR.lua");
    write_config(&path, "return {}\n").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "return {}\n");
    // No temporary left where BAR would try to read it.
    assert!(!path.with_extension("lua.modlobby-new").exists());
    assert!(read_config(dir.path()).is_some());
}

fn config_with(name: &str) -> String {
    format!(
        "-- Widget Custom data and order, order = 0 disabled widget\nreturn {{\n\tallowUserWidgets = true,\n\tdata = {{\n\t\t[\"{name}\"] = {{\n\t\t\tsize = 12,\n\t\t}},\n\t}},\n\torder = {{\n\t\t[\"{name}\"] = 5,\n\t}},\n}}\n"
    )
}

#[allow(dead_code)]
fn unused(_: &Path) {}
