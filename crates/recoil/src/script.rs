//! Start scripts, for a game with no lobby behind it.
//!
//! The engine takes a `script.txt` where it would take a `spring://` URL, and
//! plays whatever it describes. That is how every lobby runs a skirmish
//! (`bar-lobby/src/main/utils/start-script-converter.ts`), and the format is
//! documented at <https://springrts.com/wiki/Script.txt>.

use std::fmt::Write as _;

/// Where players start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartPos {
    /// Wherever the map's own start positions say.
    Fixed = 0,
    Random = 1,
    /// Placed by each player once the game has loaded.
    InGame = 2,
}

/// One AI opponent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ai {
    /// The engine's name for it: `BARb`, `CircuitAI`, `NullAI`.
    pub short_name: String,
    /// What it is called in game.
    pub name: String,
}

/// A game to start without a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skirmish {
    /// The game's full version, as rapid names it.
    pub game: String,
    /// The map's spring name, not its filename.
    pub map: String,
    pub player: String,
    pub start_pos: StartPos,
    /// One opponent per entry, each on its own ally team.
    pub opponents: Vec<Ai>,
    /// `key = value` pairs written into `[modoptions]`.
    pub modoptions: Vec<(String, String)>,
}

impl Skirmish {
    /// The script the engine reads.
    ///
    /// Everyone is on their own ally team, us as ally team 0 and each opponent
    /// after. An AI's `host` is the *player index* that owns it, which is
    /// always us — there is nobody else here to own one.
    pub fn script(&self) -> String {
        let mut out = String::from("[game] {\n");
        let _ = writeln!(out, "\tgametype = {};", self.game);
        let _ = writeln!(out, "\tmapname = {};", self.map);
        let _ = writeln!(out, "\tishost = 1;");
        let _ = writeln!(out, "\tmyplayername = {};", self.player);
        let _ = writeln!(out, "\tstartpostype = {};", self.start_pos as u8);
        let _ = writeln!(out, "\tnumusers = {};", 1 + self.opponents.len());

        if !self.modoptions.is_empty() {
            out.push_str("\n\t[modoptions] {\n");
            for (key, value) in &self.modoptions {
                let _ = writeln!(out, "\t\t{key} = {value};");
            }
            out.push_str("\t}\n");
        }

        for ally in 0..=self.opponents.len() {
            let _ = write!(out, "\n\t[allyteam{ally}] {{\n\t\tnumallies = 0;\n\t}}\n");
        }

        // Team 0 is ours; team n belongs to opponent n-1.
        for team in 0..=self.opponents.len() {
            let _ = write!(
                out,
                "\n\t[team{team}] {{\n\t\tallyteam = {team};\n\t\tteamleader = 0;\n\t}}\n"
            );
        }

        let _ = write!(
            out,
            "\n\t[player0] {{\n\t\tname = {};\n\t\tteam = 0;\n\t}}\n",
            self.player
        );

        for (index, ai) in self.opponents.iter().enumerate() {
            let _ = write!(
                out,
                "\n\t[ai{index}] {{\n\t\tname = {};\n\t\tshortname = {};\n\t\tteam = {};\n\t\thost = 0;\n\t}}\n",
                ai.name,
                ai.short_name,
                index + 1
            );
        }

        out.push_str("}\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_opponent() -> Skirmish {
        Skirmish {
            game: "Beyond All Reason test-31134".into(),
            map: "Supreme Isthmus v2.1".into(),
            player: "tetrisface".into(),
            start_pos: StartPos::InGame,
            opponents: vec![Ai {
                short_name: "BARb".into(),
                name: "BARbarian".into(),
            }],
            modoptions: vec![],
        }
    }

    #[test]
    fn a_one_on_one_names_both_sides_and_who_owns_the_ai() {
        let script = one_opponent().script();
        assert!(script.contains("gametype = Beyond All Reason test-31134;"));
        assert!(script.contains("mapname = Supreme Isthmus v2.1;"));
        assert!(script.contains("myplayername = tetrisface;"));
        assert!(script.contains("startpostype = 2;"));
        assert!(script.contains("numusers = 2;"));

        // We are player 0 on team 0; the AI is on team 1 and we own it.
        assert!(script.contains("[player0] {\n\t\tname = tetrisface;\n\t\tteam = 0;"));
        assert!(script.contains("shortname = BARb;"));
        assert!(script.contains("\t\tteam = 1;\n\t\thost = 0;"));
    }

    #[test]
    fn everyone_gets_their_own_ally_team() {
        let mut skirmish = one_opponent();
        skirmish.opponents.push(Ai {
            short_name: "CircuitAI".into(),
            name: "Circuit".into(),
        });
        let script = skirmish.script();

        for ally in ["[allyteam0]", "[allyteam1]", "[allyteam2]"] {
            assert!(script.contains(ally), "missing {ally}");
        }
        assert!(!script.contains("[allyteam3]"));
        assert!(script.contains("numusers = 3;"));
    }

    #[test]
    fn modoptions_are_written_only_when_there_are_some() {
        assert!(!one_opponent().script().contains("[modoptions]"));

        let mut skirmish = one_opponent();
        skirmish.modoptions = vec![("ranked_game".into(), "0".into())];
        let script = skirmish.script();
        assert!(script.contains("[modoptions] {"));
        assert!(script.contains("ranked_game = 0;"));
    }

    #[test]
    fn an_opponent_free_game_is_still_a_valid_script() {
        let mut skirmish = one_opponent();
        skirmish.opponents.clear();
        let script = skirmish.script();
        assert!(script.contains("numusers = 1;"));
        assert!(script.contains("[allyteam0]"));
        assert!(!script.contains("[ai0]"));
        assert!(script.starts_with("[game] {"));
        assert!(script.ends_with("}\n"));
    }
}
