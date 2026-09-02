//! What the game and the engine can tell a tweak's author.
//!
//! The engine dumps its weapon tags with `spring --list-def-tags`
//! (`rts/Sim/Misc/DefinitionTag.cpp`): name, type, default, bounds and a
//! description for each. Unit tags have no such registry -- `UnitDef.cpp`
//! reads them by hand -- so the unit side is the names a game has, which is
//! what a `tweakunits` key must be for `unitdefs_post.lua` to apply it at all.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One tag the engine reads under `weapondefs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Tag {
    pub name: String,
    /// The engine's type, in plain words: `float`, `bool`, `string`, `table`...
    pub kind: String,
    pub default: Option<String>,
    pub description: Option<String>,
    pub min: Option<String>,
    pub max: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DefTags {
    pub weapon: Vec<Tag>,
}

/// Reads the engine's dump. Only the sections it has; an engine without a
/// section yields an empty list rather than an error.
pub fn parse_def_tags(json: &str) -> Result<DefTags, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    Ok(DefTags {
        weapon: tags_of(value.get("WeaponDefs")),
    })
}

fn tags_of(section: Option<&serde_json::Value>) -> Vec<Tag> {
    let Some(map) = section.and_then(serde_json::Value::as_object) else {
        return Vec::new();
    };
    let mut tags: Vec<Tag> = map
        .iter()
        .map(|(name, meta)| Tag {
            name: name.clone(),
            kind: kind_of(text_of(meta.get("type")).as_deref()),
            default: text_of(meta.get("defaultValue")),
            description: text_of(meta.get("description")),
            min: text_of(meta.get("minimumValue")),
            max: text_of(meta.get("maximumValue")),
        })
        .collect();
    tags.sort_by_key(|tag| tag.name.to_lowercase());
    tags
}

/// A JSON value as the text an editor would show; numbers and bools as written.
fn text_of(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(text)) => Some(text.clone()),
        Some(other) => Some(other.to_string()),
    }
}

fn kind_of(engine_type: Option<&str>) -> String {
    match engine_type {
        Some("std::string") => "string".to_owned(),
        Some(other) => other.to_owned(),
        None => "unknown".to_owned(),
    }
}

/// The units a game has: the stem of every `units/**/*.lua`, lowercased as
/// the engine keys `UnitDefs`, sorted and without repeats.
pub fn unit_names<S: AsRef<str>>(files: &[S]) -> Vec<String> {
    let mut names: Vec<String> = files
        .iter()
        .map(AsRef::as_ref)
        .filter(|file| {
            let lower = file.to_ascii_lowercase();
            lower.starts_with("units/") && lower.ends_with(".lua")
        })
        .filter_map(|file| {
            let stem = file.rsplit(['/', '\\']).next()?;
            Some(stem[..stem.len() - ".lua".len()].to_ascii_lowercase())
        })
        .filter(|name| !name.is_empty())
        .collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_engines_dump_shape() {
        let json = r#"{
  "WeaponDefs": {
    "range": { "description": "How far.", "defaultValue": 0, "minimumValue": 0, "type": "float" },
    "name": { "internalName": "description", "defaultValue": "Weapon", "type": "std::string" },
    "burst": { "type": "int", "defaultValue": 1, "maximumValue": 99 },
    "customParams": { "type": "table" }
  }
}"#;
        let tags = parse_def_tags(json).unwrap();
        let names: Vec<&str> = tags.weapon.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["burst", "customParams", "name", "range"]);
        let range = &tags.weapon[3];
        assert_eq!(range.kind, "float");
        assert_eq!(range.default.as_deref(), Some("0"));
        assert_eq!(range.min.as_deref(), Some("0"));
        assert_eq!(range.description.as_deref(), Some("How far."));
        assert_eq!(tags.weapon[2].kind, "string");
        assert_eq!(tags.weapon[2].default.as_deref(), Some("Weapon"));
        assert_eq!(tags.weapon[1].default, None);
    }

    #[test]
    fn a_dump_without_weapons_is_empty_not_wrong() {
        assert_eq!(parse_def_tags("{}").unwrap(), DefTags::default());
        assert!(parse_def_tags("not json").is_err());
    }

    #[test]
    fn unit_names_are_lowercased_stems_of_the_units_folder() {
        let files = [
            "units/ArmCom.lua",
            "units/Scavs/corgolt4_scav.lua",
            "Units/other/CorAk.lua",
            "units/readme.txt",
            "gamedata/unitdefs_post.lua",
            "units/armcom.lua",
        ];
        assert_eq!(unit_names(&files), vec!["armcom", "corak", "corgolt4_scav"]);
    }
}
