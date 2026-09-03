use codex_game_domain::ArtBibleVersion;
use codex_game_domain::ArtBibleVersionId;
use codex_game_domain::BackendStatus;
use codex_game_domain::ProjectId;
use codex_game_store::ProjectAccess;
use codex_game_store::ProjectStore;
use codex_game_store::StoreError;
use codex_game_store::StoreState;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tempfile::TempDir;
use thiserror::Error;
use uuid::Uuid;

const PASSTHROUGH_DIRECTORIES: [&str; 2] = ["memory", "prompts"];
const PLACEHOLDER_DIRECTORIES: [&str; 3] = ["equipment", "maps", "scenes"];
const IGNORED_DATABASES: [&str; 3] = ["config.db", "runtime.db", ".atelier/project.db"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub character_directories: Vec<PathBuf>,
    pub copied_directories: Vec<PathBuf>,
    pub skipped_paths: Vec<PathBuf>,
    pub warnings: Vec<String>,
    pub imported_art_bible: bool,
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("source project does not exist: {0}")]
    MissingSource(PathBuf),
    #[error("destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("invalid project.json: {0}")]
    InvalidProjectJson(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("project store error: {0}")]
    Store(#[from] StoreError),
}

pub struct LegacyProjectImporter;

impl LegacyProjectImporter {
    pub async fn import(source: &Path, destination: &Path) -> Result<ImportReport, ImportError> {
        if !source.is_dir() {
            return Err(ImportError::MissingSource(source.to_path_buf()));
        }
        if destination.exists() {
            return Err(ImportError::DestinationExists(destination.to_path_buf()));
        }
        let parent = destination.parent().ok_or_else(|| {
            ImportError::Io(io::Error::new(
                ErrorKind::InvalidInput,
                "destination has no parent",
            ))
        })?;
        fs::create_dir_all(parent)?;
        let staging = TempDir::new_in(parent)?;
        let staging_root = staging.path().join("project");
        fs::create_dir_all(&staging_root)?;

        let mut report = ImportReport {
            source: source.to_path_buf(),
            destination: destination.to_path_buf(),
            character_directories: Vec::new(),
            copied_directories: Vec::new(),
            skipped_paths: IGNORED_DATABASES.iter().map(PathBuf::from).collect(),
            warnings: Vec::new(),
            imported_art_bible: false,
        };

        copy_project_json(source, &staging_root)?;
        let art_bible = source.join("art-bible.md");
        if art_bible.is_file() {
            fs::copy(&art_bible, staging_root.join("art-bible.md"))?;
            report.imported_art_bible = true;
        }
        for directory in PASSTHROUGH_DIRECTORIES {
            let source_directory = source.join(directory);
            let destination_directory = staging_root.join(directory);
            if source_directory.is_dir() {
                copy_directory(&source_directory, &destination_directory)?;
                report.copied_directories.push(PathBuf::from(directory));
            } else {
                fs::create_dir_all(destination_directory)?;
            }
        }
        for directory in PLACEHOLDER_DIRECTORIES {
            fs::create_dir_all(staging_root.join(directory))?;
            if source.join(directory).is_dir() {
                report.skipped_paths.push(PathBuf::from(directory));
            }
        }
        fs::create_dir_all(staging_root.join("characters"))?;
        import_characters(source, &staging_root, &mut report)?;
        fs::create_dir_all(staging_root.join(".codex-game"))?;
        fs::write(
            staging_root.join(".codex-game/import-report.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        let store = ProjectStore::open(&staging_root).await?;
        if store.access() != ProjectAccess::ReadWrite {
            return Err(ImportError::Io(io::Error::other(
                "staged project store unexpectedly opened read-only",
            )));
        }
        if report.imported_art_bible {
            let project: Value =
                serde_json::from_slice(&fs::read(staging_root.join("project.json"))?)?;
            let project_id = project
                .get("projectId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ImportError::InvalidProjectJson("projectId was not generated".to_string())
                })?;
            let markdown = fs::read_to_string(staging_root.join("art-bible.md"))?;
            store
                .insert_art_bible_version(
                    &ArtBibleVersion {
                        id: ArtBibleVersionId::new(Uuid::now_v7().to_string()),
                        project_id: ProjectId::new(project_id),
                        version: 1,
                        content_hash: format!("{:x}", Sha256::digest(markdown.as_bytes())),
                        source_artifact_ids: Vec::new(),
                        created_at: now(),
                    },
                    &markdown,
                )
                .await?;
        }
        store.close().await;
        fs::rename(&staging_root, destination)?;
        Ok(report)
    }
}

fn copy_project_json(source: &Path, destination: &Path) -> Result<(), ImportError> {
    let source_path = source.join("project.json");
    let contents = fs::read_to_string(&source_path)?;
    let mut project = serde_json::from_str::<Value>(&contents)?;
    let object = project.as_object_mut().ok_or_else(|| {
        ImportError::InvalidProjectJson("project.json must contain an object".to_string())
    })?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ImportError::InvalidProjectJson("project.json must contain a name".to_string())
        })?
        .to_string();
    if let Some(code) = object
        .remove("code")
        .and_then(|value| value.as_str().map(str::to_string))
    {
        object.entry("alias").or_insert_with(|| Value::String(code));
    }
    object
        .entry("displayName")
        .or_insert_with(|| Value::String(name));
    object.insert(
        "projectId".to_string(),
        Value::String(Uuid::now_v7().to_string()),
    );
    object.insert(
        "state".to_string(),
        Value::String(
            if source.join("art-bible.md").is_file() {
                "versioned"
            } else {
                "unversioned"
            }
            .to_string(),
        ),
    );
    fs::write(
        destination.join("project.json"),
        serde_json::to_vec_pretty(&project)?,
    )?;
    Ok(())
}

fn import_characters(
    source: &Path,
    destination: &Path,
    report: &mut ImportReport,
) -> Result<(), ImportError> {
    let source_characters = source.join("characters");
    if !source_characters.is_dir() {
        return Ok(());
    }
    let destination_characters = destination.join("characters");
    for directory in directories_with_marker(&source_characters, ".model.json")? {
        let relative = directory
            .strip_prefix(&source_characters)
            .map_err(io::Error::other)?;
        let target = destination_characters.join(relative);
        copy_directory(&directory, &target)?;
        let metadata = match fs::read_to_string(directory.join("meta.json")) {
            Ok(contents) => serde_json::from_str::<Value>(&contents).unwrap_or_else(|_| {
                report
                    .warnings
                    .push(format!("invalid meta.json in {}", relative.display()));
                Value::Object(Map::new())
            }),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                report
                    .warnings
                    .push(format!("missing meta.json in {}", relative.display()));
                Value::Object(Map::new())
            }
            Err(err) => return Err(ImportError::Io(err)),
        };
        fs::write(
            target.join("entity.json"),
            serde_json::to_vec_pretty(&metadata)?,
        )?;
        report.character_directories.push(relative.to_path_buf());
    }
    Ok(())
}

fn directories_with_marker(root: &Path, marker: &str) -> io::Result<Vec<PathBuf>> {
    let mut matches = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if entry.file_name() == marker
                && let Some(parent) = path.parent()
            {
                matches.push(parent.to_path_buf());
            }
        }
    }
    matches.sort();
    matches.dedup();
    Ok(matches)
}

fn copy_directory(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

/// Returns the store state used while an import is being prepared.
pub fn preparing_store_state() -> StoreState {
    StoreState::new(BackendStatus::Starting)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn imports_assets_and_replaces_legacy_identity() {
        let directory = tempfile::tempdir().expect("tempdir");
        let source = directory.path().join("legacy");
        let destination = directory.path().join("imported");
        fs::create_dir_all(source.join("memory")).expect("memory");
        fs::write(
            source.join("project.json"),
            r#"{"name":"Legacy","code":"old-code","unknown":true}"#,
        )
        .expect("project");
        fs::write(source.join("art-bible.md"), "# Existing\n").expect("art bible");
        fs::create_dir_all(source.join("maps")).expect("legacy maps");
        fs::write(source.join("maps/legacy.json"), "{}").expect("legacy map");
        let report = LegacyProjectImporter::import(&source, &destination)
            .await
            .expect("import");
        assert!(report.imported_art_bible);
        let project: Value = serde_json::from_slice(
            &fs::read(destination.join("project.json")).expect("read project"),
        )
        .expect("parse project");
        assert_eq!(project["alias"], "old-code");
        assert!(project.get("code").is_none());
        assert!(project["projectId"].as_str().is_some());
        assert_eq!(project["state"], "versioned");
        assert_eq!(project["unknown"], true);
        assert!(destination.join("characters").is_dir());
        assert!(destination.join("maps").is_dir());
        assert!(!destination.join("maps/legacy.json").exists());
        assert!(destination.join(".codex-game/project.db").is_file());
        assert!(!destination.join("config.db").exists());
        let store = ProjectStore::open(&destination)
            .await
            .expect("open imported store");
        let versions = store
            .load_art_bible_versions(project["projectId"].as_str().expect("project id"))
            .await
            .expect("load imported Art Bible");
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].1, "# Existing\n");
        store.close().await;
    }

    #[tokio::test]
    async fn invalid_import_does_not_create_the_destination() {
        let directory = tempfile::tempdir().expect("tempdir");
        let source = directory.path().join("legacy");
        let destination = directory.path().join("imported");
        fs::create_dir_all(&source).expect("source");
        fs::write(source.join("project.json"), "[]").expect("project");
        assert!(
            LegacyProjectImporter::import(&source, &destination)
                .await
                .is_err()
        );
        assert!(!destination.exists());
    }
}
