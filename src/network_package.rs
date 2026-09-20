use super::{CatalogEntry, RemoteGamePackage};
use sha2::{Digest, Sha256};
use url::Url;

const MAX_CATALOG_BYTES: usize = 512 * 1024;
const MAX_PACKAGE_DESCRIPTOR_BYTES: usize = 512 * 1024;
const MAX_PACKAGE_TEXT_BYTES: usize = 512 * 1024;
const MAX_PACKAGE_FILE_BYTES: usize = 10 * 1024 * 1024;
const MAX_PACKAGE_FILES: usize = 128;

pub(super) fn http_url(base: &Url, path: &str) -> Result<Url, String> {
    let mut url = base.clone();
    let scheme = match url.scheme() {
        "http" | "ws" => "http",
        "https" | "wss" => "https",
        scheme => return Err(format!("unsupported backend URL scheme: {scheme}")),
    };
    url.set_scheme(scheme)
        .map_err(|()| "could not set HTTP scheme".to_owned())?;
    let base_path = url.path().trim_end_matches('/');
    let request_path = if path.starts_with('/') {
        format!("{base_path}{path}")
    } else if base_path.is_empty() {
        format!("/{path}")
    } else {
        format!("{base_path}/{path}")
    };
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Url::parse(&format!(
        "{}{}",
        url.as_str().trim_end_matches('/'),
        request_path
    ))
    .map_err(|error| format!("could not build HTTP URL: {error}"))
}

pub(super) fn load_catalog(base_url: &Url) -> Result<Vec<CatalogEntry>, String> {
    let endpoint = http_url(base_url, "/cubes?page=1&page_size=50")?;
    let source = fetch_http_text(&endpoint, MAX_CATALOG_BYTES)?;
    let value: serde_json::Value = serde_json::from_str(&source)
        .map_err(|error| format!("the cube catalog was invalid JSON: {error}"))?;
    let cubes = value
        .get("cubes")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "the cube catalog did not contain a cubes list".to_owned())?;

    cubes
        .iter()
        .map(|cube| {
            let id = cube
                .get("cubeId")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "a cube catalog entry did not contain an ID".to_owned())?;
            if !is_valid_game_id(id) {
                return Err(format!(
                    "the cube catalog returned an invalid game ID: {id}"
                ));
            }
            let display_name = cube
                .get("displayName")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(id)
                .to_owned();
            let version = cube
                .get("version")
                .map(value_as_string)
                .unwrap_or_else(|| "unknown".to_owned());
            let package_path = cube
                .get("assetBaseURL")
                .and_then(serde_json::Value::as_str)
                .or_else(|| cube.get("packagePath").and_then(serde_json::Value::as_str))
                .ok_or_else(|| format!("cube {id} did not contain a package path"))?;
            let package_url = package_url(base_url, package_path)?;
            Ok(CatalogEntry {
                id: id.to_owned(),
                display_name,
                version,
                package_url,
            })
        })
        .collect()
}

pub(super) fn load_remote_package(entry: &CatalogEntry) -> Result<RemoteGamePackage, String> {
    let descriptor_url = entry
        .package_url
        .join("package.json")
        .map_err(|error| format!("could not build the cube package URL: {error}"))?;
    let descriptor_source = fetch_http_text(&descriptor_url, MAX_PACKAGE_DESCRIPTOR_BYTES)?;
    let descriptor: serde_json::Value = serde_json::from_str(&descriptor_source)
        .map_err(|error| format!("the cube package descriptor was invalid JSON: {error}"))?;
    let manifest_name = descriptor
        .get("manifest")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "the cube package descriptor has no manifest".to_owned())?;
    let entry_name = descriptor
        .get("entry")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "the cube package descriptor has no script entry".to_owned())?;
    if manifest_name != "manifest.json" || entry_name != "game.luau" {
        return Err("the cube package descriptor points to an unsupported entry".to_owned());
    }
    if descriptor.get("id").and_then(serde_json::Value::as_str) != Some(entry.id.as_str()) {
        return Err("the cube package ID does not match the catalog entry".to_owned());
    }
    let files = descriptor
        .get("files")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "the cube package descriptor has no file table".to_owned())?;
    let checksums = descriptor
        .get("sha256")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "the cube package descriptor has no checksums".to_owned())?;
    if files.len() > MAX_PACKAGE_FILES {
        return Err("the cube package contains too many files".to_owned());
    }
    let file_paths = files
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<std::collections::HashSet<_>>();
    if !file_paths.contains(manifest_name) || !file_paths.contains(entry_name) {
        return Err("the cube package file table is missing its manifest or script".to_owned());
    }

    let manifest_url = entry
        .package_url
        .join(manifest_name)
        .map_err(|error| format!("could not build the cube manifest URL: {error}"))?;
    let script_url = entry
        .package_url
        .join(entry_name)
        .map_err(|error| format!("could not build the cube script URL: {error}"))?;
    let manifest_bytes = fetch_http_bytes(&manifest_url, MAX_PACKAGE_TEXT_BYTES)?;
    let script_bytes = fetch_http_bytes(&script_url, MAX_PACKAGE_TEXT_BYTES)?;
    let manifest = String::from_utf8(manifest_bytes.clone())
        .map_err(|_| "the cube manifest was not valid UTF-8".to_owned())?;
    let script = String::from_utf8(script_bytes.clone())
        .map_err(|_| "the cube script was not valid UTF-8".to_owned())?;
    let manifest_value: serde_json::Value = serde_json::from_str(&manifest)
        .map_err(|error| format!("the cube manifest was invalid JSON: {error}"))?;
    if manifest_value.get("id").and_then(serde_json::Value::as_str) != Some(entry.id.as_str()) {
        return Err("the cube manifest ID does not match the catalog entry".to_owned());
    }
    let expected_version = descriptor.get("version").map(value_as_string);
    let manifest_version = manifest_value.get("version").map(value_as_string);
    if expected_version != manifest_version {
        return Err("the cube package version does not match its manifest".to_owned());
    }

    let image_paths = manifest_value
        .get("assets")
        .and_then(|assets| assets.get("images"))
        .and_then(serde_json::Value::as_object)
        .map(|images| {
            images
                .values()
                .filter_map(|image| image.get("path").and_then(serde_json::Value::as_str))
                .map(str::to_owned)
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();
    let model_paths = manifest_value
        .get("assets")
        .and_then(|assets| assets.get("models"))
        .and_then(serde_json::Value::as_object)
        .map(|models| {
            models
                .values()
                .filter_map(|model| model.get("path").and_then(serde_json::Value::as_str))
                .map(str::to_owned)
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();

    let mut package_files = Vec::new();
    for file in files {
        let path = file
            .as_str()
            .ok_or_else(|| "the cube package file table contains a non-string path".to_owned())?;
        if !is_safe_package_path(path) {
            return Err(format!("the cube package file path is unsafe: {path}"));
        }
        let expected_hash = checksums
            .get(path)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("the cube package has no checksum for {path}"))?;
        if !is_sha256(expected_hash) {
            return Err(format!("the cube package checksum for {path} is invalid"));
        }
        let bytes = if path == manifest_name {
            manifest_bytes.clone()
        } else if path == entry_name {
            script_bytes.clone()
        } else {
            let url = entry
                .package_url
                .join(path)
                .map_err(|error| format!("could not build the cube asset URL: {error}"))?;
            fetch_http_bytes(&url, MAX_PACKAGE_FILE_BYTES)?
        };
        if sha256_hex(&bytes) != expected_hash {
            return Err(format!(
                "the cube package checksum did not match for {path}"
            ));
        }
        if image_paths.contains(path) || model_paths.contains(path) {
            package_files.push((path.to_owned(), bytes));
        }
    }
    Ok(RemoteGamePackage {
        id: entry.id.clone(),
        manifest,
        script,
        files: package_files,
    })
}

fn package_url(base_url: &Url, raw: &str) -> Result<Url, String> {
    let url = if raw.starts_with('/') {
        http_url(base_url, raw)?
    } else {
        Url::parse(raw).map_err(|error| format!("cube package URL is invalid: {error}"))?
    };
    let expected_host = base_url.host_str();
    let allowed_host = url.host_str() == expected_host
        || (!cfg!(debug_assertions) && url.host_str() == Some("assets.cubacadabra.com"));
    if !allowed_host || !matches!(url.scheme(), "http" | "https") {
        return Err("cube package URL is outside the configured asset hosts".to_owned());
    }
    let mut url = url;
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

fn fetch_http_text(url: &Url, maximum_bytes: usize) -> Result<String, String> {
    let bytes = fetch_http_bytes(url, maximum_bytes)?;
    String::from_utf8(bytes).map_err(|_| format!("response from {url} was not valid UTF-8"))
}

fn fetch_http_bytes(url: &Url, maximum_bytes: usize) -> Result<Vec<u8>, String> {
    let mut response = ureq::get(url.as_str())
        .header("accept", "application/json, application/octet-stream")
        .call()
        .map_err(|error| format!("could not load {url}: {error}"))?;
    let bytes = response
        .body_mut()
        .read_to_vec()
        .map_err(|error| format!("could not read {url}: {error}"))?;
    if bytes.len() > maximum_bytes {
        return Err(format!("response from {url} was too large"));
    }
    Ok(bytes)
}

fn value_as_string(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn is_safe_package_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_valid_game_id(value: &str) -> bool {
    (3..=64).contains(&value.len())
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}
