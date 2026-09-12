pub mod base16;
pub mod builtin;
pub mod color;
pub mod resolve;
pub mod schema;
pub mod system;
pub mod wsz;

/// A theme id that is a file name and nothing else.
///
/// The id becomes `<themes dir>/<id>.toml`, and the name it is derived from
/// comes from `--name` or from a skin's file stem. A separator or a `..` in
/// there is a path, not a name.
pub fn safe_id(name: &str) -> anyhow::Result<String> {
    let id = name.trim().to_lowercase().replace(' ', "-");
    anyhow::ensure!(
        !id.is_empty()
            && id != "."
            && id != ".."
            && !id.contains(['/', '\\', '\0'])
            && !id.starts_with('.'),
        "a theme name must be a plain file name, and {name:?} is not"
    );
    Ok(id)
}
