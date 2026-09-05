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

const CHARACTER_SCHEMA_VERSION: u32 = 3;
const CHARACTER_FILE_NAME: &str = ".model.json";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CharacterFileDocument {
    schema_version: u32,
    character: CharacterFile,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CharacterFile {
    id: String,
    name: String,
    state: CharacterState,
    spec_path: Option<String>,
    render_path: Option<String>,
    view_paths: BTreeMap<String, String>,
    hard_constraints: Vec<Value>,
    gate_spec_confirmed_at: Option<i64>,
    gate_render_confirmed_at: Option<i64>,
    gate_views_confirmed_at: Option<i64>,
    created_at: i64,
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
    let document = CharacterFileDocument {
        schema_version: CHARACTER_SCHEMA_VERSION,
        character: CharacterFile::from(character),
    };
    let content = serde_json::to_string_pretty(&document)
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
            hard_constraints: character.hard_constraints.clone(),
            gate_spec_confirmed_at: character.gate_spec_confirmed_at,
            gate_render_confirmed_at: character.gate_render_confirmed_at,
            gate_views_confirmed_at: character.gate_views_confirmed_at,
            created_at: character.created_at,
            updated_at: character.updated_at,
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
    let document: CharacterFileDocument = serde_json::from_str(&content)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    if document.schema_version < 2 || document.character.id.trim().is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "unsupported character metadata",
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
    let metadata = document.character;
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
        hard_constraints: metadata.hard_constraints,
        gate_spec_confirmed_at: metadata.gate_spec_confirmed_at,
        gate_render_confirmed_at: metadata.gate_render_confirmed_at,
        gate_views_confirmed_at: metadata.gate_views_confirmed_at,
        created_at: metadata.created_at,
        updated_at: metadata.updated_at,
    })
}

#[cfg(test)]
#[path = "character_files_tests.rs"]
mod tests;
