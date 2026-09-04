use codex_utils_path::write_atomically;
use serde_json::Map;
use serde_json::Value;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::Path;

pub fn write_art_bible(path: &Path, markdown: &str) -> io::Result<()> {
    write_atomically(path, markdown)
}

pub fn update_project_json(
    path: &Path,
    project_id: &str,
    name: &str,
    state: &str,
) -> io::Result<()> {
    let mut document = match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str::<Value>(&contents)
            .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?,
        Err(err) if err.kind() == ErrorKind::NotFound => Value::Object(Map::new()),
        Err(err) => return Err(err),
    };
    let object = document
        .as_object_mut()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "project.json must be an object"))?;
    if let Some(existing_id) = object.get("projectId").or_else(|| object.get("id"))
        && existing_id.as_str() != Some(project_id)
    {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "project.json projectId cannot be changed",
        ));
    }
    object.insert(
        "projectId".to_string(),
        Value::String(project_id.to_string()),
    );
    object.insert("schemaVersion".to_string(), Value::from(2));
    object.insert("name".to_string(), Value::String(name.to_string()));
    object.insert("state".to_string(), Value::String(state.to_string()));
    let contents = serde_json::to_string_pretty(&document)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    write_atomically(path, &format!("{contents}\n"))
}

pub fn finalize_project_json(
    path: &Path,
    project_id: &str,
    name: &str,
    code: &str,
) -> io::Result<()> {
    update_project_json(path, project_id, name, "ready")?;
    let contents = fs::read_to_string(path)?;
    let mut document = serde_json::from_str::<Value>(&contents)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    let object = document
        .as_object_mut()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "project.json must be an object"))?;
    object.insert("code".to_string(), Value::String(code.to_string()));
    let contents = serde_json::to_string_pretty(&document)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    write_atomically(path, &format!("{contents}\n"))
}
