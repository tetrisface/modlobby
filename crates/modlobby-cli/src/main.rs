//! `modlobby-cli`: a harness for the lobby core against a live teiserver.
//! `login` watches the battle list; `join` enters a room as a spectator, prints
//! its chat and, with `--launch`, connects the engine once the game is running.

mod platform;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use lobby_core::{Effect, Session};
use spring_protocol::policy::PolicyEvent;
use spring_protocol::{Area, Endpoint, Inbound, LoginRequest, ThrottlePolicy, Transport};
use tokio::process::Child;
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
        /// BAR data directory (`engine/`, `games/`, `maps/`); defaults to the launcher's.
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
    /// Shown after `LuaLobby Chobby:`; truncated server-side.
    #[arg(long, default_value = concat!("modlobby ", env!("CARGO_PKG_VERSION")))]
    lobby_version: String,
    /// TOML override of the throttle policy (see `ThrottlePolicy`).
    #[arg(long)]
    policy: Option<PathBuf>,
    /// Stop after this many seconds; 0 keeps going until Ctrl+C. A launched engine keeps the session alive.
    #[arg(long, default_value_t = 60)]
    watch_secs: u64,
}

enum Mode {
    Watch {
        report_secs: u64,
    },
    Join {
        battle: u32,
        password: Option<String>,
        /// Data directory to launch from, when `--launch` was given.
        launch: Option<PathBuf>,
    },
}

enum Next {
    Inbound(Option<Inbound>),
    Tick,
    EngineExited(std::io::Result<ExitStatus>),
    Interrupt,
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
        } => run(connection, Mode::Watch { report_secs }).await,
        Command::Join {
            connection,
            battle,
            password,
            launch,
            data_dir,
        } => {
            let launch = match (launch, data_dir.or_else(default_data_dir)) {
                (false, _) => None,
                (true, Some(dir)) => Some(dir),
                (true, None) => bail!("no BAR data directory found; pass --data-dir"),
            };
            run(
                connection,
                Mode::Join {
                    battle,
                    password,
                    launch,
                },
            )
            .await
        }
    }
}

/// Where the BAR launcher keeps its data on Windows.
fn default_data_dir() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let dir = PathBuf::from(local)
        .join("Programs")
        .join("Beyond-All-Reason")
        .join("data");
    dir.is_dir().then_some(dir)
}

fn load_policy(path: Option<&Path>) -> anyhow::Result<ThrottlePolicy> {
    let Some(path) = path else {
        return Ok(ThrottlePolicy::default());
    };
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(toml::from_str(&text)?)
}

async fn run(conn: Connection, mode: Mode) -> anyhow::Result<()> {
    let password = std::env::var(PASSWORD_ENV).with_context(|| format!("set {PASSWORD_ENV}"))?;
    let policy = load_policy(conn.policy.as_deref())?;
    let endpoint = Endpoint::parse(&conn.server, !conn.plain)
        .with_context(|| format!("--server must be host:port, got {}", conn.server))?;
    let hardware = platform::detect();
    tracing::info!(lobby_hash = hardware.lobby_hash, "machine identity");

    let mut session = Session::new(
        LoginRequest::new(
            &conn.username,
            &password,
            &conn.lobby_version,
            hardware.lobby_hash.clone(),
        ),
        hardware.properties,
        hardware.machine_hash,
    );
    let after_flood = Duration::from_secs_f64(policy.login.after_flood_secs);
    let (transport, mut inbound) = Transport::connect(&endpoint, policy)
        .await
        .with_context(|| format!("connecting to {}", conn.server))?;
    println!(
        "connected to {} ({})",
        conn.server,
        if endpoint.tls { "tls" } else { "plain" }
    );

    let started = Instant::now();
    let deadline = (conn.watch_secs > 0).then(|| started + Duration::from_secs(conn.watch_secs));
    let tick_secs = match mode {
        Mode::Watch { report_secs } => report_secs.max(1),
        Mode::Join { .. } => 1,
    };
    let mut tick = tokio::time::interval(Duration::from_secs(tick_secs));
    tick.tick().await;
    let mut ready = false;
    let mut engine: Option<Child> = None;

    loop {
        let next = tokio::select! {
            next = inbound.recv() => Next::Inbound(next),
            _ = tick.tick() => Next::Tick,
            status = wait_engine(&mut engine) => Next::EngineExited(status),
            _ = tokio::signal::ctrl_c() => Next::Interrupt,
        };
        let event = match next {
            Next::Tick => {
                if ready && matches!(mode, Mode::Watch { .. }) {
                    print_summary(&session);
                }
                if engine.is_none() && deadline.is_some_and(|d| Instant::now() >= d) {
                    break;
                }
                continue;
            }
            Next::EngineExited(status) => {
                send_all(&transport, session.set_in_game(false)).await?;
                println!(
                    "engine exited: {}",
                    status.map_or_else(|e| e.to_string(), |s| s.to_string())
                );
                engine = None;
                break;
            }
            Next::Interrupt => break,
            Next::Inbound(None) => bail!("transport closed"),
            Next::Inbound(Some(Inbound::Closed { reason })) => bail!("connection closed: {reason}"),
            Next::Inbound(Some(Inbound::Policy(event))) => {
                match event {
                    PolicyEvent::Delayed {
                        area,
                        pending,
                        wait,
                    } => tracing::debug!(?area, pending, ?wait, "throttled"),
                    other => tracing::info!(?other, "policy"),
                }
                continue;
            }
            Next::Inbound(Some(Inbound::Message(event))) => event,
        };

        let mut queue: VecDeque<Effect> = session.handle(event).into();
        while let Some(effect) = queue.pop_front() {
            match effect {
                Effect::Send(envelope) => transport.send(envelope).await?,
                Effect::LoggedIn { username } => println!("logged in as {username}"),
                Effect::Ready => {
                    ready = true;
                    println!("login flood done in {:.1?}", started.elapsed());
                    match &mode {
                        Mode::Watch { .. } => print_summary(&session),
                        Mode::Join {
                            battle, password, ..
                        } => {
                            print_battle(&session, *battle)?;
                            let script_password =
                                format!("{}{}", rand::random::<u16>(), rand::random::<u16>());
                            queue.extend(session.join_battle(
                                *battle,
                                password.as_deref(),
                                script_password,
                            ));
                        }
                    }
                }
                Effect::Joined { id } => println!("joined battle {id} as a spectator"),
                Effect::JoinFailed { reason } => bail!("join failed: {reason}"),
                Effect::LeftBattle { id } => println!("left battle {id}"),
                Effect::BattleChat {
                    from,
                    text,
                    announcement,
                } => println!("{}{from}: {text}", if announcement { "* " } else { "" }),
                Effect::GameRunning {
                    id,
                    ip,
                    port,
                    script_password,
                } => {
                    println!("battle {id}: game running at {ip}:{port}");
                    if let Mode::Join {
                        launch: Some(data_dir),
                        ..
                    } = &mode
                        && engine.is_none()
                    {
                        let me = session.state.me.as_deref().unwrap_or(&conn.username);
                        let url = recoil::spring_url(me, &script_password, &ip, port);
                        // SPADS /adduser's us to the running game only once the lobby
                        // shows us in game; the engine needs a few seconds to reach the host.
                        send_all(&transport, session.set_in_game(true)).await?;
                        engine = Some(launch_engine(&session, id, data_dir, url)?);
                    }
                }
                Effect::LoginDenied { reason } => bail!("login denied: {reason}"),
                Effect::AgreementRequired { text } => bail!(
                    "account must confirm the user agreement first:\n{}",
                    text.join("\n")
                ),
                Effect::Redirect { host, port } => bail!(
                    "server redirects to {host}:{}",
                    port.map_or("?".into(), |p| p.to_string())
                ),
                Effect::Disconnected { reason, flood } => {
                    if flood {
                        transport
                            .trip(Area::Login, Instant::now() + after_flood)
                            .await?;
                    }
                    bail!(
                        "disconnected by server: {reason}{}",
                        if flood {
                            " (flood protection; wait before retrying)"
                        } else {
                            ""
                        }
                    );
                }
                Effect::Notice(text) => println!("server: {text}"),
            }
        }
    }

    let leaving = session.leave_battle();
    if !leaving.is_empty() {
        send_all(&transport, leaving).await?;
        // Let the writer flush LEAVEBATTLE before the socket goes away.
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    transport.shutdown().await;
    if engine.is_some() {
        println!("engine still running; it keeps its own connection to the host");
    }
    Ok(())
}

async fn send_all(transport: &Transport, effects: Vec<Effect>) -> anyhow::Result<()> {
    for effect in effects {
        if let Effect::Send(envelope) = effect {
            transport.send(envelope).await?;
        }
    }
    Ok(())
}

/// Resolves when the launched engine exits; never, when none was launched.
async fn wait_engine(engine: &mut Option<Child>) -> std::io::Result<ExitStatus> {
    match engine {
        Some(child) => child.wait().await,
        None => std::future::pending().await,
    }
}

fn launch_engine(
    session: &Session,
    battle_id: u32,
    data_dir: &Path,
    url: String,
) -> anyhow::Result<Child> {
    let battle = session
        .state
        .battles
        .get(&battle_id)
        .context("battle vanished before launch")?;
    let engine_dir = recoil::find_engine(data_dir, &battle.engine_version).with_context(|| {
        format!(
            "no engine {} with {} under {}",
            battle.engine_version,
            recoil::ENGINE_BINARY,
            data_dir.join("engine").display()
        )
    })?;
    let launch = recoil::Launch {
        engine_dir,
        data_dir: data_dir.to_path_buf(),
        target: url,
    };
    println!(
        "launching {} (engine {})",
        launch.engine_dir.display(),
        battle.engine_version
    );
    tokio::process::Command::from(launch.command())
        .spawn()
        .context("spawning the engine")
}

fn print_battle(session: &Session, id: u32) -> anyhow::Result<()> {
    let state = &session.state;
    let Some(battle) = state.battles.get(&id) else {
        bail!("battle {id} is not on the list");
    };
    let host_in_game = state
        .users
        .get(&battle.founder)
        .is_some_and(|u| u.status.in_game);
    println!(
        "battle {id}: {}  map {}  host {}  {} players {} specs{}{}{}",
        battle.title,
        battle.map_name,
        battle.founder,
        battle.player_count(),
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

fn print_summary(session: &Session) {
    let state = &session.state;
    let in_battle = state.user_battle.len();
    println!(
        "users {:>5}  battles {:>3}  in battles {:>4}  in game {:>4}",
        state.users.len(),
        state.battles.len(),
        in_battle,
        state.users.values().filter(|u| u.status.in_game).count()
    );
    for battle in state.battles_by_players().into_iter().take(10) {
        println!(
            "  {:>4}  {:>2} players {:>2} specs  {:>4}  {:<40.40}  {:<28.28}  {}",
            battle.id,
            battle.player_count(),
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
