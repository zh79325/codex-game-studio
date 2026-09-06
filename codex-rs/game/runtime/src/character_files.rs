use codex_game_domain::Character;
use codex_game_domain::CharacterState;
use codex_game_domain::Project;
use codex_game_store::write_art_bible;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

const CHARACTER_FILE_NAME: &str = ".model.json";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CharacterFileDocument {
    schema_version: u32,
    character: LegacyCharacterFile,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CharacterFile {
    id: String,
    name: String,
    state: CharacterState,
    spec_path: Option<String>,
    render_path: Option<String>,
    view_paths: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyCharacterFile {
    id: String,
    name: String,
    state: CharacterState,
    spec_path: Option<String>,
    render_path: Option<String>,
    #[serde(default)]
    view_paths: BTreeMap<String, String>,
    #[serde(default)]
    hard_constraints: Vec<Value>,
    gate_spec_confirmed_at: Option<i64>,
    gate_render_confirmed_at: Option<i64>,
    gate_views_confirmed_at: Option<i64>,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    updated_at: i64,
}

pub(crate) fn character_file_path(project: &Project, character: &Character) -> PathBuf {
    Path::new(&project.root)
        .join(&character.dir_name)
        .join(CHARACTER_FILE_NAME)
}

pub(crate) fn read_project_characters(project: &Project) -> io::Result<Vec<Character>> {
    let root = Path::new(&project.root);
    let characters_root = root.join("characters");
    if !characters_root.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_character_files(&characters_root, &mut files)?;
    files.sort();
    let mut ids = BTreeSet::new();
    let mut characters = Vec::with_capacity(files.len());
    for path in files {
        let character = read_character_file(project, &path)?;
        if !ids.insert(character.id.clone()) {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!("duplicate character id: {}", character.id),
            ));
        }
        characters.push(character);
    }
    Ok(characters)
}

pub(crate) fn write_character_file(project: &Project, character: &Character) -> io::Result<()> {
    let path = character_file_path(project, character);
    if path.is_file() {
        let existing = read_character_file(project, &path)?;
        if existing.id != character.id {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "character id cannot be changed",
            ));
        }
    }
    let content = serde_json::to_string_pretty(&CharacterFile::from(character))
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    write_art_bible(&path, &format!("{content}\n"))
}

impl From<&Character> for CharacterFile {
    fn from(character: &Character) -> Self {
        Self {
            id: character.id.clone(),
            name: character.name.clone(),
            state: character.state,
            spec_path: character.spec_path.clone(),
            render_path: character.render_path.clone(),
            view_paths: character.view_paths.clone(),
        }
    }
}

fn collect_character_files(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_character_files(&path, files)?;
        } else if entry.file_name() == CHARACTER_FILE_NAME {
            files.push(path);
        }
    }
    Ok(())
}

fn read_character_file(project: &Project, path: &Path) -> io::Result<Character> {
    let content = fs::read_to_string(path)?;
    let (metadata, hard_constraints, gate_spec, gate_render, gate_views, created_at, updated_at) =
        match serde_json::from_str::<CharacterFile>(&content) {
            Ok(metadata) => {
                let timestamp = fs::metadata(path)?
                    .modified()
                    .ok()
                    .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|value| value.as_secs() as i64)
                    .unwrap_or_default();
                let gate_spec =
                    (metadata.state != CharacterState::S0SpecDrafting).then_some(timestamp);
                let gate_render = matches!(
                    metadata.state,
                    CharacterState::S3RenderConfirmed
                        | CharacterState::S4ViewsGenerated
                        | CharacterState::S5ViewsConfirmed
                )
                .then_some(timestamp);
                let gate_views =
                    (metadata.state == CharacterState::S5ViewsConfirmed).then_some(timestamp);
                (
                    metadata,
                    Vec::new(),
                    gate_spec,
                    gate_render,
                    gate_views,
                    timestamp,
                    timestamp,
                )
            }
            Err(flat_error) => {
                let document: CharacterFileDocument = serde_json::from_str(&content).map_err(
                    |legacy_error| {
                        io::Error::new(
                            ErrorKind::InvalidData,
                            format!(
                                "invalid flat character metadata ({flat_error}); invalid legacy metadata ({legacy_error})"
                            ),
                        )
                    },
                )?;
                if document.schema_version < 2 || document.character.id.trim().is_empty() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidData,
                        "unsupported character metadata",
                    ));
                }
                let legacy = document.character;
                (
                    CharacterFile {
                        id: legacy.id,
                        name: legacy.name,
                        state: legacy.state,
                        spec_path: legacy.spec_path,
                        render_path: legacy.render_path,
                        view_paths: legacy.view_paths,
                    },
                    legacy.hard_constraints,
                    legacy.gate_spec_confirmed_at,
                    legacy.gate_render_confirmed_at,
                    legacy.gate_views_confirmed_at,
                    legacy.created_at,
                    legacy.updated_at,
                )
            }
        };
    if metadata.id.trim().is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "character id cannot be empty",
        ));
    }
    let root = Path::new(&project.root);
    let character_dir = path
        .parent()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "character file has no directory"))?;
    let dir_name = character_dir
        .strip_prefix(root)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?
        .to_string_lossy()
        .into_owned();
    let relative_to_characters = character_dir
        .strip_prefix(root.join("characters"))
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    let group = relative_to_characters
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.to_string_lossy().into_owned());
    Ok(Character {
        id: metadata.id,
        project_id: project.id.as_str().to_string(),
        name: metadata.name,
        group,
        dir_name,
        state: metadata.state,
        spec_path: metadata.spec_path,
        render_path: metadata.render_path,
        view_paths: metadata.view_paths,
        hard_constraints,
        gate_spec_confirmed_at: gate_spec,
        gate_render_confirmed_at: gate_render,
        gate_views_confirmed_at: gate_views,
        created_at,
        updated_at,
    })
}

#[cfg(test)]
#[path = "character_files_tests.rs"]
mod tests;
