use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn infers_rig_kind_and_animation_policy() {
    let constraints = vec![json!({
        "item": "生理姿态",
        "scope": "identity",
        "value": "直立双足"
    })];
    assert_eq!(infer_rig_kind(&constraints), RigKind::Biped);
    assert_eq!(
        animation_presets(RigKind::Biped),
        vec!["preset:idle", "preset:walk", "preset:run"]
    );
    assert_eq!(animation_presets(RigKind::Quadruped), vec!["preset:walk"]);
}

#[test]
fn complete_glb_requires_model_texture_skin_and_expected_animations() {
    let complete = glb(json!({
        "asset": { "version": "2.0" },
        "meshes": [{}],
        "materials": [{}],
        "textures": [{}],
        "images": [{ "bufferView": 0, "mimeType": "image/png" }],
        "skins": [{}],
        "animations": [
            { "name": "Idle" },
            { "name": "Walk" },
            { "name": "Run" }
        ]
    }));

    let summary = validate_complete_glb(
        &complete,
        &["idle".to_string(), "walk".to_string(), "run".to_string()],
    )
    .expect("complete GLB");

    assert_eq!(summary.mesh_count, 1);
    assert_eq!(summary.material_count, 1);
    assert_eq!(summary.texture_count, 1);
    assert_eq!(summary.skin_count, 1);
    assert_eq!(summary.animation_names, vec!["Idle", "Walk", "Run"]);
}

#[test]
fn incomplete_glb_is_rejected() {
    let missing_skin = glb(json!({
        "asset": { "version": "2.0" },
        "meshes": [{}],
        "materials": [{}],
        "textures": [{}],
        "animations": [{ "name": "Walk" }]
    }));

    assert!(validate_complete_glb(&missing_skin, &["walk".to_string()]).is_err());
}

fn glb(document: serde_json::Value) -> Vec<u8> {
    let mut json = serde_json::to_vec(&document).expect("json");
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let total_len = 20 + json.len();
    let mut bytes = Vec::with_capacity(total_len);
    bytes.extend_from_slice(b"glTF");
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&(total_len as u32).to_le_bytes());
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
    bytes.extend_from_slice(&json);
    bytes
}
