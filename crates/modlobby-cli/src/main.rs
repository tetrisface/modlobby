//! `modlobby-cli login`: connect to teiserver over the legacy protocol, log in as
//! a Chobby-class client, and print battle/player counts as they change.

mod platform;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use lobby_core::{Effect, Session};
use spring_protocol::policy::PolicyEvent;
use spring_protocol::{Area, Inbound, LoginRequest, ThrottlePolicy, Transport};
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
    /// Log in and watch the battle list; the password is read from `MODLOBBY_PASSWORD`.
    Login {
        #[arg(long)]
        username: String,
        #[arg(long, default_value = "server4.beyondallreason.info:8200")]
        server: String,
        /// Shown after `LuaLobby Chobby:`; truncated server-side.
        #[arg(long, default_value = concat!("modlobby ", env!("CARGO_PKG_VERSION")))]
        lobby_version: String,
        /// TOML override of the throttle policy (see `ThrottlePolicy`).
        #[arg(long)]
        policy: Option<PathBuf>,
        /// Stop after this many seconds; 0 keeps watching until Ctrl+C.
        #[arg(long, default_value_t = 60)]
        watch_secs: u64,
        /// Summary interval while watching.
        #[arg(long, default_value_t = 10)]
        report_secs: u64,
    },
    /// Print the default throttle policy as TOML, as a starting point for `--policy`.
    Policy,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(true)
        .init();
    match Cli::parse().command {
        Command::Policy => {
            print!("{}", toml::to_string_pretty(&ThrottlePolicy::default())?);
            Ok(())
        }
        Command::Login {
            username,
            server,
            lobby_version,
            policy,
            watch_secs,
            report_secs,
        } => {
            let policy = match policy {
                Some(path) => toml::from_str(
                    &std::fs::read_to_string(&path)
                        .with_context(|| format!("reading {}", path.display()))?,
                )?,
                None => ThrottlePolicy::default(),
            };
            login(
                &username,
                &server,
                &lobby_version,
                policy,
                watch_secs,
                report_secs,
            )
            .await
        }
    }
}

async fn login(
    username: &str,
    server: &str,
    lobby_version: &str,
    policy: ThrottlePolicy,
    watch_secs: u64,
    report_secs: u64,
) -> anyhow::Result<()> {
    let password = std::env::var(PASSWORD_ENV).with_context(|| format!("set {PASSWORD_ENV}"))?;
    let hardware = platform::detect();
    tracing::info!(lobby_hash = hardware.lobby_hash, "machine identity");

    let mut session = Session::new(
        LoginRequest::new(
            username,
            &password,
            lobby_version,
            hardware.lobby_hash.clone(),
        ),
        hardware.properties,
        hardware.machine_hash,
    );
    let after_flood = Duration::from_secs_f64(policy.login.after_flood_secs);
    let (transport, mut inbound) = Transport::connect(server, policy)
        .await
        .with_context(|| format!("connecting to {server}"))?;
    println!("connected to {server}");

    let started = Instant::now();
    let deadline = (watch_secs > 0).then(|| started + Duration::from_secs(watch_secs));
    let mut report = tokio::time::interval(Duration::from_secs(report_secs.max(1)));
    report.tick().await;
    let mut ready = false;

    loop {
        let next = tokio::select! {
            next = inbound.recv() => next,
            _ = report.tick() => {
                if ready { print_summary(&session); }
                if deadline.is_some_and(|d| Instant::now() >= d) { break; }
                continue;
            }
            _ = tokio::signal::ctrl_c() => break,
        };
        let Some(next) = next else {
            bail!("transport closed")
        };
        match next {
            Inbound::Message(event) => {
                for effect in session.handle(event) {
                    match effect {
                        Effect::Send(envelope) => transport.send(envelope).await?,
                        Effect::LoggedIn { username } => println!("logged in as {username}"),
                        Effect::Ready => {
                            ready = true;
                            println!("login flood done in {:.1?}", started.elapsed());
                            print_summary(&session);
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
            Inbound::Policy(event) => match event {
                PolicyEvent::Delayed {
                    area,
                    pending,
                    wait,
                } => tracing::debug!(?area, pending, ?wait, "throttled"),
                other => tracing::info!(?other, "policy"),
            },
            Inbound::Closed { reason } => bail!("connection closed: {reason}"),
        }
    }
    transport.shutdown().await;
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
            "  {:>4}  {:>2} players {:>2} specs  {:<40.40}  {:<28.28}  {}",
            battle.id,
            battle.player_count(),
            battle.spectator_count,
            battle.title,
            battle.map_name,
            if battle.locked { "locked" } else { "" }
        );
    }
}
