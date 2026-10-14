//! Files of local recognition on disk: downloaded models, the installed
//! runtime and unfinished downloads. They live in the local (non-roaming)
//! app data folder, since a model is up to 1.7 GB.
//!
//! The runtime is unpacked into a temporary folder, checked (its
//! contract.json must name the transcribe.cpp version and C header Typr was
//! written for) and only then renamed into place, so a half-installed
//! runtime never loads.

use serde::Deserialize;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use super::catalog::{ModelSpec, HEADER_HASH, RUNTIME, TRANSCRIBE_VERSION};

/// Written last, after the runtime was unpacked and checked
const COMPLETE_MARKER: &str = ".typr-complete";
pub const LIBRARY_NAME: &str = "transcribe.dll";

#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// `%LOCALAPPDATA%\com.typr.app\local`
    pub fn default_root() -> PathBuf {
        dirs::data_local_dir()
            .or_else(dirs::config_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("com.typr.app")
            .join("local")
    }

    pub fn models(&self) -> PathBuf {
        self.root.join("models")
    }

    pub fn runtimes(&self) -> PathBuf {
        self.root.join("runtimes")
    }

    pub fn downloads(&self) -> PathBuf {
        self.root.join("downloads")
    }

    pub fn model_file(&self, model: &ModelSpec) -> PathBuf {
        self.models().join(model.file)
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.runtimes().join(RUNTIME.dir_name())
    }

    /// Where the runtime archive is downloaded to (with `.part` while unfinished)
    pub fn runtime_download(&self) -> PathBuf {
        self.downloads().join(RUNTIME.file_name())
    }
}

pub fn part_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

/// Folder the runtime is unpacked into before it is moved into place. (Its
/// folder name has dots in the version, so `with_extension` won't do.)
fn staging_dir(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    target.with_file_name(name)
}

pub fn is_model_installed(paths: &Paths, model: &ModelSpec) -> bool {
    fs::metadata(paths.model_file(model))
        .map(|m| m.is_file() && m.len() == model.size)
        .unwrap_or(false)
}

pub fn is_runtime_installed(paths: &Paths) -> bool {
    let dir = paths.runtime_dir();
    dir.join(COMPLETE_MARKER).is_file() && dir.join(LIBRARY_NAME).is_file()
}

#[derive(Debug, Deserialize)]
pub struct Contract {
    pub version: String,
    pub header_hash: String,
    #[serde(default)]
    pub backends: Vec<String>,
}

pub fn read_contract(dir: &Path) -> Result<Contract, String> {
    let path = dir.join("contract.json");
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("The runtime has no readable contract.json ({}): {}", path.display(), e))?;
    let contract: Contract = serde_json::from_str(&text)
        .map_err(|e| format!("The runtime's contract.json is invalid: {}", e))?;
    check_contract(&contract)?;
    Ok(contract)
}

fn check_contract(contract: &Contract) -> Result<(), String> {
    if contract.version != TRANSCRIBE_VERSION || contract.header_hash != HEADER_HASH {
        return Err(format!(
            "The runtime is transcribe.cpp {} (header {}), Typr needs {} (header {})",
            contract.version, contract.header_hash, TRANSCRIBE_VERSION, HEADER_HASH
        ));
    }
    Ok(())
}

/// Unpacks the downloaded runtime `archive` and puts it in place. Blocking;
/// run it off the async runtime.
pub fn install_runtime(paths: &Paths, archive: &Path) -> Result<(), String> {
    let target = paths.runtime_dir();
    let staging = staging_dir(&target);
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|e| format!("Failed to clean {}: {}", staging.display(), e))?;
    }
    fs::create_dir_all(&staging)
        .map_err(|e| format!("Failed to create {}: {}", staging.display(), e))?;

    let result = (|| {
        log::info!("Unpacking {}", RUNTIME.file_name());
        unpack_tar_gz(archive, &staging)?;
        let contract = read_contract(&staging)?;
        if !staging.join(LIBRARY_NAME).is_file() {
            return Err(format!("The runtime has no {}", LIBRARY_NAME));
        }
        log::info!(
            "Runtime checked: transcribe.cpp {}, backends {}",
            contract.version,
            contract.backends.join(", ")
        );
        fs::write(staging.join(COMPLETE_MARKER), TRANSCRIBE_VERSION)
            .map_err(|e| format!("Failed to finish the install: {}", e))
    })();

    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    if target.exists() {
        fs::remove_dir_all(&target).map_err(|e| {
            format!("Failed to replace {} (is it in use?): {}", target.display(), e)
        })?;
    }
    fs::rename(&staging, &target)
        .map_err(|e| format!("Failed to move the runtime into place: {}", e))?;
    Ok(())
}

/// A path inside the archive without its top-level folder, with plain
/// components only, so no entry can write outside the target folder.
fn safe_relative(path: &Path) -> Option<PathBuf> {
    let mut parts = path.components().filter(|c| !matches!(c, Component::CurDir));
    parts.next()?;
    let mut out = PathBuf::new();
    for part in parts {
        match part {
            Component::Normal(name) => out.push(name),
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn unpack_tar_gz(file: &Path, target: &Path) -> Result<(), String> {
    let reader = fs::File::open(file).map_err(|e| format!("Failed to open {}: {}", file.display(), e))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(io::BufReader::new(reader)));
    let entries = archive
        .entries()
        .map_err(|e| format!("Failed to read {}: {}", file.display(), e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("Failed to read {}: {}", file.display(), e))?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| format!("Bad path in {}: {}", file.display(), e))?
            .into_owned();
        let Some(relative) = safe_relative(&path) else {
            continue;
        };
        let dest = target.join(relative);
        if kind.is_dir() {
            fs::create_dir_all(&dest).map_err(|e| format!("Failed to create {}: {}", dest.display(), e))?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
        }
        let mut out = fs::File::create(&dest).map_err(|e| format!("Failed to create {}: {}", dest.display(), e))?;
        io::copy(&mut entry, &mut out).map_err(|e| format!("Failed to unpack {}: {}", dest.display(), e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_safe_relative() {
        assert_eq!(safe_relative(Path::new("bundle/ggml.dll")), Some(PathBuf::from("ggml.dll")));
        assert_eq!(
            safe_relative(Path::new("./bundle/licenses/LICENSE")),
            Some(PathBuf::from("licenses/LICENSE"))
        );
        assert_eq!(safe_relative(Path::new("bundle/../../evil.dll")), None);
        assert_eq!(safe_relative(Path::new("bundle")), None);
    }

    #[test]
    fn test_part_path() {
        assert_eq!(
            part_path(Path::new(r"C:\models\model.gguf")),
            PathBuf::from(r"C:\models\model.gguf.part")
        );
    }

    #[test]
    fn test_contract_must_match() {
        let good = Contract {
            version: TRANSCRIBE_VERSION.to_string(),
            header_hash: HEADER_HASH.to_string(),
            backends: vec!["vulkan".into(), "cpu".into()],
        };
        assert!(check_contract(&good).is_ok());
        let old = Contract {
            version: "0.1.3".to_string(),
            ..good
        };
        assert!(check_contract(&old).is_err());
    }

    fn write_bundle(path: &Path, files: &[(&str, &[u8])]) {
        let gz = flate2::write::GzEncoder::new(fs::File::create(path).unwrap(), flate2::Compression::fast());
        let mut builder = tar::Builder::new(gz);
        for (name, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn test_install_runtime() {
        let dir = temp("typr_test_install_runtime");
        let paths = Paths::new(dir.join("local"));
        let contract = format!(
            r#"{{"version":"{}","header_hash":"{}","backends":["vulkan","cpu"]}}"#,
            TRANSCRIBE_VERSION, HEADER_HASH
        );
        let bundle = dir.join("bundle.tar.gz");
        write_bundle(
            &bundle,
            &[
                ("transcribe-native/contract.json", contract.as_bytes()),
                ("transcribe-native/transcribe.dll", b"dll"),
                ("transcribe-native/licenses/LICENSE", b"MIT"),
            ],
        );

        install_runtime(&paths, &bundle).unwrap();

        let installed = paths.runtime_dir();
        assert!(is_runtime_installed(&paths));
        assert_eq!(fs::read(installed.join("transcribe.dll")).unwrap(), b"dll");
        assert!(installed.join("licenses").join("LICENSE").is_file());
        assert!(!staging_dir(&installed).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_install_rejects_wrong_version() {
        let dir = temp("typr_test_install_wrong");
        let paths = Paths::new(dir.join("local"));
        let bundle = dir.join("bundle.tar.gz");
        write_bundle(
            &bundle,
            &[
                (
                    "native/contract.json",
                    br#"{"version":"0.1.0","header_hash":"0000","backends":["cpu"]}"#,
                ),
                ("native/transcribe.dll", b"dll"),
            ],
        );
        assert!(install_runtime(&paths, &bundle).is_err());
        assert!(!is_runtime_installed(&paths));
        assert!(!staging_dir(&paths.runtime_dir()).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
