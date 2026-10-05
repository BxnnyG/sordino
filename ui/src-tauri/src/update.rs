//! In-app updates from the project's GitHub releases.
//!
//! * Checking is one HTTPS request to the GitHub API (no data about the user or the system), once
//!   a day while the app runs, and can be switched off in the settings.
//! * Installing only happens on the user's click, only for the package format Sordino was
//!   installed with, and only after two checks: the release's `SHA256SUMS` must carry a minisign
//!   signature from the key compiled into the app (`packaging/minisign.pub`) whose trusted
//!   comment names exactly this release, and the downloaded package must match its checksum.
//!   The package manager then runs through `pkexec`, which asks for the password.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const REPO: &str = "BxnnyG/sordino";
const PUBLIC_KEY: &str = include_str!("../../../packaging/minisign.pub");
const MAX_PACKAGE_BYTES: u64 = 300 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Install {
    Arch,
    Deb,
    Rpm,
    Flatpak,
    /// Tarball or self-built: no one-click update.
    Other,
}

#[derive(Clone, Debug, Serialize)]
pub struct Available {
    pub version: String,
    pub tag: String,
    /// Release notes (Markdown) from the release.
    pub notes: String,
    pub url: String,
    /// One-click install is possible; otherwise `reason` says why not.
    pub can_install: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Status {
    pub current: String,
    pub available: Option<Available>,
    /// Last check failed (offline, rate limit, ...).
    pub error: Option<String>,
    /// Unix seconds of the last successful check.
    pub checked_at: Option<u64>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize, Clone)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(concat!("sordino/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// `0.2.0` -> (0, 2, 0). Anything else (pre-release suffixes etc.) is not considered.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim_start_matches('v').split('.');
    let t = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    it.next().is_none().then_some(t)
}

pub fn detect_install() -> Install {
    if std::env::var_os("FLATPAK_ID").is_some() {
        return Install::Flatpak;
    }
    if std::env::consts::ARCH != "x86_64" {
        return Install::Other;
    }
    let owns = |cmd: &str, args: &[&str]| {
        Command::new(cmd)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if owns("pacman", &["-Qq", "bxy-sordino"]) {
        Install::Arch
    } else if owns("dpkg-query", &["-W", "bxy-sordino"]) {
        Install::Deb
    } else if owns("rpm", &["-q", "bxy-sordino"]) {
        Install::Rpm
    } else {
        Install::Other
    }
}

fn package_suffix(install: Install) -> Option<&'static str> {
    match install {
        Install::Arch => Some("-x86_64.pkg.tar.zst"),
        Install::Deb => Some("_amd64.deb"),
        Install::Rpm => Some(".x86_64.rpm"),
        Install::Flatpak | Install::Other => None,
    }
}

fn signing_key() -> Option<minisign_verify::PublicKey> {
    minisign_verify::PublicKey::decode(PUBLIC_KEY).ok()
}

/// Ask GitHub for the newest release and compare it with this build.
pub fn check() -> Result<Option<Available>, String> {
    let current = parse_version(env!("CARGO_PKG_VERSION")).ok_or("bad own version")?;
    let releases: Vec<Release> = agent()
        .get(&format!(
            "https://api.github.com/repos/{REPO}/releases?per_page=10"
        ))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))?;
    let newest = releases
        .into_iter()
        .filter(|r| !r.draft)
        .filter_map(|r| parse_version(&r.tag_name).map(|v| (v, r)))
        .max_by_key(|(v, _)| *v);
    let Some((v, r)) = newest.filter(|(v, _)| *v > current) else {
        return Ok(None);
    };
    let install = detect_install();
    let reason = if signing_key().is_none() {
        Some("signing_not_set_up")
    } else if !r.assets.iter().any(|a| a.name == "SHA256SUMS.minisig") {
        Some("release_not_signed")
    } else {
        match package_suffix(install) {
            None if install == Install::Flatpak => Some("flatpak"),
            None => Some("manual"),
            Some(sfx) if !r.assets.iter().any(|a| a.name.ends_with(sfx)) => Some("no_package"),
            Some(_) => None,
        }
    };
    Ok(Some(Available {
        version: format!("{}.{}.{}", v.0, v.1, v.2),
        tag: r.tag_name,
        notes: r.body.unwrap_or_default(),
        url: r.html_url,
        can_install: reason.is_none(),
        reason: reason.map(String::from),
    }))
}

fn download(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut resp = agent().get(url).call().map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    resp.body_mut()
        .with_config()
        .limit(limit)
        .reader()
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

/// A private directory for the download (cleared first).
fn work_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or("no cache directory")?;
    let dir = base.join("sordino").join("update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

/// The checksum listed for `name` in a verified `SHA256SUMS`.
fn listed_sum<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.split_once("  ")?;
        (file.trim_start_matches('*') == name).then_some(hash)
    })
}

/// Verify the signature over `SHA256SUMS` (and that it was made for `tag`).
fn verify_sums(sums: &[u8], sig: &str, tag: &str) -> Result<(), String> {
    let key = signing_key().ok_or("no signing key in this build")?;
    verify_with(&key, sums, sig, tag)
}

fn verify_with(
    key: &minisign_verify::PublicKey,
    sums: &[u8],
    sig: &str,
    tag: &str,
) -> Result<(), String> {
    let sig = minisign_verify::Signature::decode(sig).map_err(|e| format!("signature: {e}"))?;
    key.verify(sums, &sig, false)
        .map_err(|_| "the release signature is invalid".to_string())?;
    // The trusted comment is covered by the signature; it must name exactly this release, so an
    // old signed release cannot be passed off as a new one.
    if sig.trusted_comment() != format!("sordino {tag}") {
        return Err("the signature belongs to a different release".into());
    }
    Ok(())
}

/// Download, verify and install `tag`. Blocks; run it off the UI thread.
pub fn install(tag: &str) -> Result<(), String> {
    let install = detect_install();
    let suffix = package_suffix(install).ok_or("no one-click update for this installation")?;
    let r: Release = agent()
        .get(&format!(
            "https://api.github.com/repos/{REPO}/releases/tags/{tag}"
        ))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))?;
    let find = |pred: &dyn Fn(&str) -> bool| {
        r.assets
            .iter()
            .find(|a| pred(&a.name))
            .cloned()
            .ok_or_else(|| "the release is missing a file".to_string())
    };
    let sums_asset = find(&|n| n == "SHA256SUMS")?;
    let sig_asset = find(&|n| n == "SHA256SUMS.minisig")?;
    let pkg_asset = find(&|n| n.starts_with("bxy-sordino") && n.ends_with(suffix))?;
    if pkg_asset.size > MAX_PACKAGE_BYTES || pkg_asset.name.contains('/') {
        return Err("unexpected package".into());
    }

    let sums = download(&sums_asset.browser_download_url, 1 << 20)?;
    let sig = download(&sig_asset.browser_download_url, 1 << 16)?;
    let sig = String::from_utf8(sig).map_err(|_| "signature is not text")?;
    verify_sums(&sums, &sig, tag)?;
    let sums = String::from_utf8(sums).map_err(|_| "SHA256SUMS is not text")?;
    let want = listed_sum(&sums, &pkg_asset.name).ok_or("package not listed in SHA256SUMS")?;

    let pkg = download(&pkg_asset.browser_download_url, MAX_PACKAGE_BYTES)?;
    let got: String = Sha256::digest(&pkg)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != want {
        return Err("the download does not match the signed checksum".into());
    }
    let dir = work_dir()?;
    let path = dir.join(&pkg_asset.name);
    std::fs::write(&path, &pkg).map_err(|e| e.to_string())?;
    let result = run_package_manager(install, &path);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn run_package_manager(install: Install, pkg: &Path) -> Result<(), String> {
    let pkg = pkg.to_str().ok_or("bad path")?;
    let args: Vec<&str> = match install {
        Install::Arch => vec!["pacman", "-U", "--noconfirm", pkg],
        Install::Deb => vec!["apt-get", "install", "-y", pkg],
        Install::Rpm => vec!["dnf", "install", "-y", pkg],
        _ => return Err("no one-click update for this installation".into()),
    };
    let out = Command::new("pkexec")
        .args(&args)
        .output()
        .map_err(|e| format!("pkexec: {e}"))?;
    match out.status.code() {
        Some(0) => Ok(()),
        // pkexec: 126 = the user dismissed the password dialog, 127 = not authorised.
        Some(126) | Some(127) => Err("cancelled".into()),
        _ => Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .last()
            .unwrap_or("the package manager failed")
            .to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.2.10"), Some((0, 2, 10)));
        assert!(parse_version("v0.2.10") > parse_version("v0.2.9"));
        assert_eq!(parse_version("v1.0.0-rc1"), None);
        assert_eq!(parse_version("v1.0"), None);
    }

    #[test]
    fn checksum_lines() {
        let sums =
            "aaa  bxy-sordino_0.2.0_amd64.deb\nbbb  bxy-sordino-0.2.0-1-x86_64.pkg.tar.zst\n";
        assert_eq!(listed_sum(sums, "bxy-sordino_0.2.0_amd64.deb"), Some("aaa"));
        assert_eq!(listed_sum(sums, "other"), None);
    }

    /// A throw-away key made only for this test (minisign -G -W), and a signature over
    /// `abc  bxy-sordino_9.9.9_amd64.deb\n` with the trusted comment "sordino v9.9.9".
    const TEST_KEY: &str = "untrusted comment: minisign public key 17802E3B8F61D78F
RWSP12GPOy6AFxesvtvuM30EeNXqSu0BEA8UUkFjU5lNv1bq3JsfiTCh
";
    const TEST_SIG: &str = "untrusted comment: signature from minisign secret key
RUSP12GPOy6AF3U2zGt7asYosWZNkP2a9LCgPQKZc+WXJn6mD9Chr7ijWdY+uPt6L+VsiEmtgulB/9xTuy1uCa7ydZGjoLGWEQA=
trusted comment: sordino v9.9.9
XQ87GLrIp28Br8xTHt/AifNNU/cJkOJrm4fqihCs7n2GhmG67zeGke30pMYwJHh04BtLjMVyqyzged558w13Aw==
";
    const TEST_SUMS: &[u8] = b"abc  bxy-sordino_9.9.9_amd64.deb\n";

    #[test]
    fn signatures_are_checked_against_key_content_and_release() {
        let key = minisign_verify::PublicKey::decode(TEST_KEY).unwrap();
        assert_eq!(verify_with(&key, TEST_SUMS, TEST_SIG, "v9.9.9"), Ok(()));
        assert!(verify_with(
            &key,
            b"abd  bxy-sordino_9.9.9_amd64.deb\n",
            TEST_SIG,
            "v9.9.9"
        )
        .is_err());
        assert!(
            verify_with(&key, TEST_SUMS, TEST_SIG, "v9.9.10").is_err(),
            "a signature for another release must be refused"
        );
        let mut forged = TEST_SIG.replace("sordino v9.9.9", "sordino v9.9.10");
        forged.push('\n');
        assert!(verify_with(&key, TEST_SUMS, &forged, "v9.9.10").is_err());
    }

    #[test]
    fn the_shipped_key_file_is_either_a_key_or_the_placeholder() {
        let has_key = PUBLIC_KEY.lines().any(|l| l.starts_with("RW"));
        assert_eq!(signing_key().is_some(), has_key);
    }

    /// Talks to GitHub: `cargo test -p sordino-ui -- --ignored live_check`.
    #[test]
    #[ignore]
    fn live_check() {
        let r = check();
        eprintln!(
            "{:?}",
            r.as_ref()
                .map(|a| a.as_ref().map(|a| (&a.version, a.can_install, &a.reason)))
        );
        assert!(r.is_ok());
    }
}
