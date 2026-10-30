use std::path::{Path, PathBuf};

pub fn storage_paths(repo: &Path, home: &Path) -> Result<Vec<PathBuf>, String> {
    let config = repo.join("src-tauri/tauri.conf.json");
    let text = match std::fs::read_to_string(&config) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Cannot read native app configuration: {error}")),
    };
    let config: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("Invalid native app configuration: {error}"))?;
    let id = config["identifier"]
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.split('.').all(|part| !part.is_empty())
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
        })
        .ok_or("Native app configuration requires a valid identifier")?;
    let library = home.join("Library");
    let mut paths: Vec<_> = [
        "Application Support",
        "Caches",
        "WebKit",
        "Logs",
        "HTTPStorages",
    ]
    .into_iter()
    .map(|folder| library.join(folder).join(id))
    .collect();
    paths.push(library.join("Preferences").join(format!("{id}.plist")));
    paths.push(
        library
            .join("Saved Application State")
            .join(format!("{id}.savedState")),
    );
    let manifest = repo.join("src-tauri/Cargo.toml");
    let output = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(&manifest)
        .output()
        .map_err(|error| format!("Cannot identify native app binaries: {error}"))?;
    if !output.status.success() {
        return Err("Cannot identify native app binaries with cargo metadata".into());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Invalid native app metadata: {error}"))?;
    if let Some(packages) = metadata["packages"].as_array() {
        for package in packages {
            if package["manifest_path"].as_str() != manifest.to_str() {
                continue;
            }
            if let Some(targets) = package["targets"].as_array() {
                for target in targets {
                    if target["kind"]
                        .as_array()
                        .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
                    {
                        if let Some(name) = target["name"].as_str().filter(|name| {
                            !name.is_empty()
                                && *name != "."
                                && *name != ".."
                                && !name.contains(['/', '\\'])
                        }) {
                            for folder in ["Caches", "WebKit", "HTTPStorages"] {
                                paths.push(library.join(folder).join(name));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(paths)
}
