use crate::DEFAULT_MODEL3D_MAX_BYTES;
use crate::Model3dError;
use crate::Result;

const GLB_MAGIC: &[u8; 4] = b"glTF";
const GLB_VERSION: u32 = 2;
const JSON_CHUNK_TYPE: u32 = 0x4E4F534A;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlbSummary {
    pub mesh_count: usize,
    pub material_count: usize,
    pub texture_count: usize,
    pub skin_count: usize,
    pub animation_names: Vec<String>,
}

pub fn validate_complete_glb(bytes: &[u8], expected_animations: &[String]) -> Result<GlbSummary> {
    if bytes.len() < 20 || bytes.len() > DEFAULT_MODEL3D_MAX_BYTES {
        return Err(Model3dError::InvalidGlb(
            "file is empty, truncated, or exceeds 256 MiB".to_string(),
        ));
    }
    if &bytes[..4] != GLB_MAGIC || read_u32(bytes, 4)? != GLB_VERSION {
        return Err(Model3dError::InvalidGlb(
            "expected a binary glTF 2.0 file".to_string(),
        ));
    }
    let declared_length = read_u32(bytes, 8)? as usize;
    if declared_length != bytes.len() {
        return Err(Model3dError::InvalidGlb(
            "declared GLB length does not match file size".to_string(),
        ));
    }
    let chunk_length = read_u32(bytes, 12)? as usize;
    let chunk_type = read_u32(bytes, 16)?;
    let json_end = 20usize
        .checked_add(chunk_length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| Model3dError::InvalidGlb("invalid JSON chunk length".to_string()))?;
    if chunk_type != JSON_CHUNK_TYPE {
        return Err(Model3dError::InvalidGlb(
            "first GLB chunk must contain JSON".to_string(),
        ));
    }
    let document: serde_json::Value = serde_json::from_slice(&bytes[20..json_end])
        .map_err(|error| Model3dError::InvalidGlb(format!("invalid glTF JSON: {error}")))?;
    let mesh_count = array_len(&document, "meshes");
    let material_count = array_len(&document, "materials");
    let texture_count = array_len(&document, "textures").max(array_len(&document, "images"));
    let skin_count = array_len(&document, "skins");
    let animations = document
        .get("animations")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let animation_names = animations
        .iter()
        .filter_map(|animation| animation.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if mesh_count == 0 {
        return Err(Model3dError::InvalidGlb(
            "GLB contains no meshes".to_string(),
        ));
    }
    if material_count == 0 || texture_count == 0 {
        return Err(Model3dError::InvalidGlb(
            "GLB contains no embedded material/texture data".to_string(),
        ));
    }
    if skin_count == 0 {
        return Err(Model3dError::InvalidGlb("GLB contains no skin".to_string()));
    }
    if animations.len() < expected_animations.len() {
        return Err(Model3dError::InvalidGlb(format!(
            "GLB contains {} animations, expected {}",
            animations.len(),
            expected_animations.len()
        )));
    }
    for expected in expected_animations {
        if !animation_names.iter().any(|name| {
            name.to_ascii_lowercase()
                .contains(&expected.to_ascii_lowercase())
        }) {
            return Err(Model3dError::InvalidGlb(format!(
                "GLB is missing animation clip `{expected}`"
            )));
        }
    }
    Ok(GlbSummary {
        mesh_count,
        material_count,
        texture_count,
        skin_count,
        animation_names,
    })
}

fn array_len(document: &serde_json::Value, key: &str) -> usize {
    document
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| Model3dError::InvalidGlb("truncated GLB header".to_string()))?;
    Ok(u32::from_le_bytes(value))
}

#[cfg(test)]
#[path = "glb_import_tests.rs"]
mod import_tests;
