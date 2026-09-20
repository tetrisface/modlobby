//! The script the host's engine starts on, from the room as it stands.
//!
//! Everything a guest's engine will be checked against is here: each member
//! as a `[player]` with the password it joined by, the seats as `[team]`s
//! under ally teams renumbered without gaps, the AIs with who runs them, and
//! the modoptions the founder set. The founder is player 0, as the one whose
//! machine every AI runs on.

use std::collections::BTreeMap;

use recoil::script::{Ai, AllyTeam, Player, Rect, Skirmish, StartPos, Team, faction};
use spring_protocol::BattleStatus;

use crate::ENGINE_PORT;
use crate::room::Room;

/// Legion is not in the game until this is on (`sidedata.lua:21`).
const LEGION: &str = "game/modoptions/experimentallegionfaction";

impl Room {
	/// The start script for the game this room describes.
	pub fn host_script(&self) -> Skirmish {
		let founder = &self.config().founder;
		let legion = self.tags().get(LEGION).map(String::as_str) == Some("1");
		// The founder first, whatever order people arrived in: AIs name
		// their host by player index, and it is 0.
		let mut members: Vec<(&str, &str, BattleStatus, u32)> = self.members().collect();
		members.sort_by_key(|(name, ..)| *name != founder);

		// Ally teams as the room numbers them, renumbered without gaps.
		let mut sides: Vec<u8> = members
			.iter()
			.filter(|(_, _, status, _)| status.player)
			.map(|(_, _, status, _)| status.ally_team)
			.chain(self.bots().map(|(_, _, status, ..)| status.ally_team))
			.collect();
		sides.sort_unstable();
		sides.dedup();
		let side_of = |ally: u8| sides.iter().position(|s| *s == ally).unwrap_or(0) as u8;

		let mut teams: Vec<Team> = Vec::new();
		let mut players: Vec<Player> = Vec::new();
		for (name, password, status, colour) in &members {
			let team = status.player.then(|| {
				teams.push(Team {
					ally_team: side_of(status.ally_team),
					leader: players.len() as u8,
					side: faction(status.side, legion, false),
					colour: Some(*colour),
					handicap: status.handicap,
				});
				teams.len() as u8 - 1
			});
			players.push(Player {
				name: (*name).to_owned(),
				team,
				password: Some((*password).to_owned()),
			});
		}
		let index_of = |name: &str| {
			players
				.iter()
				.position(|player| player.name == name)
				.unwrap_or(0) as u8
		};
		let mut ais: Vec<Ai> = Vec::new();
		for (name, owner, status, colour, kind) in self.bots() {
			let host = index_of(owner);
			teams.push(Team {
				ally_team: side_of(status.ally_team),
				leader: host,
				side: faction(status.side, legion, true),
				colour: Some(colour),
				handicap: status.handicap,
			});
			ais.push(Ai {
				name: name.to_owned(),
				short_name: kind.to_owned(),
				version: None,
				team: teams.len() as u8 - 1,
				host,
				options: Vec::new(),
			});
		}

		let rects: &BTreeMap<u8, (u16, u16, u16, u16)> = self.rects();
		let ally_teams = sides
			.iter()
			.map(|ally| AllyTeam {
				start_rect: rects
					.get(ally)
					.map(|(l, t, r, b)| Rect::from_200(*l, *t, *r, *b)),
			})
			.collect();

		let start_pos = match self.tags().get("game/startpostype").map(String::as_str) {
			Some("0") => StartPos::Fixed,
			Some("1") => StartPos::Random,
			_ => StartPos::InGame,
		};
		let modoptions = self
			.tags()
			.iter()
			.filter_map(|(key, value)| {
				let key = key.strip_prefix("game/modoptions/")?;
				(!value.is_empty()).then(|| (key.to_owned(), value.clone()))
			})
			.collect();

		Skirmish {
			game: self.config().game.clone(),
			map: self.config().map.clone(),
			player: founder.clone(),
			start_pos,
			modoptions,
			ally_teams,
			teams,
			players,
			ais,
			host_ip: "0.0.0.0".into(),
			host_port: ENGINE_PORT,
		}
	}
}

#[cfg(test)]
mod tests {
	use std::net::{IpAddr, Ipv4Addr};

	use spring_protocol::{MyBattleStatus, Sync};

	use super::*;
	use crate::room::{Config, Policy};

	const IP: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

	fn seated(
		room: &mut Room,
		peer: u64,
		name: &str,
		sp: &str,
		status: MyBattleStatus,
		colour: u32,
	) {
		room.connect(peer, IP);
		room.apply(
			peer,
			&format!("LOGIN {name} * 0 * modlobby:0.1\tx\tb sp"),
			IP,
		);
		room.apply(peer, &format!("JOINBATTLE 1 empty {sp}"), IP);
		room.apply(
			peer,
			&format!("MYBATTLESTATUS {} {colour}", status.bits()),
			IP,
		);
	}

	/// Two people against an AI: the founder and a guest on side 2 of the
	/// room's numbering, the guest's AI on side 4, nobody on 0, 1 or 3 -- the
	/// gaps the engine cannot have. The guest arrived first.
	#[test]
	fn the_script_seats_everyone_with_their_passwords_and_no_gaps() {
		let mut room = Room::new(Config {
			founder: "ann".into(),
			title: "t".into(),
			engine_version: "2026.07.04".into(),
			game: "BAR test-1".into(),
			map: "Comet Catcher".into(),
			max_players: 8,
			policy: Policy::Open,
		});
		seated(
			&mut room,
			2,
			"bob",
			"4242",
			MyBattleStatus::player(Sync::Synced, 1, 2).side(1),
			255,
		);
		seated(
			&mut room,
			1,
			"ann",
			"1111",
			MyBattleStatus::player(Sync::Synced, 0, 2).handicap(10),
			16,
		);
		room.apply(
			2,
			&format!(
				"ADDBOT Bot1 {} 99 BARb",
				MyBattleStatus::player(Sync::Bot, 2, 4).bits()
			),
			IP,
		);
		room.connect(3, IP);
		room.apply(3, "LOGIN cat * 0 * modlobby:0.1\tx\tb sp", IP);
		room.apply(3, "JOINBATTLE 1 empty 3333", IP);
		room.apply(1, "SAYBATTLE !bSet startmetal 1500", IP);
		room.apply(1, "SAYBATTLE !set startPosType 1", IP);

		let script = room.host_script();
		assert_eq!(script.player, "ann");
		assert_eq!(
			(script.host_ip.as_str(), script.host_port),
			("0.0.0.0", 8452)
		);
		assert_eq!(
			script
				.players
				.iter()
				.map(|p| p.name.as_str())
				.collect::<Vec<_>>(),
			["ann", "bob", "cat"],
			"the founder first, then arrivals in order"
		);
		assert_eq!(script.players[0].password.as_deref(), Some("1111"));
		assert_eq!(script.players[1].password.as_deref(), Some("4242"));
		assert_eq!(script.players[2].team, None, "a spectator holds no team");
		// Sides 2 and 4 became ally teams 0 and 1.
		assert_eq!(script.ally_teams.len(), 2);
		assert_eq!(script.teams.len(), 3);
		assert_eq!(script.teams[0].ally_team, 0);
		assert_eq!(script.teams[0].handicap, 10);
		assert_eq!(script.teams[0].leader, 0);
		assert_eq!(script.teams[1].side.as_deref(), Some("Cortex"));
		assert_eq!(script.teams[1].leader, 1, "bob's own seat");
		assert_eq!(script.teams[2].ally_team, 1);
		assert_eq!(script.teams[2].leader, 1, "bob runs his AI");
		assert_eq!(script.ais[0].host, 1);
		assert_eq!(script.ais[0].team, 2);
		assert_eq!(script.start_pos, StartPos::Random);
		assert_eq!(
			script.modoptions,
			[("startmetal".to_owned(), "1500".to_owned())]
		);
		let text = script.script();
		assert!(text.contains("hostip = 0.0.0.0;"));
		assert!(text.contains("name = bob;\n\t\tpassword = 4242;"));
		assert!(text.contains("numplayers = 3;"));
		assert!(text.contains("numusers = 4;"));
	}
}
