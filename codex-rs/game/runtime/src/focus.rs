use codex_game_domain::Character;
use codex_game_domain::FeedbackImageAttachment;
use codex_game_domain::Project;
use codex_game_store::write_art_bible;
use image::ImageFormat;
use image::ImageReader;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::fs;
use std::io;
use std::io::Cursor;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

const MAX_FEEDBACK_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_FEEDBACK_IMAGE_PIXELS: u64 = 40_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FocusManifest {
    pub art_bible_base_hash: Option<String>,
    pub character_spec_base_hash: Option<String>,
    pub accepted_render_generation_id: Option<String>,
    pub stage: String,
    pub created_at: i64,
}

pub(crate) struct FocusPaths {
    pub root: PathBuf,
    pub art_bible: PathBuf,
    pub character_spec: PathBuf,
    pub render_media: PathBuf,
    pub views_media: PathBuf,
    pub reference_media: PathBuf,
    pub video_media: PathBuf,
    pub manifest: PathBuf,
}

pub(crate) fn paths(project: &Project, character: &Character) -> FocusPaths {
    let root = Path::new(&project.root)
        .join(&character.dir_name)
        .join("tmp/focus");
    FocusPaths {
        art_bible: root.join("project/art-bible.md"),
        character_spec: root.join("character/角色定稿.md"),
        render_media: root.join("media/render"),
        views_media: root.join("media/views"),
        reference_media: root.join("media/references"),
        video_media: root.join("media/video"),
        manifest: root.join("manifest.json"),
        root,
    }
}

pub(crate) fn ensure(
    project: &Project,
    character: &Character,
    created_at: i64,
) -> io::Result<FocusManifest> {
    let paths = paths(project, character);
    fs::create_dir_all(&paths.render_media)?;
    fs::create_dir_all(&paths.views_media)?;
    fs::create_dir_all(&paths.reference_media)?;
    fs::create_dir_all(&paths.video_media)?;
    if paths.manifest.is_file() {
        return read_manifest(&paths.manifest);
    }

    let formal_art_bible = Path::new(&project.root).join("art-bible.md");
    let formal_character_spec = character
        .spec_path
        .as_deref()
        .map(|path| Path::new(&project.root).join(path))
        .unwrap_or_else(|| {
            Path::new(&project.root)
                .join(&character.dir_name)
                .join("docs/角色定稿.md")
        });
    let art_bible = fs::read(&formal_art_bible).unwrap_or_default();
    let character_spec = fs::read(&formal_character_spec).unwrap_or_default();
    write_bytes(&paths.art_bible, &art_bible)?;
    write_bytes(&paths.character_spec, &character_spec)?;
    let manifest = FocusManifest {
        art_bible_base_hash: file_hash(&formal_art_bible)?,
        character_spec_base_hash: file_hash(&formal_character_spec)?,
        accepted_render_generation_id: None,
        stage: character.state.stage().to_string(),
        created_at,
    };
    write_manifest(&paths.manifest, &manifest)?;
    Ok(manifest)
}

pub(crate) fn read_manifest(path: &Path) -> io::Result<FocusManifest> {
    serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
}

pub(crate) fn write_manifest(path: &Path, manifest: &FocusManifest) -> io::Result<()> {
    let content = serde_json::to_string_pretty(manifest)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    write_art_bible(path, &format!("{content}\n"))
}

pub(crate) fn validate_media_path(
    project: &Project,
    character: &Character,
    relative_path: &str,
) -> io::Result<PathBuf> {
    let project_root = fs::canonicalize(&project.root)?;
    let media_root = paths(project, character).root.join("media");
    fs::create_dir_all(&media_root)?;
    let media_root = fs::canonicalize(media_root)?;
    let candidate = Path::new(&project.root).join(relative_path);
    let candidate = fs::canonicalize(candidate)?;
    if !media_root.starts_with(&project_root) || !candidate.starts_with(&media_root) {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "media path is outside the current character focus workspace",
        ));
    }
    Ok(candidate)
}

pub(crate) fn validate_stage_media_path(
    project: &Project,
    character: &Character,
    stage: &str,
    relative_path: &str,
) -> io::Result<PathBuf> {
    let candidate = validate_media_path(project, character, relative_path)?;
    let stage_root = match stage {
        "render" => paths(project, character).render_media,
        "views" => paths(project, character).views_media,
        _ => {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "unsupported focus media stage",
            ));
        }
    };
    let stage_root = fs::canonicalize(stage_root)?;
    if !candidate.starts_with(stage_root) {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "media path is outside the current focus stage",
        ));
    }
    Ok(candidate)
}

pub(crate) fn import_generation_media(
    project: &Project,
    character: &Character,
    stage: &str,
    relative_path: &str,
) -> io::Result<(String, String)> {
    let source = validate_media_path(project, character, relative_path)?;
    let bytes = fs::read(&source)?;
    if bytes.is_empty() || bytes.len() > 32 * 1024 * 1024 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "generated image is empty or exceeds 32 MiB",
        ));
    }
    let extension = match image::guess_format(&bytes)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?
    {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::WebP => "webp",
        _ => {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "generated media must be PNG, JPEG, or WebP",
            ));
        }
    };
    let stage_root = match stage {
        "render" => paths(project, character).render_media,
        "views" => paths(project, character).views_media,
        _ => {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "unsupported focus media stage",
            ));
        }
    };
    fs::create_dir_all(&stage_root)?;
    let hash = hash_bytes(&bytes);
    let target = stage_root.join(format!("{hash}.{extension}"));
    if !target.exists() {
        fs::write(&target, bytes)?;
    }
    let relative = target
        .strip_prefix(&project.root)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?
        .to_string_lossy()
        .into_owned();
    validate_stage_media_path(project, character, stage, &relative)?;
    Ok((relative, hash))
}

pub(crate) fn save_feedback_image(
    project: &Project,
    character: &Character,
    mime_type: &str,
    bytes: &[u8],
) -> io::Result<FeedbackImageAttachment> {
    if bytes.is_empty() || bytes.len() > MAX_FEEDBACK_IMAGE_BYTES {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "feedback image is empty or exceeds 16 MiB",
        ));
    }
    let expected_format = match mime_type {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/webp" => ImageFormat::WebP,
        _ => {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "feedback image must be PNG, JPEG, or WebP",
            ));
        }
    };
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    if reader.format() != Some(expected_format) {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "feedback image MIME does not match its file content",
        ));
    }
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    if u64::from(width) * u64::from(height) > MAX_FEEDBACK_IMAGE_PIXELS {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "feedback image exceeds the pixel limit",
        ));
    }
    let hash = hash_bytes(bytes);
    let extension = match expected_format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::WebP => "webp",
        _ => unreachable!("validated formats are exhaustive"),
    };
    let focus = paths(project, character);
    fs::create_dir_all(&focus.reference_media)?;
    let target = focus.reference_media.join(format!("{hash}.{extension}"));
    if !target.exists() {
        fs::write(&target, bytes)?;
    }
    let relative = target
        .strip_prefix(&project.root)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?
        .to_string_lossy()
        .into_owned();
    Ok(FeedbackImageAttachment {
        kind: "feedbackImage".to_string(),
        path: relative,
        mime_type: mime_type.to_string(),
        content_hash: hash,
    })
}

pub(crate) fn verify_baselines(
    project: &Project,
    character: &Character,
    manifest: &FocusManifest,
) -> io::Result<()> {
    let art_bible = Path::new(&project.root).join("art-bible.md");
    let character_spec = character
        .spec_path
        .as_deref()
        .map(|path| Path::new(&project.root).join(path))
        .unwrap_or_else(|| {
            Path::new(&project.root)
                .join(&character.dir_name)
                .join("docs/角色定稿.md")
        });
    if file_hash(&art_bible)? != manifest.art_bible_base_hash
        || file_hash(&character_spec)? != manifest.character_spec_base_hash
    {
        return Err(io::Error::new(
            ErrorKind::Other,
            "formal visual settings changed after focus workspace creation",
        ));
    }
    Ok(())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)
}

fn file_hash(path: &Path) -> io::Result<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(hash_bytes(&bytes))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_game_domain::CharacterState;
    use codex_game_domain::ProjectId;
    use codex_game_domain::ProjectState;
    use image::DynamicImage;
    use image::Rgb;
    use image::RgbImage;
    use std::collections::BTreeMap;
    use std::io::Cursor;
    use tempfile::tempdir;

    fn fixture() -> (tempfile::TempDir, Project, Character) {
        let temp = tempdir().expect("temp dir");
        let root = temp.path();
        fs::write(root.join("art-bible.md"), "formal art bible").expect("art bible");
        let character_dir = root.join("characters/heroes/wukong");
        fs::create_dir_all(character_dir.join("docs")).expect("character docs");
        fs::write(character_dir.join("docs/角色定稿.md"), "formal character")
            .expect("character spec");
        let project = Project {
            id: ProjectId::new("project-1"),
            name: "project".to_string(),
            code: None,
            root: root.to_string_lossy().into_owned(),
            state: ProjectState::StyleSettled,
        };
        let character = Character {
            id: "character-1".to_string(),
            project_id: "project-1".to_string(),
            name: "孙悟空".to_string(),
            group: Some("heroes".to_string()),
            dir_name: "characters/heroes/wukong".to_string(),
            state: CharacterState::S1SpecConfirmed,
            spec_path: Some("characters/heroes/wukong/docs/角色定稿.md".to_string()),
            render_path: None,
            view_paths: BTreeMap::new(),
            hard_constraints: Vec::new(),
            gate_spec_confirmed_at: Some(1),
            gate_render_confirmed_at: None,
            gate_views_confirmed_at: None,
            created_at: 1,
            updated_at: 1,
        };
        (temp, project, character)
    }

    fn png_bytes() -> Vec<u8> {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, Rgb([1, 2, 3])));
        let mut bytes = Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("encode png");
        bytes.into_inner()
    }

    #[test]
    fn ensure_preserves_existing_focus_edits() {
        let (_temp, project, character) = fixture();
        let initial = ensure(&project, &character, 1).expect("initialize focus");
        let focus_paths = paths(&project, &character);
        fs::write(&focus_paths.art_bible, "edited focus art bible").expect("edit focus");

        let restored = ensure(&project, &character, 2).expect("reuse focus");

        assert_eq!(restored, initial);
        assert_eq!(
            fs::read_to_string(focus_paths.art_bible).expect("read focus"),
            "edited focus art bible"
        );
    }

    #[test]
    fn generation_media_is_archived_by_stage_and_cannot_escape_focus() {
        let (_temp, project, character) = fixture();
        ensure(&project, &character, 1).expect("initialize focus");
        let source = paths(&project, &character)
            .root
            .join("media/generated_images/call.png");
        fs::create_dir_all(source.parent().expect("source parent")).expect("source dir");
        fs::write(&source, png_bytes()).expect("source image");
        let relative = source
            .strip_prefix(&project.root)
            .expect("relative source")
            .to_string_lossy()
            .into_owned();

        let (archived, _) = import_generation_media(&project, &character, "render", &relative)
            .expect("archive image");

        assert!(archived.contains("tmp/focus/media/render/"));
        assert!(Path::new(&project.root).join(archived).is_file());
        assert!(validate_media_path(&project, &character, "art-bible.md").is_err());
    }

    #[test]
    fn baseline_changes_block_focus_publication() {
        let (_temp, project, character) = fixture();
        let manifest = ensure(&project, &character, 1).expect("initialize focus");
        assert!(verify_baselines(&project, &character, &manifest).is_ok());

        fs::write(
            Path::new(&project.root).join("art-bible.md"),
            "concurrent edit",
        )
        .expect("change formal art bible");

        assert!(verify_baselines(&project, &character, &manifest).is_err());
    }

    #[test]
    fn feedback_image_validates_declared_mime() {
        let (_temp, project, character) = fixture();
        ensure(&project, &character, 1).expect("initialize focus");
        let bytes = png_bytes();

        assert!(save_feedback_image(&project, &character, "image/jpeg", &bytes).is_err());
        let attachment = save_feedback_image(&project, &character, "image/png", &bytes)
            .expect("save feedback image");
        assert!(attachment.path.contains("tmp/focus/media/references/"));
    }
}
