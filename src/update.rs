//! Signed silent updates from GitHub Releases (same protocol as 1.9, so 1.9 updates to this version and
//! this version updates to the next ones).
//!
//! A background thread looks at the latest published release (at most every 6 hours), reads its
//! signature (Owlingstone.exe.sig, made with the maintainer's secret key, which is never on GitHub),
//! checks it with the public key built into the app (PUBLIC_KEYS), downloads the exe, checks that its
//! SHA-256 is the signed one and puts it next to the running one (Owlingstone.exe.new). Without a valid
//! signature: no update. The Update button only shows once all is checked; `install()` then swaps the
//! two exes and starts the new one, without a window or a script.
//!
//! Signature file (4 lines): "owlingstone-release 1", "version X", "sha256 H", "signature S" where S is
//! the Ed25519 signature of the first 3 lines plus a newline.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use ed25519_dalek::{Signature, VerifyingKey};
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{net, paths};

pub const REPO: &str = "owlingstone/owlingstone";
pub const API_LATEST: &str = "https://api.github.com/repos/owlingstone/owlingstone/releases/latest";
pub const DOWNLOAD_PREFIX: &str = "https://github.com/owlingstone/owlingstone/releases/download/";
/// The file players download; its version = the tag (v1.2.3).
pub const ASSET_EXE: &str = "Owlingstone.exe";
/// Its signature (tools/release_key.py), added to the release after the build.
pub const ASSET_SIG: &str = "Owlingstone.exe.sig";
/// Public keys of release signatures (Ed25519, hex). The secret key stays offline with the maintainer.
/// To change keys: publish a version signed by the old key that holds the new one.
pub const PUBLIC_KEYS: &[&str] = &["1fe80b4b9b1d67d1cf0bda79a7c1c3f7f6b60a97a71711e875021314377f8a59"];
/// Tells the new exe to wait until the old one is closed.
pub const AFTER_UPDATE: &str = "OWLINGSTONE_AFTER_UPDATE";
/// Seconds between two questions to GitHub.
pub const CHECK_EVERY: f64 = 6.0 * 3600.0;
/// A bigger download is refused (abnormal).
pub const MAX_DOWNLOAD: u64 = 300 * 1024 * 1024;

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^v(\d+(?:\.\d+){1,3})$").expect("valid regex"));

/// "1.10.2" -> [1, 10, 2], to compare correctly (1.10 > 1.9).
pub fn parse(version: &str) -> Vec<u64> {
    version.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse(candidate) > parse(current)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseInfo {
    pub version: String,
    pub url: String,
    pub sig_url: String,
    /// GitHub's SHA-256 of the exe ("" if not given).
    pub digest: String,
    pub size: u64,
}

/// "latest release" API answer -> the usable release, or None. The exe AND its signature are needed.
pub fn pick_release(release: &Value) -> Option<ReleaseInfo> {
    if release.get("draft").and_then(Value::as_bool) == Some(true) || release.get("prerelease").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let tag = release.get("tag_name").and_then(Value::as_str).unwrap_or("");
    let version = TAG.captures(tag)?[1].to_string();
    let assets = release.get("assets").and_then(Value::as_array)?;
    let find = |name: &str| assets.iter().find(|a| a.get("name").and_then(Value::as_str) == Some(name));
    let (exe, sig) = (find(ASSET_EXE)?, find(ASSET_SIG)?); // not signed yet: wait
    let url = exe.get("browser_download_url").and_then(Value::as_str).unwrap_or("").to_string();
    let sig_url = sig.get("browser_download_url").and_then(Value::as_str).unwrap_or("").to_string();
    if !url.starts_with(DOWNLOAD_PREFIX) || !sig_url.starts_with(DOWNLOAD_PREFIX) {
        return None;
    }
    let digest = exe.get("digest").and_then(Value::as_str).unwrap_or("");
    let digest = digest.strip_prefix("sha256:").map(str::to_lowercase).unwrap_or_default();
    Some(ReleaseInfo { version, url, sig_url, digest, size: exe.get("size").and_then(Value::as_u64).unwrap_or(0) })
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Text of Owlingstone.exe.sig -> the SHA-256 signed for this version. Err if the signature is not from
/// one of our keys, or is for another version (no going back).
pub fn signed_sha256(sig_text: &str, version: &str, keys: &[&str]) -> Result<String, String> {
    let text = sig_text.replace("\r\n", "\n");
    let lines: Vec<&str> = text.trim().split('\n').collect();
    if lines.len() != 4 || lines[0] != "owlingstone-release 1" || lines[1] != format!("version {version}") {
        return Err("signature file is not for this version".into());
    }
    let sha = lines[2].strip_prefix("sha256 ").unwrap_or(lines[2]);
    let sig = lines[3].strip_prefix("signature ").unwrap_or(lines[3]);
    if !is_hex(sha, 64) || !is_hex(sig, 128) {
        return Err("malformed signature file".into());
    }
    let message = format!("{}\n", lines[..3].join("\n"));
    let sig_bytes: [u8; 64] = hex::decode(sig).map_err(|e| e.to_string())?.try_into().map_err(|_| "bad signature length")?;
    let signature = Signature::from_bytes(&sig_bytes);
    let good = keys.iter().any(|k| {
        hex::decode(k)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
            .and_then(|b| VerifyingKey::from_bytes(&b).ok())
            .is_some_and(|key| key.verify_strict(message.as_bytes(), &signature).is_ok())
    });
    if !good {
        return Err("bad signature: update refused".into());
    }
    Ok(sha.to_string())
}

pub fn sha256_of(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Downloads: (url, limit, timeout seconds) -> bytes. Replaced in tests.
pub type Get<'a> = &'a dyn Fn(&str, u64, u64) -> Result<Vec<u8>, String>;

pub fn github_get(url: &str, limit: u64, timeout: u64) -> Result<Vec<u8>, String> {
    net::get_github(url, limit, timeout)
}

/// Finds, downloads and checks the latest GitHub release (in a background thread, never the window).
pub struct Updater {
    pub current: String,
    pub exe_path: PathBuf,
    pub state_path: PathBuf,
    pub keys: Vec<String>,
    /// The version ready to install, once the exe is checked and next to the running one.
    pub ready: Option<ReleaseInfo>,
}

impl Updater {
    pub fn new(current: &str) -> Updater {
        let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok()).unwrap_or_else(|| PathBuf::from(ASSET_EXE));
        Updater {
            current: current.to_string(),
            exe_path: exe,
            state_path: paths::user_dir().join("update_check.json"),
            keys: PUBLIC_KEYS.iter().map(|k| k.to_string()).collect(),
            ready: None,
        }
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = self.exe_path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(suffix);
        self.exe_path.with_file_name(name)
    }

    pub fn new_exe(&self) -> PathBuf {
        self.sibling(".new")
    }

    pub fn old_exe(&self) -> PathBuf {
        self.sibling(".old")
    }

    fn last_check(&self) -> f64 {
        std::fs::read_to_string(&self.state_path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get("checked").and_then(Value::as_f64))
            .unwrap_or(0.0)
    }

    /// One check, at most every CHECK_EVERY. Ok(Some) once a newer signed version is downloaded and checked.
    pub fn check_once(&mut self, get: Get, now: f64) -> Result<Option<ReleaseInfo>, String> {
        if self.ready.is_some() || now - self.last_check() < CHECK_EVERY {
            return Ok(self.ready.clone());
        }
        self.check_now(get, now)
    }

    /// One check right away, whatever the time of the last one (tray menu "Check for updates now").
    pub fn check_now(&mut self, get: Get, now: f64) -> Result<Option<ReleaseInfo>, String> {
        if self.ready.is_some() {
            return Ok(self.ready.clone());
        }
        if let Some(dir) = self.state_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&self.state_path, serde_json::json!({"checked": now}).to_string()).map_err(|e| e.to_string())?;
        let latest: Value = serde_json::from_slice(&get(API_LATEST, 1_000_000, 20)?).map_err(|e| e.to_string())?;
        let Some(info) = pick_release(&latest) else { return Ok(None) };
        if !is_newer(&info.version, &self.current) {
            return Ok(None);
        }
        if info.size > MAX_DOWNLOAD {
            return Err("release too large".into());
        }
        let sig = get(&info.sig_url, 4096, 20)?;
        let keys: Vec<&str> = self.keys.iter().map(String::as_str).collect();
        let sha = signed_sha256(&String::from_utf8_lossy(&sig), &info.version, &keys)?;
        if !info.digest.is_empty() && info.digest != sha {
            return Err("GitHub's checksum differs from the signed one: download refused".into());
        }
        let dir = self.state_path.parent().map(Path::to_path_buf).unwrap_or_default();
        let tmp = dir.join(format!("update-{}.exe.part", uuid::Uuid::new_v4().simple()));
        let result = (|| {
            std::fs::write(&tmp, get(&info.url, MAX_DOWNLOAD, 300)?).map_err(|e| e.to_string())?;
            if sha256_of(&tmp).map_err(|e| e.to_string())? != sha {
                return Err("checksum mismatch: download refused".to_string());
            }
            // Checked: put next to the running exe (copy, as the user folder may be on another drive).
            std::fs::copy(&tmp, self.new_exe()).map_err(|e| e.to_string())?;
            Ok(())
        })();
        let _ = std::fs::remove_file(&tmp);
        result?;
        self.ready = Some(info);
        Ok(self.ready.clone())
    }

    /// Swaps the exes and starts the new one (the app must then quit). Err if impossible: nothing changed.
    pub fn install(&self) -> Result<(), String> {
        if self.ready.is_none() || !self.new_exe().exists() {
            return Err("no update ready".into());
        }
        // A running exe can be renamed on Windows.
        let _ = std::fs::remove_file(self.old_exe());
        std::fs::rename(&self.exe_path, self.old_exe()).map_err(|e| e.to_string())?;
        if let Err(e) = std::fs::rename(self.new_exe(), &self.exe_path) {
            let _ = std::fs::rename(self.old_exe(), &self.exe_path); // back: nothing changed
            return Err(e.to_string());
        }
        spawn_detached(&self.exe_path).map_err(|e| e.to_string())
    }
}

/// Starts our (checked) exe on its own, telling it to wait for this one to quit.
pub fn spawn_detached(exe: &Path) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(exe);
    cmd.env(AFTER_UPDATE, "1").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn().map(|_| ())
}

/// After an update: removes the old exe (Owlingstone.exe.old) if it is still there.
pub fn remove_old_exe() {
    if let Ok(exe) = std::env::current_exe() {
        let mut name = exe.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(".old");
        let _ = std::fs::remove_file(exe.with_file_name(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const SEED: [u8; 32] = [7; 32];
    const SIG_URL: &str = "https://github.com/owlingstone/owlingstone/releases/download/v9.9.9/Owlingstone.exe.sig";
    const EXE_URL: &str = "https://github.com/owlingstone/owlingstone/releases/download/v9.9.9/Owlingstone.exe";

    fn test_key() -> String {
        hex::encode(SigningKey::from_bytes(&SEED).verifying_key().to_bytes())
    }

    fn sha(data: &[u8]) -> String {
        hex::encode(Sha256::digest(data))
    }

    fn sig_file(data: &[u8], version: &str, seed: [u8; 32]) -> String {
        let head = format!("owlingstone-release 1\nversion {version}\nsha256 {}\n", sha(data));
        let s = SigningKey::from_bytes(&seed).sign(head.as_bytes());
        format!("{head}signature {}\n", hex::encode(s.to_bytes()))
    }

    fn release(data: &[u8], signed: bool) -> Value {
        let mut assets = vec![json!({"name": ASSET_EXE, "browser_download_url": EXE_URL, "digest": format!("sha256:{}", sha(data)), "size": data.len()})];
        if signed {
            assets.push(json!({"name": ASSET_SIG, "browser_download_url": SIG_URL}));
        }
        json!({"tag_name": "v9.9.9", "draft": false, "prerelease": false, "assets": assets})
    }

    fn updater() -> Updater {
        let dir = std::env::temp_dir().join(format!("owl-up-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("Owlingstone.exe");
        std::fs::write(&exe, b"old exe").unwrap();
        Updater { current: "2.0.0".into(), exe_path: exe, state_path: dir.join("update_check.json"), keys: vec![test_key()], ready: None }
    }

    fn files(release: Value, sig: String, exe: &[u8]) -> HashMap<String, Vec<u8>> {
        HashMap::from([
            (API_LATEST.to_string(), release.to_string().into_bytes()),
            (SIG_URL.to_string(), sig.into_bytes()),
            (EXE_URL.to_string(), exe.to_vec()),
        ])
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("1.10.0", "1.9.0") && is_newer("2.0.0", "1.9.0") && !is_newer("1.9.0", "1.9.0"));
    }

    #[test]
    fn pick_release_reads_version_and_digest() {
        let info = pick_release(&release(b"abc", true)).unwrap();
        assert_eq!((info.version.as_str(), info.digest.clone()), ("9.9.9", sha(b"abc")));
        assert_eq!(info.sig_url, SIG_URL);
        assert!(pick_release(&release(b"abc", false)).is_none()); // not signed yet
    }

    #[test]
    fn pick_release_refuses_other_hosts_drafts_and_bad_tags() {
        let mut r = release(b"abc", true);
        r["assets"][0]["browser_download_url"] = json!("https://evil.example/Owlingstone.exe");
        assert!(pick_release(&r).is_none());
        let mut r = release(b"abc", true);
        r["draft"] = json!(true);
        assert!(pick_release(&r).is_none());
        let mut r = release(b"abc", true);
        r["tag_name"] = json!("latest");
        assert!(pick_release(&r).is_none());
    }

    #[test]
    fn signature_checks() {
        let k = test_key();
        assert_eq!(signed_sha256(&sig_file(b"x", "9.9.9", SEED), "9.9.9", &[&k]).unwrap(), sha(b"x"));
        assert!(signed_sha256(&sig_file(b"x", "1.0.0", SEED), "9.9.9", &[&k]).is_err()); // other version
        assert!(signed_sha256(&sig_file(b"x", "9.9.9", [0; 32]), "9.9.9", &[&k]).is_err()); // other key
        let windows = sig_file(b"x", "9.9.9", SEED).replace('\n', "\r\n");
        assert!(signed_sha256(&windows, "9.9.9", &[&k]).is_ok());
        assert!(signed_sha256("garbage", "9.9.9", &[&k]).is_err());
    }

    #[test]
    fn real_public_key_is_valid() {
        for k in PUBLIC_KEYS {
            let b: [u8; 32] = hex::decode(k).unwrap().try_into().unwrap();
            assert!(VerifyingKey::from_bytes(&b).is_ok());
        }
    }

    #[test]
    fn python_signature_tool_is_compatible() {
        // Made by bgtracker/ed25519.py (the Python signing tool) with seed bytes 7*32, for b"same bytes".
        const SIG: &str = "owlingstone-release 1\nversion 2.0.0\nsha256 58100dc8fc06562ce3e578231dc948e083520ee49c4b4ee5a5a28bb4b4003feb\nsignature 4b0f4a13f822eb41ee2d93db36c1b9e8f96d59e71b5f517c17d35fbf60d3ae8e1702cd484b2b43440182d6aafd73fc82ababddf002b4bc52367c4d02c8d44806\n";
        const KEY: &str = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c";
        assert_eq!(test_key(), KEY);
        assert_eq!(signed_sha256(SIG, "2.0.0", &[KEY]).unwrap(), "58100dc8fc06562ce3e578231dc948e083520ee49c4b4ee5a5a28bb4b4003feb");
        assert!(signed_sha256(SIG, "2.0.1", &[KEY]).is_err());
    }

    #[test]
    fn download_verify_and_swap() {
        let mut up = updater();
        let new = b"new exe";
        let f = files(release(new, true), sig_file(new, "9.9.9", SEED), new);
        let get = |url: &str, _: u64, _: u64| f.get(url).cloned().ok_or_else(|| format!("404 {url}"));
        let info = up.check_once(&get, 1e9).unwrap().unwrap();
        assert_eq!(info.version, "9.9.9");
        assert_eq!(std::fs::read(up.new_exe()).unwrap(), new);
        // A second check does not ask GitHub again.
        assert!(up.check_once(&|_: &str, _: u64, _: u64| Err("no".into()), 1e9 + 1.0).unwrap().is_some());
    }

    #[test]
    fn tampered_download_is_refused() {
        let mut up = updater();
        let f = files(release(b"new", true), sig_file(b"new", "9.9.9", SEED), b"evil");
        let get = |url: &str, _: u64, _: u64| f.get(url).cloned().ok_or_else(|| "404".to_string());
        assert!(up.check_once(&get, 1e9).is_err());
        assert!(!up.new_exe().exists() && up.ready.is_none());
    }

    #[test]
    fn release_signed_by_another_key_is_refused() {
        // Even with a stolen GitHub account (exe, signature and digest consistent), without our key: refused.
        let mut up = updater();
        let f = files(release(b"evil", true), sig_file(b"evil", "9.9.9", [0; 32]), b"evil");
        let get = |url: &str, _: u64, _: u64| f.get(url).cloned().ok_or_else(|| "404".to_string());
        assert!(up.check_once(&get, 1e9).is_err());
        assert!(!up.new_exe().exists());
    }

    #[test]
    fn same_version_is_not_downloaded_and_checks_wait() {
        let mut up = updater();
        up.current = "9.9.9".into();
        let calls = RefCell::new(Vec::new());
        let f = files(release(b"x", true), sig_file(b"x", "9.9.9", SEED), b"x");
        let get = |url: &str, _: u64, _: u64| {
            calls.borrow_mut().push(url.to_string());
            f.get(url).cloned().ok_or_else(|| "404".to_string())
        };
        assert!(up.check_once(&get, 1e9).unwrap().is_none());
        assert!(up.check_once(&get, 1e9 + 60.0).unwrap().is_none()); // too soon: not asked
        assert_eq!(calls.borrow().len(), 1);
        // "Check for updates now" asks again at once.
        assert!(up.check_now(&get, 1e9 + 61.0).unwrap().is_none());
        assert_eq!(calls.into_inner(), [API_LATEST, API_LATEST]);
    }
}
