use super::*;
use codex_game_domain::ProjectId;
use codex_game_domain::ProjectState;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn character_file_uses_disk_location_as_ownership_source() {
    let directory = tempdir().expect("tempdir");
    let source = project(directory.path().join("source"), "source-project");
    let target = project(directory.path().join("target"), "target-project");
    let character = character(&source);

    fs::create_dir_all(Path::new(&source.root).join(&character.dir_name)).expect("source dir");
    write_character_file(&source, &character).expect("write character");

    let source_path = character_file_path(&source, &character);
    let document: Value =
        serde_json::from_str(&fs::read_to_string(&source_path).expect("read character document"))
            .expect("parse character document");
    assert_eq!(document["schemaVersion"], 3);
    assert!(document["character"].get("dirName").is_none());
    assert!(document["character"].get("group").is_none());
    assert!(document["character"].get("projectId").is_none());

    let target_dir = Path::new(&target.root).join("characters/共享角色/孙悟空");
    fs::create_dir_all(&target_dir).expect("target dir");
    fs::copy(source_path, target_dir.join(CHARACTER_FILE_NAME)).expect("copy character document");

    let loaded = read_project_characters(&target).expect("read target characters");
    assert_eq!(
        loaded,
        vec![Character {
            project_id: target.id.as_str().to_string(),
            group: Some("共享角色".to_string()),
            dir_name: "characters/共享角色/孙悟空".to_string(),
            ..character
        }]
    );
}

#[test]
fn legacy_character_location_fields_are_ignored() {
    let directory = tempdir().expect("tempdir");
    let project = project(directory.path().join("game"), "current-project");
    let character_dir = Path::new(&project.root).join("characters/新分组/孙悟空");
    fs::create_dir_all(&character_dir).expect("character dir");
    let document = json!({
        "schemaVersion": 2,
        "character": {
            "id": "stable-id",
            "projectId": "old-project",
            "name": "孙悟空",
            "group": "旧分组",
            "dirName": "characters/旧分组/孙悟空",
            "state": "s0_spec_drafting",
            "specPath": null,
            "renderPath": null,
            "viewPaths": {},
            "hardConstraints": [],
            "gateSpecConfirmedAt": null,
            "gateRenderConfirmedAt": null,
            "gateViewsConfirmedAt": null,
            "createdAt": 1,
            "updatedAt": 2
        }
    });
    fs::write(
        character_dir.join(CHARACTER_FILE_NAME),
        serde_json::to_vec_pretty(&document).expect("serialize legacy document"),
    )
    .expect("write legacy document");

    let loaded = read_project_characters(&project).expect("read characters");
    assert_eq!(loaded[0].project_id, "current-project");
    assert_eq!(loaded[0].group.as_deref(), Some("新分组"));
    assert_eq!(loaded[0].dir_name, "characters/新分组/孙悟空");
}

fn project(root: PathBuf, id: &str) -> Project {
    Project {
        id: ProjectId::new(id),
        name: id.to_string(),
        code: None,
        root: root.to_string_lossy().into_owned(),
        state: ProjectState::Ready,
    }
}

fn character(project: &Project) -> Character {
    Character {
        id: "stable-id".to_string(),
        project_id: project.id.as_str().to_string(),
        name: "孙悟空".to_string(),
        group: Some("玩家角色".to_string()),
        dir_name: "characters/玩家角色/孙悟空".to_string(),
        state: CharacterState::S0SpecDrafting,
        spec_path: None,
        render_path: None,
        view_paths: BTreeMap::new(),
        hard_constraints: Vec::new(),
        gate_spec_confirmed_at: None,
        gate_render_confirmed_at: None,
        gate_views_confirmed_at: None,
        created_at: 1,
        updated_at: 2,
    }
}
