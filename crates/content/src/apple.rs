//! The one engine Beyond All Reason does not publish.
//!
//! Every other platform's engine comes from BAR's own file index, which is
//! where [`release`](crate::release) asks. There is no Apple build in it, and
//! there is not going to be: the port is one person's, it is not affiliated
//! with either project, and its author turns online play off at the build
//! level precisely so it cannot appear on the community servers.
//!
//! That left macOS with an install by hand — find the releases page, download
//! a `.dmg`, drag an app into a directory the lobby names — for a build that is
//! ad-hoc signed and so needs a right-click and Open to get past Gatekeeper the
//! first time. Four steps, each of which a person can do wrong, standing
//! between a Mac and a game against AI.
//!
//! So modlobby fetches it, from the same releases the page offers and with the
//! checksum GitHub publishes beside each asset. Two things make that a smaller
//! thing to do than it sounds:
//!
//! - The `.zip` asset holds the `.app` directly, so unpacking it is this file's
//!   own [`unpack`] and needs no `hdiutil`, no mounted volume and no
//!   privileges. (The `.dmg` beside it is the same bundle for a person who
//!   would rather do it by hand.)
//! - An archive this process downloads and extracts itself is never marked
//!   `com.apple.quarantine` — that attribute is put on by whatever *browser*
//!   saved the file — so the right-click dance does not apply to it. The
//!   engine is spawned directly rather than through LaunchServices, which is
//!   the other half of the same fact.
//!
//! What it does *not* do is decide the engine version. BAR's index is asked for
//! a named version because every version is in it; a port release carries the
//! one engine its author built against, and which one that is can only be read
//! out of the bundle afterwards ([`recoil::EngineLayout::declared_version`]).
//! The request here is therefore "the current Apple build", and the answer says
//! which engine that turned out to be.

use serde::Deserialize;

/// Whose build this is. Named rather than described, because a person
/// installing a third-party binary should be able to go and look at it.
pub const REPO: &str = "Vandomas/RecoilEngine-AppleSilicon";

/// The releases page, for somebody who would rather do it by hand or read the
/// notes first.
pub const RELEASES_PAGE: &str = "https://github.com/Vandomas/RecoilEngine-AppleSilicon/releases";

/// The current release, as JSON. `latest` skips prereleases and drafts, which
/// is what a release page's "Latest" badge means and what an automatic install
/// should take.
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/Vandomas/RecoilEngine-AppleSilicon/releases/latest";

/// Whether this machine is the one the port is built for.
///
/// Apple Silicon only: the author publishes no Intel build, and there is no
/// second-best answer to fall back on the way `engine_linux64` would be wrong
/// rather than absent. A `const fn` so the whole path compiles away everywhere
/// else and so [`crate::release::source`] can be one too.
pub const fn supported() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

/// What this build is, in one sentence, for wherever it is offered.
///
/// It says the three things somebody agreeing to the download should know and
/// could not work out from a progress bar: whose it is, that it is not BAR's,
/// and the one capability it does not have. Written once here so the room, the
/// download button and the error all say the same thing.
pub const PROVENANCE: &str = "Beyond All Reason publishes no macOS engine. This is the unofficial Apple Silicon build by Vandomas, which is not affiliated with Beyond All Reason or Recoil and has online play turned off at the build level: skirmish against AI, replays and LAN games work, the community servers do not.";

/// Why a Mac that is not Apple Silicon gets nothing.
pub const NO_INTEL_BUILD: &str = "Beyond All Reason publishes no macOS engine, and the unofficial Apple Silicon build is the only one that exists -- there is no Intel build of it. An Intel Mac cannot run Beyond All Reason natively.";

/// Why no engine could be fetched when the releases held none this can use.
pub const NO_ASSET: &str = "the current Apple Silicon release publishes no .zip to install; the .dmg beside it can be opened by hand from the releases page";

/// The asset to fetch: a bundle in a zip, with the digest GitHub took of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    /// The port's own version, which is what the release is tagged with
    /// (`v0.15.1`). Not the engine version -- see the module note.
    pub port_version: String,
    pub filename: String,
    pub url: String,
    /// Bytes, for a progress bar that means something.
    pub size: u64,
    /// Lowercase hex, without the `sha256:` GitHub prefixes it with.
    pub sha256: Option<String>,
}

impl Build {
    /// What to call the directory it installs into, once the bundle has said
    /// which engine it holds.
    ///
    /// `recoil_<version>`, which is what the BAR launcher names an engine
    /// directory and therefore what [`recoil::find_engine`] finds by name. A
    /// bundle also declares its version from the inside, so a directory named
    /// after the port instead -- the fallback, for a build that stops writing
    /// `EngineVersion` -- is still found; the name is for a person reading the
    /// folder.
    pub fn install_name(&self, engine_version: Option<&str>) -> String {
        match engine_version {
            Some(version) => format!("recoil_{version}"),
            None => format!("BAR-Launcher-{}", self.port_version.trim_start_matches('v')),
        }
    }
}

/// One asset on a release. The fields not read here are ignored rather than
/// fatal; the API carries a great many and adds more.
#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    /// `sha256:<hex>` since 2025; absent on assets uploaded before that.
    #[serde(default)]
    digest: Option<String>,
    /// Anything but `uploaded` is an asset still arriving or gone wrong.
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

/// The build to fetch out of one release's JSON, or `None` when it has none.
///
/// The `.zip` and not the `.dmg`: both hold the same bundle, and one of them
/// can be unpacked by a library while the other needs a disk image mounted.
/// An asset that is not finished uploading is skipped rather than fetched
/// half-written.
pub fn pick(body: &str) -> Option<Build> {
    let release: GithubRelease = serde_json::from_str(body).ok()?;
    let asset = release
        .assets
        .into_iter()
        .filter(|asset| asset.state.as_deref().unwrap_or("uploaded") == "uploaded")
        .find(|asset| asset.name.to_ascii_lowercase().ends_with(".zip"))?;
    Some(Build {
        port_version: release.tag_name,
        filename: asset.name,
        url: asset.browser_download_url,
        size: asset.size,
        sha256: asset
            .digest
            .as_deref()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .map(|hex| hex.trim().to_ascii_lowercase())
            .filter(|hex| !hex.is_empty()),
    })
}

/// Why an archive could not be unpacked.
#[derive(Debug, thiserror::Error)]
pub enum UnpackError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("the archive is not a readable zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// An entry naming a path outside the directory being unpacked into.
    /// Refused rather than clamped: an archive doing this is not one to take
    /// the rest of on trust either.
    #[error("the archive holds an entry that would write outside it: {0}")]
    Escapes(String),
    #[error("the archive holds no application bundle with {0} in it")]
    NoEngine(&'static str),
}

/// Unpacks the release zip into `into`, and answers where the engine ended up.
///
/// Three things this does that a plain extract loop does not:
///
/// - **Refuses an entry that would write outside `into`.** An absolute path or
///   a `..` in a zip is the oldest trick there is, and this archive is fetched
///   over the network from a third party.
/// - **Keeps the executable bit.** A zip records Unix modes and this one uses
///   them: `spring`, `pr-downloader` and the launcher's helpers arrive at 0755
///   and a file that loses it cannot be run. (The 7z an engine from BAR's index
///   arrives in carries Windows attributes only, which is why the other path
///   has `recoil::mark_executable` instead.)
/// - **Leaves out `__MACOSX/`.** The resource-fork shadow tree Apple's own
///   archiver writes: `._`-prefixed files mirroring every real one. Nothing
///   reads them here, and extracting them would put a `._spring` beside the
///   engine for every scan to step over.
///
/// The bundle's code signature survives all of this, since it is over the file
/// contents, and the engine is spawned directly rather than through
/// LaunchServices in any case.
pub fn unpack(archive: &std::path::Path, into: &std::path::Path) -> Result<(), UnpackError> {
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(archive)?))?;
    std::fs::create_dir_all(into)?;
    // Symlinks are followed when checking containment, so `into` has to be the
    // real path or a link on the way to it would make every check wrong.
    let root = into.canonicalize()?;

    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let Some(name) = entry.enclosed_name() else {
            return Err(UnpackError::Escapes(entry.name().to_owned()));
        };
        if skipped(&name) {
            continue;
        }
        let target = root.join(&name);
        // `enclosed_name` has already refused `..` and absolute paths; this is
        // the same question asked of the path that will actually be written,
        // which is what a symlink unpacked earlier could have moved.
        if !target.starts_with(&root) {
            return Err(UnpackError::Escapes(entry.name().to_owned()));
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        if entry.is_symlink() {
            symlink(&mut entry, &target, &root)?;
            continue;
        }
        let mut file = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut file)?;
        drop(file);
        set_mode(&target, entry.unix_mode())?;
    }

    recoil::EngineLayout::at(into)
        .map(|_| ())
        .ok_or(UnpackError::NoEngine(recoil::ENGINE_BINARY))
}

/// Entries that are Apple's archiver talking to itself rather than part of the
/// bundle.
fn skipped(name: &std::path::Path) -> bool {
    let mut parts = name.components().filter_map(|part| match part {
        std::path::Component::Normal(part) => part.to_str(),
        _ => None,
    });
    parts.next() == Some("__MACOSX")
        || name
            .file_name()
            .and_then(|file| file.to_str())
            .is_some_and(|file| file == ".DS_Store")
}

/// Recreates a symlink the archive carried, when it stays inside the bundle.
///
/// The releases so far have none -- the bundle is a flat tree of real files --
/// but a framework laid out the usual Apple way is full of them, and an
/// archive that starts carrying them should keep working rather than arrive as
/// files holding link text. A link pointing out of the tree is the escape this
/// whole function exists to not perform, so it is refused like any other.
#[cfg(unix)]
fn symlink(
    entry: &mut zip::read::ZipFile<'_, impl std::io::Read>,
    target: &std::path::Path,
    root: &std::path::Path,
) -> Result<(), UnpackError> {
    use std::io::Read as _;
    let mut points_at = String::new();
    entry.read_to_string(&mut points_at)?;
    let resolved = target
        .parent()
        .unwrap_or(root)
        .join(std::path::Path::new(&points_at));
    if !normalise(&resolved).starts_with(root) {
        return Err(UnpackError::Escapes(points_at));
    }
    let _ = std::fs::remove_file(target);
    std::os::unix::fs::symlink(&points_at, target)?;
    Ok(())
}

/// A path with its `.` and `..` resolved textually.
///
/// `canonicalize` is the usual answer and is the wrong one here: the path being
/// checked does not exist yet, and the link it would create is exactly what
/// must not be followed while deciding whether to create it.
#[cfg(unix)]
fn normalise(path: &std::path::Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Gives a file back the permissions the archive recorded for it.
///
/// Only the executable bits, and only when they were set: the rest of a mode
/// out of an archive is not worth honouring -- a world-writable or setuid file
/// unpacked out of a download is a hole, not a feature -- while the executable
/// bit is the one that decides whether the engine can be started at all.
#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: Option<u32>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if mode.is_none_or(|mode| mode & 0o111 == 0) {
        return Ok(());
    }
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    std::fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn set_mode(_path: &std::path::Path, _mode: Option<u32>) -> std::io::Result<()> {
    Ok(())
}

/// Takes `com.apple.quarantine` off an unpacked bundle, if it is there.
///
/// It should not be: the attribute is applied by whatever *downloads* a file,
/// and modlobby is not a browser. But an app can inherit
/// `LSFileQuarantineEnabled` from its own bundle, and the cost of being wrong
/// about it is the one failure this whole module exists to remove -- an
/// ad-hoc-signed build that Gatekeeper refuses with a dialog about an
/// unidentified developer. So it is cleared rather than assumed absent.
///
/// Best effort by design: a machine with no `xattr` is a machine with no
/// quarantine either, and a failure here must not fail an install that is
/// otherwise complete. Only ever called on a directory this process just wrote.
#[cfg(target_os = "macos")]
pub fn clear_quarantine(path: &std::path::Path) {
    match std::process::Command::new("/usr/bin/xattr")
        .arg("-dr")
        .arg("com.apple.quarantine")
        .arg(path)
        .output()
    {
        Ok(done) if done.status.success() => {}
        Ok(done) => tracing::debug!(
            status = ?done.status.code(),
            "xattr did not clear the quarantine attribute; it was most likely never set"
        ),
        Err(err) => tracing::debug!(%err, "no xattr to clear the quarantine attribute with"),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn clear_quarantine(_path: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// The shape the API actually answers with, trimmed to the fields read.
    const RELEASE: &str = r#"{
      "tag_name": "v0.15.1",
      "prerelease": false,
      "assets": [
        {
          "name": "BAR-Launcher-v0.15.1.dmg",
          "content_type": "application/x-apple-diskimage",
          "state": "uploaded",
          "size": 77932596,
          "digest": "sha256:E8EDCF0659A676D5D730CA9009A7CA02C7BFCFAAA23E397E54E803255343DAE0",
          "browser_download_url": "https://github.com/Vandomas/RecoilEngine-AppleSilicon/releases/download/v0.15.1/BAR-Launcher-v0.15.1.dmg"
        },
        {
          "name": "BAR-Launcher-v0.15.1.zip",
          "content_type": "application/zip",
          "state": "uploaded",
          "size": 75353432,
          "digest": "sha256:14a856fa9ede0d7b590482e76099955c9760cb1d6aac11274165a4731d343b70",
          "browser_download_url": "https://github.com/Vandomas/RecoilEngine-AppleSilicon/releases/download/v0.15.1/BAR-Launcher-v0.15.1.zip"
        }
      ]
    }"#;

    #[test]
    fn the_zip_is_taken_and_the_disk_image_left() {
        let build = pick(RELEASE).expect("a build");
        assert_eq!(build.port_version, "v0.15.1");
        assert_eq!(build.filename, "BAR-Launcher-v0.15.1.zip");
        assert!(build.url.ends_with("BAR-Launcher-v0.15.1.zip"));
        assert_eq!(build.size, 75_353_432);
        // The prefix is dropped and the case is the one a comparison expects.
        assert_eq!(
            build.sha256.as_deref(),
            Some("14a856fa9ede0d7b590482e76099955c9760cb1d6aac11274165a4731d343b70")
        );
    }

    #[test]
    fn a_release_with_nothing_to_unpack_is_an_answer_rather_than_a_failure() {
        assert!(pick(r#"{"tag_name":"v1","assets":[]}"#).is_none());
        assert!(pick("not json").is_none());
        // Still uploading is not something to fetch half of.
        assert!(
            pick(r#"{"tag_name":"v1","assets":[{"name":"a.zip","browser_download_url":"u","state":"starter"}]}"#)
                .is_none()
        );
        // An older asset with no digest is fetched, and says it has none.
        let build =
            pick(r#"{"tag_name":"v1","assets":[{"name":"a.zip","browser_download_url":"u"}]}"#)
                .expect("a build");
        assert_eq!(build.sha256, None);
    }

    #[test]
    fn the_directory_is_named_after_the_engine_the_bundle_declares() {
        let build = pick(RELEASE).unwrap();
        assert_eq!(build.install_name(Some("2026.07.04")), "recoil_2026.07.04");
        // A bundle that says nothing about itself still gets a name a person
        // can read, and is found by its contents rather than by it.
        assert_eq!(build.install_name(None), "BAR-Launcher-0.15.1");
    }

    /// Writes a zip shaped like the release: the bundle, the shadow tree Apple
    /// puts beside it, and an engine binary that has to come out executable.
    fn release_zip(path: &std::path::Path, extra: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let plain: zip::write::SimpleFileOptions =
            zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        let runnable = plain.unix_permissions(0o755);

        zip.start_file("BAR Launcher.app/Contents/Info.plist", plain)
            .unwrap();
        zip.write_all(
            b"<plist><dict><key>EngineVersion</key><string>2026.07.04</string></dict></plist>",
        )
        .unwrap();
        zip.start_file(
            format!("BAR Launcher.app/Contents/MacOS/{}", recoil::ENGINE_BINARY),
            runnable,
        )
        .unwrap();
        zip.write_all(b"\xcf\xfa\xed\xfe not really mach-o")
            .unwrap();
        zip.start_file("BAR Launcher.app/Contents/Resources/base/keep-me", plain)
            .unwrap();
        zip.write_all(b"content").unwrap();
        // What Apple's archiver leaves beside every real file.
        zip.start_file("__MACOSX/BAR Launcher.app/Contents/MacOS/._spring", plain)
            .unwrap();
        zip.write_all(b"resource fork").unwrap();
        zip.start_file("BAR Launcher.app/Contents/.DS_Store", plain)
            .unwrap();
        zip.write_all(b"finder").unwrap();

        for (name, bytes) in extra {
            zip.start_file(*name, plain).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn the_bundle_comes_out_runnable_and_without_apples_shadow_tree() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("release.zip");
        release_zip(&archive, &[]);
        let into = dir.path().join("recoil_2026.07.04");

        unpack(&archive, &into).expect("a bundle");

        let layout = recoil::EngineLayout::at(&into).expect("an engine in the bundle");
        assert!(layout.bundled());
        assert_eq!(
            layout.declared_version().as_deref(),
            Some("2026.07.04"),
            "the bundle says which engine it holds"
        );
        assert!(
            into.join("BAR Launcher.app/Contents/Resources/base/keep-me")
                .is_file()
        );
        assert!(
            !into.join("__MACOSX").exists(),
            "the resource-fork shadow tree is not part of the bundle"
        );
        assert!(!into.join("BAR Launcher.app/Contents/.DS_Store").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &std::path::Path| {
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777
            };
            assert_eq!(
                mode(&layout.engine()) & 0o111,
                0o111,
                "an engine that cannot be executed is not an install"
            );
            assert_eq!(
                mode(&into.join("BAR Launcher.app/Contents/Info.plist")) & 0o111,
                0,
                "and nothing else gains the bit"
            );
        }
    }

    /// The archive is a third party's, fetched over the network. An entry that
    /// names a path outside the directory is refused rather than clamped.
    #[test]
    fn an_entry_reaching_outside_the_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("evil.zip");
        release_zip(&archive, &[("../escaped", b"not here")]);
        let into = dir.path().join("engine");

        // `enclosed_name` refuses it, which is the same answer either way: it
        // does not become a file, and it does not become one outside.
        let _ = unpack(&archive, &into);
        assert!(!dir.path().join("escaped").exists());
    }

    #[test]
    fn an_archive_with_no_engine_in_it_is_not_an_install() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("wrong.zip");
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("readme.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"nothing to run").unwrap();
        zip.finish().unwrap();

        let err = unpack(&archive, &dir.path().join("engine")).unwrap_err();
        assert!(matches!(err, UnpackError::NoEngine(_)), "{err}");
    }

    /// The real release archive, unpacked by this code, against a real Mac.
    ///
    /// Everything above is written against a zip this file also wrote, which
    /// proves the loop and proves nothing about the archive the releases
    /// actually publish -- whether the bundle survives coming out of a library
    /// rather than out of `unzip`, and whether the ad-hoc signed Mach-O inside
    /// it still runs afterwards. That question cannot be answered without the
    /// seventy-five megabytes, so it is asked by hand:
    ///
    /// ```text
    /// curl -L -o /tmp/bar.zip https://github.com/Vandomas/RecoilEngine-AppleSilicon/releases/latest/download/BAR-Launcher-v0.15.1.zip
    /// MODLOBBY_APPLE_ZIP=/tmp/bar.zip cargo test -p content -- --ignored apple
    /// ```
    #[test]
    #[ignore = "needs a release archive; set MODLOBBY_APPLE_ZIP"]
    fn a_real_release_archive_unpacks_into_an_engine_that_runs() {
        let Some(archive) = std::env::var_os("MODLOBBY_APPLE_ZIP") else {
            panic!("set MODLOBBY_APPLE_ZIP to a downloaded release .zip");
        };
        let dir = tempfile::tempdir().unwrap();
        let into = dir.path().join("engine");
        unpack(std::path::Path::new(&archive), &into).expect("a bundle");

        let layout = recoil::EngineLayout::at(&into).expect("an engine in the bundle");
        let version = layout
            .declared_version()
            .expect("a declared engine version");
        let port = layout.declared_port().expect("a declared port version");
        assert!(layout.bundled());
        assert!(layout.downloader().is_file(), "pr-downloader comes with it");
        assert!(
            !layout.environment().is_empty(),
            "a bundle that would come up blank without its graphics environment"
        );

        // The one thing only a real archive can answer: the binary is ad-hoc
        // signed, and a file that came out of the extractor with its bytes or
        // its executable bit wrong is refused by the kernel rather than by
        // anything this code could check.
        let mut engine = std::process::Command::new(layout.engine());
        for (key, value) in layout.environment() {
            engine.env(key, value);
        }
        let ran = engine
            .arg("--version")
            .current_dir(&layout.bin)
            .output()
            .expect("the unpacked engine executes");
        let said = String::from_utf8_lossy(&ran.stdout);
        assert!(
            said.contains(&version),
            "the engine says which it is: {said:?}"
        );
        println!("engine {version} from port {port}: {}", said.trim());
    }

    /// Only Apple Silicon, and the same answer wherever it is asked.
    #[test]
    fn only_the_machine_the_port_is_built_for() {
        assert_eq!(
            supported(),
            cfg!(all(target_os = "macos", target_arch = "aarch64"))
        );
        assert!(!supported() || cfg!(target_os = "macos"));
    }
}
