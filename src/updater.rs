//! Release discovery and installation. Call these blocking operations on a worker.

use std::time::Duration;

use self_update::{Release, ReleaseAsset, ReleaseSource};
use semver::Version;
use serde::Deserialize;

use crate::models::UpdateChannel;

pub(crate) const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub(crate) struct AvailableUpdate {
    release: Release,
    platform: Platform,
    available: bool,
}

impl AvailableUpdate {
    pub(crate) fn version(&self) -> &str {
        self.release.version()
    }

    pub(crate) fn is_available(&self) -> bool {
        self.available
    }
}

pub(crate) fn check(channel: UpdateChannel) -> Result<AvailableUpdate, String> {
    let platform = Platform::current()?;
    let endpoint = match channel {
        UpdateChannel::Release => "latest",
        UpdateChannel::Nightly => "tags/nightly",
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let release = agent
        .get(format!(
            "https://api.github.com/repos/hlcfan/beam/releases/{endpoint}"
        ))
        .header("User-Agent", "Beam-Updater")
        .header("Accept", "application/vnd.github+json")
        .call()
        .and_then(|mut response| response.body_mut().read_json::<GitHubRelease>())
        .map_err(|error| format!("Could not check for updates: {error}"))?
        .into_release(channel)?;
    prepare_update(channel, release, platform, CURRENT_VERSION)
}

pub(crate) fn install(update: AvailableUpdate) -> Result<(), String> {
    if !update.available {
        return Err("This version is already up to date.".into());
    }
    let key = verification_key(option_env!("BEAM_UPDATE_PUBLIC_KEY"))?;
    let mut builder = self_update::backends::custom::Update::configure();
    builder
        .source(PinnedRelease(update.release.clone()))
        .bin_name("beam")
        .current_version(CURRENT_VERSION)
        // Pin the release the user saw, including an explicitly selected nightly.
        .release_tag(update.version())
        .asset_matcher(|assets| assets.first().cloned())
        .verifying_keys([key])
        .check_install_path_writable(true)
        .timeout(Duration::from_secs(300))
        .retries(1)
        .show_output(false)
        .no_confirm(true);
    match update.platform {
        Platform::MacArm => {
            builder.bundle_path_in_archive("beam-macos-aarch64/Beam.app");
        }
        Platform::LinuxX64 => {
            builder.bin_path_in_archive("beam-linux-x86_64/beam");
        }
        Platform::WindowsX64 => {
            builder.bin_path_in_archive("beam.exe");
        }
    }
    let status = builder
        .build()
        .and_then(|updater| updater.update())
        .map_err(|error| format!("Could not install the update: {error}"))?;
    if !status.is_updated() {
        return Err("The update was not installed. Check for updates and try again.".into());
    }
    Ok(())
}

pub(crate) fn installation_error() -> Option<String> {
    verification_key(option_env!("BEAM_UPDATE_PUBLIC_KEY")).err()
}

// The GitHub backend in self_update requires semver tags. Parse the rolling
// nightly tag here, then hand a single immutable release to its install engine.
#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: Option<String>,
    draft: bool,
    prerelease: bool,
    assets: Vec<GitHubAsset>,
}

impl GitHubRelease {
    fn into_release(self, channel: UpdateChannel) -> Result<Release, String> {
        if self.draft
            || (channel == UpdateChannel::Release && self.prerelease)
            || (channel == UpdateChannel::Nightly && self.tag_name != "nightly")
        {
            return Err("The release does not match the selected channel.".into());
        }
        let name = self.name.unwrap_or_default();
        let version = match channel {
            UpdateChannel::Release => self.tag_name.trim_start_matches('v'),
            UpdateChannel::Nightly => name
                .strip_prefix("Nightly ")
                .ok_or("This nightly predates in-app updates. Wait for a new nightly build.")?,
        };
        Release::builder()
            .version(version)
            .name(&name)
            .assets(
                self.assets
                    .into_iter()
                    .map(|asset| {
                        let mut download =
                            ReleaseAsset::new(asset.name, asset.browser_download_url);
                        if let Some(digest) = asset.digest {
                            download = download.with_digest(digest);
                        }
                        download
                    })
                    .collect::<Vec<_>>(),
            )
            .build()
            .map_err(|error| error.to_string())
    }
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

fn prepare_update(
    channel: UpdateChannel,
    release: Release,
    platform: Platform,
    current: &str,
) -> Result<AvailableUpdate, String> {
    let version = match channel {
        UpdateChannel::Release => release.version(),
        UpdateChannel::Nightly => release
            .name()
            .strip_prefix("Nightly ")
            .ok_or("This nightly predates in-app updates. Wait for a new nightly build.")?,
    };
    let remote = Version::parse(version)
        .map_err(|_| "The release does not contain a valid update version.".to_string())?;
    let current = Version::parse(current).map_err(|error| error.to_string())?;
    match channel {
        UpdateChannel::Release if !remote.pre.is_empty() => {
            return Err("The normal release channel returned a prerelease.".into());
        }
        UpdateChannel::Nightly if !is_nightly(&remote) => {
            return Err("The nightly release does not contain a nightly build version.".into());
        }
        _ => {}
    }
    let expected = platform.archive_name(channel, &remote);
    let asset = release
        .assets()
        .iter()
        .find(|asset| asset.name() == expected)
        .ok_or_else(|| format!("This release has no download for {}.", platform.label()))?;
    if !asset.download_url().starts_with("https://") {
        return Err("The update download must use HTTPS.".into());
    }
    let available = match channel {
        UpdateChannel::Release => remote.cmp_precedence(&current).is_gt() || is_nightly(&current),
        UpdateChannel::Nightly => {
            // The rolling release identifies the latest commit. SHAs have no chronological order.
            let remote_base = (remote.major, remote.minor, remote.patch);
            let current_base = (current.major, current.minor, current.patch);
            remote_base > current_base || (remote_base == current_base && remote != current)
        }
    };
    let release = Release::builder()
        .version(remote.to_string())
        .asset(asset.clone())
        .build()
        .map_err(|error| error.to_string())?;
    Ok(AvailableUpdate {
        release,
        platform,
        available,
    })
}

fn is_nightly(version: &Version) -> bool {
    let prerelease = version.pre.as_str();
    prerelease == "nightly" || prerelease.starts_with("nightly.")
}

fn verification_key(value: Option<&str>) -> Result<self_update::VerifyingKey, String> {
    let value = value.filter(|value| !value.is_empty()).ok_or(
        "This build cannot verify updates. Install an official release to enable updates.",
    )?;
    if value.len() != 64 || !value.is_ascii() {
        return Err("The update verification key is invalid.".into());
    }
    let mut key = [0; 32];
    for (byte, hex) in key.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
        let hex = std::str::from_utf8(hex).map_err(|error| error.to_string())?;
        *byte = u8::from_str_radix(hex, 16)
            .map_err(|_| "The update verification key is invalid.".to_string())?;
    }
    Ok(key)
}

struct PinnedRelease(Release);

impl ReleaseSource for PinnedRelease {
    fn get_releases(&self) -> self_update::Result<Vec<Release>> {
        Ok(vec![self.0.clone()])
    }
}

#[derive(Clone, Copy, Debug)]
enum Platform {
    MacArm,
    LinuxX64,
    WindowsX64,
}

impl Platform {
    fn current() -> Result<Self, String> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Ok(Self::MacArm),
            ("linux", "x86_64") => Ok(Self::LinuxX64),
            ("windows", "x86_64") => Ok(Self::WindowsX64),
            _ => Err("In-app updates are not available for this platform.".into()),
        }
    }

    fn archive_name(self, channel: UpdateChannel, version: &Version) -> String {
        let (name, extension) = match self {
            Self::MacArm => ("beam-macos-aarch64", "tar.gz"),
            Self::LinuxX64 => ("beam-linux-x86_64", "tar.gz"),
            Self::WindowsX64 => ("beam-windows-x86_64", "zip"),
        };
        match channel {
            UpdateChannel::Release => format!("{name}.{extension}"),
            UpdateChannel::Nightly => format!("{name}-{version}.{extension}"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::MacArm => "macOS Apple Silicon",
            Self::LinuxX64 => "Linux x86_64",
            Self::WindowsX64 => "Windows x86_64",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_verification_rejects_unsigned_tampered_and_wrong_key_downloads() {
        use std::io::Cursor;
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("beam.tar.gz");
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, "beam", b"beam".as_slice())
            .expect("tar entry");
        let bytes = archive.into_inner().expect("tar").finish().expect("gzip");
        let signer = zipsign_api::SigningKey::from_bytes(&[42; 32]);
        let key = signer.verifying_key().to_bytes();
        std::fs::write(&path, &bytes).expect("unsigned archive");
        assert!(self_update::verify_signature(&path, &[key]).is_err());
        let mut signed = Cursor::new(Vec::new());
        zipsign_api::sign::copy_and_sign_tar(
            &mut Cursor::new(bytes),
            &mut signed,
            &[signer],
            Some(b"beam.tar.gz"),
        )
        .expect("sign archive");
        let mut bytes = signed.into_inner();
        std::fs::write(&path, &bytes).expect("signed archive");
        self_update::verify_signature(&path, &[key]).expect("valid signature");
        let wrong_key = zipsign_api::SigningKey::from_bytes(&[43; 32])
            .verifying_key()
            .to_bytes();
        assert!(self_update::verify_signature(&path, &[wrong_key]).is_err());
        bytes[15] ^= 1;
        std::fs::write(&path, bytes).expect("tampered archive");
        assert!(self_update::verify_signature(&path, &[key]).is_err());
    }

    #[test]
    fn rolling_nightly_metadata_becomes_a_pinned_semver_release() {
        let json = r#"{"tag_name":"nightly","name":"Nightly 0.8.0-nightly+g0123456789ab","draft":false,"prerelease":true,"assets":[{"name":"beam-linux-x86_64-0.8.0-nightly+g0123456789ab.tar.gz","browser_download_url":"https://example.com/beam.tar.gz","digest":"sha256:abc"}]}"#;
        let release: GitHubRelease = serde_json::from_str(json).expect("metadata");
        let release = release
            .into_release(UpdateChannel::Nightly)
            .expect("nightly");
        assert_eq!(release.version(), "0.8.0-nightly+g0123456789ab");
        assert_eq!(release.assets()[0].digest(), Some("sha256:abc"));
        assert_eq!(
            PinnedRelease(release)
                .get_release_version("0.8.0-nightly+g0123456789ab")
                .expect("pinned release")
                .version(),
            "0.8.0-nightly+g0123456789ab"
        );
    }

    fn release(version: &str, name: &str, assets: &[&str]) -> Release {
        Release::builder()
            .version(version)
            .name(name)
            .assets(
                assets
                    .iter()
                    .map(|name| ReleaseAsset::new(*name, format!("https://example.com/{name}")))
                    .collect::<Vec<_>>(),
            )
            .build()
            .expect("release")
    }

    #[test]
    fn stable_selects_exact_platform_archive_and_compares_versions() {
        for (platform, archive) in [
            (Platform::MacArm, "beam-macos-aarch64.tar.gz"),
            (Platform::LinuxX64, "beam-linux-x86_64.tar.gz"),
            (Platform::WindowsX64, "beam-windows-x86_64.zip"),
        ] {
            let release = release(
                "0.9.0",
                "v0.9.0",
                &["beam-linux-x86_64.tar.gz.sha256", archive],
            );
            let newer = prepare_update(UpdateChannel::Release, release.clone(), platform, "0.8.0")
                .expect("newer");
            assert!(newer.is_available());
            assert_eq!(newer.release.assets()[0].name(), archive);
            for current in ["0.9.0", "1.0.0"] {
                assert!(
                    !prepare_update(UpdateChannel::Release, release.clone(), platform, current)
                        .expect("check")
                        .is_available()
                );
            }
        }
    }

    #[test]
    fn nightly_updates_compare_base_version_and_commit_identity() {
        let version = "0.8.0-nightly+g0123456789ab";
        let archive = "beam-linux-x86_64-0.8.0-nightly+g0123456789ab.tar.gz";
        let release = release(version, &format!("Nightly {version}"), &[archive]);
        for (current, available) in [
            ("0.8.0", true),
            ("0.7.0-nightly+gffffffffffff", true),
            ("0.8.0-nightly+g0123456789ab", false),
            ("0.8.0-nightly+g000000000000", true),
            ("0.8.0-nightly+gffffffffffff", true),
            ("0.9.0", false),
            ("0.9.0-nightly+g000000000000", false),
        ] {
            let update = prepare_update(
                UpdateChannel::Nightly,
                release.clone(),
                Platform::LinuxX64,
                current,
            )
            .expect("nightly with commit SHA");
            assert_eq!(update.is_available(), available, "{current}");
            assert_eq!(update.version(), version);
            assert_eq!(update.release.assets()[0].name(), archive);
        }
    }

    #[test]
    fn switching_from_nightly_offers_the_latest_normal_release() {
        let release = release("0.8.0", "v0.8.0", &["beam-linux-x86_64.tar.gz"]);
        assert!(
            prepare_update(
                UpdateChannel::Release,
                release,
                Platform::LinuxX64,
                "0.9.0-nightly+g0123456789ab"
            )
            .expect("normal release")
            .is_available()
        );
    }

    #[test]
    fn rejects_missing_archives_and_wrong_channel_metadata() {
        let stable = release("0.9.0", "v0.9.0", &["beam-linux-x86_64.tar.gz.sha256"]);
        assert!(
            prepare_update(UpdateChannel::Release, stable, Platform::LinuxX64, "0.8.0").is_err()
        );
        let nightly = release("0.9.0-beta.1", "Nightly", &["beam-linux-x86_64.tar.gz"]);
        for channel in [UpdateChannel::Release, UpdateChannel::Nightly] {
            assert!(prepare_update(channel, nightly.clone(), Platform::LinuxX64, "0.8.0").is_err());
        }
    }

    #[test]
    fn verification_requires_an_embedded_valid_key() {
        for invalid in [
            None,
            Some(""),
            Some("00"),
            Some(&"z".repeat(64)),
            Some(&"é".repeat(32)),
        ] {
            assert!(verification_key(invalid).is_err());
        }
        assert_eq!(
            verification_key(Some(&"ab".repeat(32))).expect("key"),
            [0xab; 32]
        );
    }
}
