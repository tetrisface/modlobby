//! The commands behind the preset table.
//!
//! Kept out of `commands.rs` because there are a dozen of them and they share
//! two things nothing else needs: where Chobby keeps its own file, and how to
//! read the room we are standing in.

use std::path::PathBuf;

use presets::{Book, Preset, Sections, Stamp};
use tauri::State;

use crate::commands::{ApiError, Result};
use crate::state::App;

impl From<presets::Error> for ApiError {
    fn from(err: presets::Error) -> Self {
        let code = match err {
            presets::Error::Unknown(_) => "missing",
            presets::Error::Duplicate(_) => "duplicate",
            presets::Error::Invalid { .. } => "invalid",
            presets::Error::Io { .. } => "io",
        };
        ApiError::new(code, err.to_string())
    }
}

impl From<content::demo::Error> for ApiError {
    fn from(err: content::demo::Error) -> Self {
        ApiError::new("replay", err.to_string())
    }
}

fn now() -> Stamp {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

/// Where Chobby keeps its presets, under the data directory in use.
///
/// The same file the game itself reads, so importing and exporting land where
/// the other client will see them without anyone typing a path.
fn chobby_file(app: &App) -> Option<PathBuf> {
    let dir = app
        .settings
        .get()
        .paths
        .data_dir
        .or_else(lobby_runtime::launch::default_data_dir)?;
    Some(dir.join("optionsPresets.json"))
}

#[tauri::command]
pub fn list_presets(app: State<'_, App>) -> Result<Book> {
    Ok(app.presets.load()?)
}

/// Where the Chobby file is, so the front end can say so before touching it.
#[tauri::command]
pub fn chobby_presets_path(app: State<'_, App>) -> Option<String> {
    chobby_file(&app).map(|path| path.display().to_string())
}

/// The current room, as a preset.
///
/// The modoptions come from the room's script tags, which is the server's own
/// statement of what it is set to — not from anything we remember sending.
#[tauri::command]
pub async fn save_preset(app: State<'_, App>, name: String) -> Result<Book> {
    let snapshot = app.client.snapshot().await?;
    let Some(my) = snapshot.my_battle.as_ref() else {
        return Err(ApiError::new("not in a room", "join a room to save it"));
    };
    let room = snapshot
        .battles
        .iter()
        .find(|battle| battle.id == my.id)
        .ok_or_else(|| ApiError::new("not in a room", "the room is not in the list"))?;

    let mut preset = Preset::new(name, now());
    preset.map = Some(room.map_name.clone());
    preset.modoptions = my
        .script_tags
        .iter()
        .filter_map(|(key, value)| {
            Some((
                key.strip_prefix("game/modoptions/")?.to_owned(),
                value.clone(),
            ))
        })
        .collect();
    if let Some(layout) = &room.layout {
        preset
            .battle
            .insert("teamSize".into(), layout.team_size.to_string());
        preset
            .battle
            .insert("nbTeams".into(), layout.teams.to_string());
    }
    preset
        .battle
        .insert("locked".into(), u8::from(room.locked).to_string());
    preset.start_boxes = room
        .start_rects
        .iter()
        .map(|rect| {
            (
                rect.ally_team,
                presets::StartBox {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                },
            )
        })
        .collect();

    Ok(app.presets.put(preset, now())?)
}

/// A preset made from the game a replay recorded.
#[tauri::command]
pub fn preset_from_replay(app: State<'_, App>, path: String, name: String) -> Result<Book> {
    let text = content::demo::script(&path)?;
    let script = recoil::script_read::parse(&text);

    let mut preset = Preset::new(name, now());
    preset.map = script.map().map(str::to_owned);
    preset.modoptions = script.modoptions.clone();
    preset.start_boxes = script
        .boxes_out_of_200()
        .into_iter()
        .map(|(ally, (left, top, right, bottom))| {
            (
                ally,
                presets::StartBox {
                    left,
                    top,
                    right,
                    bottom,
                },
            )
        })
        .collect();

    Ok(app.presets.put(preset, now())?)
}

#[tauri::command]
pub fn delete_preset(app: State<'_, App>, name: String) -> Result<Book> {
    Ok(app.presets.remove(&name)?)
}

#[tauri::command]
pub fn rename_preset(app: State<'_, App>, from: String, to: String) -> Result<Book> {
    Ok(app.presets.rename(&from, &to)?)
}

/// What applying a preset would send, without sending it.
#[tauri::command]
pub async fn plan_preset(
    app: State<'_, App>,
    name: String,
    sections: Sections,
) -> Result<presets::Plan> {
    let preset = one(&app, &name)?;
    Ok(presets::plan(&preset, &current_room(&app).await?, sections))
}

/// Sends a preset to the room.
///
/// Every line is queued through the ordinary command throttle rather than
/// blasted: SPADS ignores a client for four minutes after eight commands in
/// eight seconds, so a big preset takes the couple of minutes it takes.
#[tauri::command]
pub async fn apply_preset(
    app: State<'_, App>,
    name: String,
    sections: Sections,
) -> Result<presets::Plan> {
    let preset = one(&app, &name)?;
    let plan = presets::plan(&preset, &current_room(&app).await?, sections);

    for line in &plan.lines {
        app.client.say(line.clone()).await?;
    }

    app.presets.touch(&name, now())?;
    Ok(plan)
}

/// What BAR's PvE Stats service says the current room scores.
///
/// Answers `None` rather than an error for a room that is not PvE, or when the
/// setting is off: neither is a failure, and a panel that says nothing is the
/// right outcome for both.
#[tauri::command]
pub async fn pve_score(app: State<'_, App>) -> Result<Option<pve::Score>> {
    if !app.settings.get().play.pve_stats {
        return Ok(None);
    }
    let snapshot = app.client.snapshot().await?;
    let Some(my) = snapshot.my_battle.as_ref() else {
        return Ok(None);
    };
    let Some(room) = snapshot.battles.iter().find(|battle| battle.id == my.id) else {
        return Ok(None);
    };

    let ai_names: Vec<String> = room.bots.iter().map(|bot| bot.ai.clone()).collect();
    let Some(kind) = pve::ai_type(&ai_names) else {
        return Ok(None);
    };

    let ask = pve::Ask {
        ai_type: kind.as_str(),
        map: room.map_name.clone(),
        game_settings: my
            .script_tags
            .iter()
            .filter_map(|(key, value)| {
                Some((
                    key.strip_prefix("game/modoptions/")?.to_owned(),
                    value.clone(),
                ))
            })
            .collect(),
        encounter_context: pve::Encounter {
            human_team_size: room.player_count,
            enemy_ai_count: matches!(kind, pve::AiType::Barbarian)
                .then_some(room.bots.len() as u32),
            // One per seated human. The service derives its `Player Handicap`
            // column from the average of these, and a governed column it
            // cannot derive counts as missing rather than defaulted — which is
            // what makes it decline to score the room at all.
            human_player_income_multipliers: room
                .members
                .iter()
                .filter_map(|name| snapshot.users.iter().find(|user| &user.name == name))
                .filter_map(|user| user.battle_status.as_ref())
                .filter(|status| status.player)
                .map(|status| pve::income_multiplier(status.handicap))
                .collect(),
        },
    };

    pve::fetch(&ask)
        .await
        .map(Some)
        .map_err(|err| ApiError::new("pve", err.to_string()))
}

/// Brings in everything from Chobby's file.
#[tauri::command]
pub fn import_presets(app: State<'_, App>, path: Option<String>) -> Result<Imported> {
    let path = path
        .map(PathBuf::from)
        .or_else(|| chobby_file(&app))
        .ok_or_else(|| ApiError::new("no path", "no BAR data directory to look in"))?;
    let (book, skipped) = app.presets.import_chobby(&path, now())?;
    Ok(Imported { book, skipped })
}

#[derive(serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Imported {
    pub book: Book,
    /// Presets in their file whose name we already had, and so left alone.
    pub skipped: usize,
}

/// Writes presets back into Chobby's file, leaving its other entries alone.
#[tauri::command]
pub fn export_presets(
    app: State<'_, App>,
    path: Option<String>,
    names: Vec<String>,
) -> Result<usize> {
    let path = path
        .map(PathBuf::from)
        .or_else(|| chobby_file(&app))
        .ok_or_else(|| ApiError::new("no path", "no BAR data directory to write to"))?;
    Ok(app.presets.export_chobby(&path, &names)?)
}

fn one(app: &App, name: &str) -> Result<Preset> {
    app.presets
        .load()?
        .presets
        .into_iter()
        .find(|preset| preset.name == name)
        .ok_or_else(|| ApiError::new("missing", format!("no preset called {name}")))
}

/// The room as it stands, so a plan can leave out what is already true.
async fn current_room(app: &App) -> Result<presets::Room> {
    let snapshot = app.client.snapshot().await?;
    let Some(my) = snapshot.my_battle.as_ref() else {
        return Ok(presets::Room::default());
    };
    Ok(presets::Room {
        map: snapshot
            .battles
            .iter()
            .find(|battle| battle.id == my.id)
            .map(|battle| battle.map_name.clone()),
        modoptions: my
            .script_tags
            .iter()
            .filter_map(|(key, value)| {
                Some((
                    key.strip_prefix("game/modoptions/")?.to_owned(),
                    value.clone(),
                ))
            })
            .collect(),
    })
}
