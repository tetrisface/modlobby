//! Escape, in the game, meaning what it means everywhere else.
//!
//! The engine gives a lobby no way in from the outside: a global hotkey works,
//! but Escape belongs to the game, and grabbing it system-wide would take it
//! away from every other program too. The only thing that can see Escape
//! *inside* a game is a widget, so there is one — about forty lines, whose
//! entire job is to notice Escape pressed with nothing selected and tell us.
//!
//! It deliberately draws nothing. The obvious design has the widget paint its
//! own little menu in Lua; this one just raises the lobby, which already is the
//! menu, in HTML, with the room and the chat still in it. Fewer moving parts
//! and a much better-looking result.
//!
//! Two properties make it safe to install into a directory we do not own:
//!
//! - It is inert unless modlobby is listening. A failed connection means the
//!   key is *not* consumed, so a Chobby game behaves exactly as it always did.
//! - It is removed when modlobby exits cleanly, and rewritten on each launch,
//!   so a stale one from an old version never lingers.
//!
//! The socket is bound to loopback on an ephemeral port and guarded by a token
//! generated per run and baked into the widget we write. Any local process can
//! reach a loopback port, and this one can quit a game, so being reachable is
//! not the same as being allowed.

pub mod protocol;
pub mod widget;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

pub use protocol::{Command, parse};

/// What the in-game widget can ask for.
///
/// Both answer whether they acted. That answer is the whole reason the widget
/// waits for a reply rather than firing and forgetting: it consumes Escape
/// only when the key actually did something, so a game started from another
/// lobby while modlobby happens to be open keeps its own Escape.
pub trait Actions: Send + Sync + 'static {
    /// Raise the lobby over the game. `false` if this is not our game.
    fn raise(&self) -> bool;
    /// Stop the game, leaving the lobby up. `false` if there was none.
    fn quit_game(&self) -> bool;
}

/// A running control socket, and where its widget was written.
pub struct InGame {
    pub port: u16,
    pub token: String,
    installed: Option<PathBuf>,
}

impl InGame {
    /// Binds loopback and starts listening. The port is chosen by the OS.
    pub async fn start(actions: Arc<dyn Actions>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let token = token();

        let guard = token.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    // The listener is gone; nothing will arrive again.
                    return;
                };
                let actions = actions.clone();
                let guard = guard.clone();
                tokio::spawn(async move { serve(stream, guard, actions).await });
            }
        });

        Ok(Self {
            port,
            token,
            installed: None,
        })
    }

    /// Writes the widget into the BAR data directory, replacing any older one.
    pub fn install(&mut self, data_dir: &Path) -> std::io::Result<PathBuf> {
        let path = widget::path(data_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, widget::source(self.port, &self.token))?;
        self.installed = Some(path.clone());
        Ok(path)
    }

    /// Takes the widget back out. Called on the way down, and safe to repeat.
    pub fn uninstall(&mut self) {
        let Some(path) = self.installed.take() else {
            return;
        };
        // A widget we cannot remove is inert anyway: it fails to connect and
        // stops consuming the key.
        if let Err(err) = std::fs::remove_file(&path) {
            tracing::warn!(%err, path = %path.display(), "leaving the in-game widget behind");
        }
    }
}

impl Drop for InGame {
    fn drop(&mut self) {
        self.uninstall();
    }
}

async fn serve(stream: TcpStream, token: String, actions: Arc<dyn Actions>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    // One connection may carry several presses; the widget keeps it open.
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let acted = match parse(&line, &token) {
            Some(Command::Raise) => actions.raise(),
            Some(Command::QuitGame) => actions.quit_game(),
            // A bad token is the interesting case and says nothing back: a
            // reply would tell a guesser whether it was close.
            None => return,
        };
        let reply: &[u8] = if acted { b"ok\n" } else { b"no\n" };
        if reader.get_mut().write_all(reply).await.is_err() {
            return;
        }
    }
}

/// A per-run secret, baked into the widget we write and required on the wire.
fn token() -> String {
    // Four 32-bit draws rather than 32 nibbles: the same 128 bits, and it uses
    // the one `rand` entry point the rest of this workspace already uses.
    (0..4).fold(String::new(), |mut token, _| {
        use std::fmt::Write;
        let _ = write!(token, "{:08x}", rand::random::<u32>());
        token
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct Spy {
        raised: AtomicUsize,
        quit: AtomicUsize,
        /// What the app would answer: whether this is our game.
        answer: bool,
    }

    fn spy(answer: bool) -> Arc<Spy> {
        Arc::new(Spy {
            answer,
            ..Spy::default()
        })
    }

    impl Actions for Spy {
        fn raise(&self) -> bool {
            self.raised.fetch_add(1, Ordering::SeqCst);
            self.answer
        }
        fn quit_game(&self) -> bool {
            self.quit.fetch_add(1, Ordering::SeqCst);
            self.answer
        }
    }

    /// Sends one line and reads the answer back.
    async fn ask(port: u16, line: &str) -> String {
        let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut stream = BufReader::new(stream);
        stream.get_mut().write_all(line.as_bytes()).await.unwrap();
        let mut reply = String::new();
        stream.read_line(&mut reply).await.unwrap();
        reply.trim().to_owned()
    }

    #[tokio::test]
    async fn a_press_with_the_right_token_raises_the_lobby() {
        let spy = spy(true);
        let ingame = InGame::start(spy.clone()).await.unwrap();

        let reply = ask(ingame.port, &format!("{} raise\n", ingame.token)).await;
        assert_eq!(reply, "ok");
        assert_eq!(spy.raised.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_game_that_is_not_ours_is_told_so_and_keeps_its_escape() {
        // The widget consumes the key only on `ok`. Answering `no` is what
        // leaves a Chobby game's Escape alone while modlobby happens to run.
        let spy = spy(false);
        let ingame = InGame::start(spy.clone()).await.unwrap();

        let reply = ask(ingame.port, &format!("{} raise\n", ingame.token)).await;
        assert_eq!(reply, "no");
        assert_eq!(spy.raised.load(Ordering::SeqCst), 1, "asked, and declined");
    }

    #[tokio::test]
    async fn something_else_on_the_port_gets_nowhere() {
        let spy = spy(true);
        let ingame = InGame::start(spy.clone()).await.unwrap();

        let mut stream = TcpStream::connect(("127.0.0.1", ingame.port))
            .await
            .unwrap();
        // Anyone can reach a loopback port; reaching it is not permission.
        stream.write_all(b"0000 quit\nalso quit\n").await.unwrap();
        stream.flush().await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        assert_eq!(spy.quit.load(Ordering::SeqCst), 0);
        assert_eq!(spy.raised.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_widget_is_written_and_taken_back_out() {
        let home = tempfile::tempdir().unwrap();
        let mut ingame = InGame {
            port: 4242,
            token: "abc".into(),
            installed: None,
        };

        let path = ingame.install(home.path()).unwrap();
        assert!(path.is_file());
        let source = std::fs::read_to_string(&path).unwrap();
        assert!(source.contains("4242"), "the port is baked in");
        assert!(source.contains("abc"), "and so is the token");

        ingame.uninstall();
        assert!(!path.exists());
        // Removing twice is what a drop after an explicit stop does.
        ingame.uninstall();
    }
}
