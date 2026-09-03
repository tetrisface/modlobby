//! `modlobby-cli`: a harness for the lobby runtime against a live teiserver.
//! `login` watches the battle list; `join` enters a room as a spectator, prints
//! its chat and, with `--launch`, connects the engine once the game is running.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use lobby_runtime::{Client, launch, platform};
use lobby_ui::{ChatKind, Delta, EngineStatus, Snapshot, UiClosed, UiMessage, UiTransport};
use spring_protocol::{Endpoint, LoginRequest, ThrottlePolicy};
use tracing_subscriber::EnvFilter;

const PASSWORD_ENV: &str = "MODLOBBY_PASSWORD";

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Log in and watch the battle list.
    Login {
        #[command(flatten)]
        connection: Connection,
        /// Summary interval while watching.
        #[arg(long, default_value_t = 10)]
        report_secs: u64,
    },
    /// Log in and join a battle as a spectator; prints the room's chat.
    Join {
        #[command(flatten)]
        connection: Connection,
        /// Battle id, as listed by `login`.
        #[arg(long)]
        battle: u32,
        /// Room password, if it has one.
        #[arg(long)]
        password: Option<String>,
        /// Connect the engine as soon as the room's game is running (or already is).
        #[arg(long)]
        launch: bool,
        /// BAR data directory to write (`engine/`, `games/`, `maps/`); defaults to
        /// modlobby's own. Other lobbies' installs are read either way.
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Print the default throttle policy as TOML, as a starting point for `--policy`.
    Policy,
}

#[derive(Args)]
struct Connection {
    /// The password is read from `MODLOBBY_PASSWORD`; a `.env` in the working directory is loaded first.
    #[arg(long, env = "MODLOBBY_USERNAME")]
    username: String,
    /// `host:port`; teiserver speaks TLS on 8201 and plain TCP on 8200.
    #[arg(long, default_value = "server4.beyondallreason.info:8201")]
    server: String,
    /// Plain TCP instead of TLS (what Chobby does on 8200).
    #[arg(long)]
    plain: bool,
    /// Announced client. teiserver stores the leading `[a-zA-Z ]+` of
    /// `<name>:<version>` and gives unlisted names the filtered `:full` feed,
    /// where other rooms' rosters read empty.
    #[arg(long, default_value = spring_protocol::login::MODLOBBY_CLIENT)]
    client_name: String,
    /// Shown after `<client>:`; truncated server-side.
    #[arg(long, default_value = env!("CARGO_PKG_VERSION"))]
    lobby_version: String,
    /// TOML override of the throttle policy (see `ThrottlePolicy`).
    #[arg(long)]
    policy: Option<PathBuf>,
    /// Stop after this many seconds; 0 keeps going until Ctrl+C. A launched engine keeps the session alive.
    #[arg(long, default_value_t = 60)]
    watch_secs: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dotenv = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(true)
        .init();
    match dotenv {
        Ok(path) => tracing::debug!(path = %path.display(), "loaded .env"),
        Err(err) if err.not_found() => {}
        Err(err) => tracing::warn!(%err, "ignoring .env"),
    }
    match Cli::parse().command {
        Command::Policy => {
            print!("{}", toml::to_string_pretty(&ThrottlePolicy::default())?);
            Ok(())
        }
        Command::Login {
            connection,
            report_secs,
        } => {
            let client = connect(&connection).await?;
            let outcome = watch(&client, &connection, report_secs).await;
            client.shutdown().await;
            outcome
        }
        Command::Join {
            connection,
            battle,
            password,
            launch,
            data_dir,
        } => {
            let data_dir = match (launch, launch::data_dirs(data_dir)) {
                (false, _) => None,
                (true, Some(dirs)) => Some(dirs),
                (true, None) => bail!("no home directory for BAR content; pass --data-dir"),
            };
            let client = connect(&connection).await?;
            let outcome = spectate(&client, &connection, battle, password, data_dir).await;
            client.shutdown().await;
            outcome
        }
    }
}

/// Prints what a front end would render.
struct Print;

impl UiTransport for Print {
    fn send(&self, message: UiMessage) -> Result<(), UiClosed> {
        let UiMessage::Deltas(deltas) = message else {
            return Ok(());
        };
        for delta in deltas {
            match delta {
                Delta::Chat(line) => {
                    let prefix = match line.kind {
                        ChatKind::Chat => "",
                        ChatKind::Announcement => "* ",
                        ChatKind::Private => "[pm] ",
                        ChatKind::Emote => "* ",
                        ChatKind::System => "-- ",
                        // Kept in the harness: watching the host's own state
                        // go past is most of what this tool is for.
                        ChatKind::Machine => "~ ",
                    };
                    println!(
                        "{}{prefix}{}: {}",
                        room_tag(&line.room),
                        line.from,
                        line.text
                    );
                }
                Delta::Notice { level, text } => println!("[{level:?}] {text}"),
                Delta::GameRunning(Some(game)) => {
                    println!(
                        "battle {}: game running at {}:{}",
                        game.id, game.ip, game.port
                    )
                }
                Delta::MyBattle(Some(my)) => println!("joined battle {} as a spectator", my.id),
                Delta::MyBattle(None) => println!("left the battle"),
                Delta::Engine(EngineStatus::Running { pid }) => {
                    println!(
                        "engine launched{}",
                        match pid {
                            Some(pid) => format!(" (pid {pid})"),
                            None => String::new(),
                        }
                    )
                }
                Delta::Engine(EngineStatus::Exited { code }) => println!("engine exited: {code:?}"),
                _ => {}
            }
        }
        Ok(())
    }
}

fn load_policy(path: Option<&Path>) -> anyhow::Result<ThrottlePolicy> {
    let Some(path) = path else {
        return Ok(ThrottlePolicy::default());
    };
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(toml::from_str(&text)?)
}

async fn connect(conn: &Connection) -> anyhow::Result<Client> {
    let password = std::env::var(PASSWORD_ENV).with_context(|| format!("set {PASSWORD_ENV}"))?;
    let policy = load_policy(conn.policy.as_deref())?;
    let endpoint = Endpoint::parse(&conn.server, !conn.plain)
        .with_context(|| format!("--server must be host:port, got {}", conn.server))?;
    let hardware = platform::detect();
    tracing::info!(lobby_hash = hardware.lobby_hash, "machine identity");
    let request = LoginRequest::new(
        &conn.username,
        &password,
        &conn.lobby_version,
        hardware.lobby_hash.clone(),
    )
    .client_name(&conn.client_name);

    let client = Client::spawn(policy, hardware, None);
    client.subscribe(Print).await?;
    let started = Instant::now();
    client
        .login(endpoint, request)
        .await
        .with_context(|| format!("logging in to {}", conn.server))?;
    println!(
        "logged in to {} ({}) in {:.1?}",
        conn.server,
        if endpoint_tls(conn) { "tls" } else { "plain" },
        started.elapsed()
    );
    Ok(client)
}

fn endpoint_tls(conn: &Connection) -> bool {
    !conn.plain
}

async fn watch(client: &Client, conn: &Connection, report_secs: u64) -> anyhow::Result<()> {
    let deadline = deadline(conn);
    let mut tick = tokio::time::interval(Duration::from_secs(report_secs.max(1)));
    print_summary(&client.snapshot().await?);
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let snapshot = client.snapshot().await?;
                if snapshot.phase.is_none() {
                    bail!("disconnected");
                }
                print_summary(&snapshot);
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    return Ok(());
                }
            }
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

async fn spectate(
    client: &Client,
    conn: &Connection,
    battle: u32,
    password: Option<String>,
    data_dir: Option<content::DataDirs>,
) -> anyhow::Result<()> {
    print_battle(&client.snapshot().await?, battle)?;
    client
        .join_battle(battle, password)
        .await
        .context("joining")?;
    if let Some(dirs) = data_dir {
        client.launch(dirs).await.context("launching")?;
    }
    let deadline = deadline(conn);
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let snapshot = client.snapshot().await?;
                if snapshot.phase.is_none() {
                    bail!("disconnected");
                }
                let engine_running = matches!(snapshot.engine, EngineStatus::Running { .. });
                if !engine_running && deadline.is_some_and(|d| Instant::now() >= d) {
                    return Ok(());
                }
            }
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

fn deadline(conn: &Connection) -> Option<Instant> {
    (conn.watch_secs > 0).then(|| Instant::now() + Duration::from_secs(conn.watch_secs))
}

fn print_battle(snapshot: &Snapshot, id: u32) -> anyhow::Result<()> {
    let Some(battle) = snapshot.battles.iter().find(|b| b.id == id) else {
        bail!("battle {id} is not on the list");
    };
    let host_in_game = snapshot
        .users
        .iter()
        .any(|u| u.name == battle.founder && u.status.in_game);
    println!(
        "battle {id}: {}  map {}  host {}  {} players {} specs{}{}{}",
        battle.title,
        battle.map_name,
        battle.founder,
        battle.player_count,
        battle.spectator_count,
        if host_in_game { "  in game" } else { "" },
        if battle.locked { "  locked" } else { "" },
        if battle.passworded {
            "  passworded"
        } else {
            ""
        },
    );
    Ok(())
}

fn print_summary(snapshot: &Snapshot) {
    let in_battle = snapshot
        .users
        .iter()
        .filter(|u| u.battle_id.is_some())
        .count();
    println!(
        "users {:>5}  battles {:>3}  in battles {:>4}  in game {:>4}",
        snapshot.users.len(),
        snapshot.battles.len(),
        in_battle,
        snapshot.users.iter().filter(|u| u.status.in_game).count()
    );
    let mut battles: Vec<_> = snapshot.battles.iter().collect();
    battles.sort_by(|a, b| {
        b.player_count
            .cmp(&a.player_count)
            .then_with(|| a.id.cmp(&b.id))
    });
    for battle in battles.into_iter().take(10) {
        println!(
            "  {:>4}  {:>2} players {:>2} specs  {:>4}  {:<40.40}  {:<28.28}  {}",
            battle.id,
            battle.player_count,
            battle.spectator_count,
            battle
                .layout
                .map_or(String::new(), |l| format!("{}x{}", l.teams, l.team_size)),
            battle.title,
            battle.map_name,
            if battle.locked { "locked" } else { "" }
        );
    }
}

/// Channels are worth naming in a log that carries several at once; the room
/// we are in is not, since it is the only one.
fn room_tag(room: &str) -> String {
    if room == lobby_ui::BATTLE_ROOM {
        String::new()
    } else {
        format!("{room} ")
    }
}
