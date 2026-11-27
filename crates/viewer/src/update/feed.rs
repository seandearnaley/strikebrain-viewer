//! The latest release as GitHub reports it, and what it offers this installation.

use std::fmt;

use serde::Deserialize;

use super::install::Owner;

/// The latest release: its version, its page and the files it carries.
#[derive(Debug)]
pub(crate) struct Release {
    pub(crate) version: semver::Version,
    /// The release's page on GitHub, which opens where nothing installs.
    pub(crate) page: String,
    /// Only files GitHub gave a `sha256` digest for: a file without one
    /// cannot be verified, so it is not offered.
    pub(crate) assets: Vec<Asset>,
}

/// One file of a release, with the size and digest GitHub computed for it.
#[derive(Clone, Debug)]
pub(crate) struct Asset {
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) sha256: [u8; 32],
    pub(crate) url: String,
}

/// Why a reply is not a release to offer.
#[derive(Debug)]
pub(crate) enum FeedError {
    /// The reply is not the JSON read.
    Json(serde_json::Error),
    /// The tag is not a version.
    Version(semver::Error),
    /// The release is a draft. GitHub's latest is never one; a feed pointed
    /// elsewhere might be.
    Draft,
    /// The release is a pre-release, by GitHub's flag or by its tag: the
    /// spec offers none. GitHub's latest is never one; a feed pointed
    /// elsewhere might be.
    Prerelease,
    /// The tag carries build metadata (`+...`): it would not name the
    /// release's files and compares oddly with the running version.
    BuildMetadata,
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "the release is not readable: {error}"),
            Self::Version(error) => write!(f, "the release's tag is not a version: {error}"),
            Self::Draft => f.write_str("the release is a draft"),
            Self::Prerelease => f.write_str("the release is a pre-release"),
            Self::BuildMetadata => f.write_str("the release's tag carries build metadata"),
        }
    }
}

impl std::error::Error for FeedError {}

/// The fields of GitHub's reply that are read; the rest is ignored.
#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    size: u64,
    digest: Option<String>,
    browser_download_url: String,
}

impl RawAsset {
    /// The asset, unless its digest is missing or not `sha256:<64 hex>`.
    fn trusted(self) -> Option<Asset> {
        let sha256 = decode_sha256(self.digest?.strip_prefix("sha256:")?)?;
        Some(Asset {
            name: self.name,
            size: self.size,
            sha256,
            url: self.browser_download_url,
        })
    }
}

/// Reads GitHub's latest-release reply.
pub(crate) fn parse(json: &[u8]) -> Result<Release, FeedError> {
    let raw: RawRelease = serde_json::from_slice(json).map_err(FeedError::Json)?;
    if raw.draft {
        return Err(FeedError::Draft);
    }
    if raw.prerelease {
        return Err(FeedError::Prerelease);
    }
    let tag = raw.tag_name.strip_prefix('v').unwrap_or(&raw.tag_name);
    let version = semver::Version::parse(tag).map_err(FeedError::Version)?;
    if !version.pre.is_empty() {
        return Err(FeedError::Prerelease);
    }
    if !version.build.is_empty() {
        return Err(FeedError::BuildMetadata);
    }
    Ok(Release {
        version,
        page: raw.html_url,
        assets: raw
            .assets
            .into_iter()
            .filter_map(RawAsset::trusted)
            .collect(),
    })
}

/// Whether `release` is a greater version than the one running. An equal or
/// lower version is not an update.
pub(crate) fn newer(release: &Release, running: &semver::Version) -> bool {
    release.version > *running
}

/// The file that updates an installation `owner` owns, named as the release
/// workflow publishes it; none where the release page opens instead or the
/// release lacks the file.
pub(crate) fn file_for(release: &Release, owner: Owner) -> Option<&Asset> {
    let version = &release.version;
    let name = match owner {
        Owner::MacBundle | Owner::MacUnreplaceable => {
            format!("Strikebrain-Viewer-{version}-macos-arm64.dmg")
        }
        Owner::WindowsInstaller => format!("Strikebrain-Viewer-{version}-windows-x64-setup.exe"),
        Owner::Deb => format!("strikebrain-viewer_{version}_amd64.deb"),
        Owner::ReleasePage => return None,
    };
    release.assets.iter().find(|asset| asset.name == name)
}

/// Decodes 64 hex digits, in either case, into the digest's 32 bytes.
fn decode_sha256(hex: &str) -> Option<[u8; 32]> {
    let digits = hex.as_bytes();
    if digits.len() != 64 {
        return None;
    }
    let mut bytes = [0_u8; 32];
    for (byte, pair) in bytes.iter_mut().zip(digits.as_chunks::<2>().0) {
        let high = char::from(pair[0]).to_digit(16)?;
        let low = char::from(pair[1]).to_digit(16)?;
        *byte = u8::try_from(high << 4 | low).ok()?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub's latest-release reply, trimmed to the fields read, with the
    /// version set to 0.1.3.
    const SAMPLE: &str = r#"{
  "tag_name": "v0.1.3",
  "html_url": "https://github.com/seandearnaley/strikebrain-viewer/releases/tag/v0.1.3",
  "draft": false,
  "prerelease": false,
  "assets": [
    {"name": "SHA256SUMS", "size": 539, "digest": "sha256:d53fce5fa80c500ff003ae7bf1807a0bb9abae892bb02c2f79a5fc1fa7499397", "browser_download_url": "https://github.com/seandearnaley/strikebrain-viewer/releases/download/v0.1.3/SHA256SUMS"},
    {"name": "strikebrain-viewer-0.1.3-linux-x86_64.tar.gz", "size": 18976050, "digest": "sha256:18e2975f16f2c943054173cade51b41d776a6fc4d19911e6a41edf360ee9add9", "browser_download_url": "https://github.com/seandearnaley/strikebrain-viewer/releases/download/v0.1.3/strikebrain-viewer-0.1.3-linux-x86_64.tar.gz"},
    {"name": "Strikebrain-Viewer-0.1.3-macos-arm64.dmg", "size": 13081978, "digest": "sha256:97375d994f4840e55a5492948f60bfe7c054958829e9efe317a5e43bda198927", "browser_download_url": "https://github.com/seandearnaley/strikebrain-viewer/releases/download/v0.1.3/Strikebrain-Viewer-0.1.3-macos-arm64.dmg"},
    {"name": "Strikebrain-Viewer-0.1.3-windows-x64-setup.exe", "size": 9935736, "digest": "sha256:c2a39b8e706a9a0efe41aef274749cb824d9254dbe1a588daa0bab5ca660f6b2", "browser_download_url": "https://github.com/seandearnaley/strikebrain-viewer/releases/download/v0.1.3/Strikebrain-Viewer-0.1.3-windows-x64-setup.exe"},
    {"name": "strikebrain-viewer-0.1.3-windows-x64.zip", "size": 11614855, "digest": "sha256:e3406e81b461a42e9f359ad8a362ecf2c8a33bf9fb2373f61c829d7f09144a5f", "browser_download_url": "https://github.com/seandearnaley/strikebrain-viewer/releases/download/v0.1.3/strikebrain-viewer-0.1.3-windows-x64.zip"},
    {"name": "strikebrain-viewer_0.1.3_amd64.deb", "size": 14401848, "digest": "sha256:3f6559bc74f0a25cd7a19e624ebe39cfa8da9137d1c1e50b65c45b643aab066d", "browser_download_url": "https://github.com/seandearnaley/strikebrain-viewer/releases/download/v0.1.3/strikebrain-viewer_0.1.3_amd64.deb"}
  ]
}"#;

    /// The sample's DMG digest, decoded from its hex.
    const SAMPLE_DMG_SHA256: [u8; 32] = [
        0x97, 0x37, 0x5d, 0x99, 0x4f, 0x48, 0x40, 0xe5, 0x5a, 0x54, 0x92, 0x94, 0x8f, 0x60, 0xbf,
        0xe7, 0xc0, 0x54, 0x95, 0x88, 0x29, 0xe9, 0xef, 0xe3, 0x17, 0xa5, 0xe4, 0x3b, 0xda, 0x19,
        0x89, 0x27,
    ];

    /// One asset with no digest, one with another algorithm, one with a
    /// digest of the wrong length and one that is not hex.
    const SAMPLE_WITHOUT_DIGEST: &str = r#"{
  "tag_name": "v0.1.3",
  "html_url": "x",
  "assets": [
    {"name": "a", "size": 1, "browser_download_url": "u"},
    {"name": "b", "size": 1, "digest": null, "browser_download_url": "u"},
    {"name": "c", "size": 1, "digest": "sha512:97375d99", "browser_download_url": "u"},
    {"name": "d", "size": 1, "digest": "sha256:97375d99", "browser_download_url": "u"},
    {"name": "e", "size": 1, "digest": "sha256:zz375d994f4840e55a5492948f60bfe7c054958829e9efe317a5e43bda198927", "browser_download_url": "u"}
  ]
}"#;

    #[test]
    fn a_release_parses_its_version_page_and_files() {
        let release = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(release.version, semver::Version::new(0, 1, 3));
        assert_eq!(
            release.page,
            "https://github.com/seandearnaley/strikebrain-viewer/releases/tag/v0.1.3"
        );
        let dmg = release
            .assets
            .iter()
            .find(|a| a.name.contains("macos-arm64"))
            .unwrap();
        assert_eq!(dmg.size, 13_081_978);
        assert_eq!(dmg.sha256, SAMPLE_DMG_SHA256);
        assert_eq!(
            dmg.url,
            "https://github.com/seandearnaley/strikebrain-viewer/releases/download/v0.1.3/Strikebrain-Viewer-0.1.3-macos-arm64.dmg"
        );
    }

    #[test]
    fn only_a_greater_version_is_newer() {
        let release = parse(SAMPLE.as_bytes()).unwrap();
        assert!(newer(&release, &semver::Version::new(0, 1, 2)));
        assert!(!newer(&release, &semver::Version::new(0, 1, 3)));
        assert!(!newer(&release, &semver::Version::new(0, 2, 0)));
    }

    #[test]
    fn a_tag_that_is_not_a_version_or_an_asset_without_a_sha256_is_refused() {
        assert!(matches!(
            parse(br#"{"tag_name":"latest","html_url":"x","assets":[]}"#),
            Err(FeedError::Version(_))
        ));
        // An asset whose digest is missing or not sha256 is dropped, not trusted.
        let release = parse(SAMPLE_WITHOUT_DIGEST.as_bytes()).unwrap();
        assert!(release.assets.is_empty());
    }

    #[test]
    fn a_draft_a_pre_release_or_a_reply_that_is_not_json_is_refused() {
        let draft = br#"{"tag_name":"v1.0.0","html_url":"x","draft":true,"assets":[]}"#;
        assert!(matches!(parse(draft), Err(FeedError::Draft)));
        let pre = br#"{"tag_name":"v1.0.0","html_url":"x","prerelease":true,"assets":[]}"#;
        assert!(matches!(parse(pre), Err(FeedError::Prerelease)));
        assert!(matches!(parse(b"not json"), Err(FeedError::Json(_))));
        assert_eq!(FeedError::Draft.to_string(), "the release is a draft");
    }

    #[test]
    fn a_tag_that_names_a_pre_release_is_refused_whatever_the_flag_says() {
        let tag = br#"{"tag_name":"v1.0.0-rc.1","html_url":"x","prerelease":false,"assets":[]}"#;
        assert!(matches!(parse(tag), Err(FeedError::Prerelease)));
        let untagged = br#"{"tag_name":"1.0.0-beta","html_url":"x","assets":[]}"#;
        assert!(matches!(parse(untagged), Err(FeedError::Prerelease)));
    }

    #[test]
    fn a_tag_with_build_metadata_is_refused() {
        let build = br#"{"tag_name":"v1.0.0+build.5","html_url":"x","assets":[]}"#;
        assert!(matches!(parse(build), Err(FeedError::BuildMetadata)));
        assert_eq!(
            FeedError::BuildMetadata.to_string(),
            "the release's tag carries build metadata"
        );
    }

    #[test]
    fn a_tag_without_the_leading_v_still_parses() {
        let release = parse(br#"{"tag_name":"2.0.1","html_url":"x","assets":[]}"#).unwrap();
        assert_eq!(release.version, semver::Version::new(2, 0, 1));
    }

    #[test]
    fn each_owner_takes_its_own_file() {
        let release = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(
            file_for(&release, Owner::MacBundle).unwrap().name,
            "Strikebrain-Viewer-0.1.3-macos-arm64.dmg"
        );
        assert_eq!(
            file_for(&release, Owner::MacUnreplaceable).unwrap().name,
            "Strikebrain-Viewer-0.1.3-macos-arm64.dmg"
        );
        assert_eq!(
            file_for(&release, Owner::WindowsInstaller).unwrap().name,
            "Strikebrain-Viewer-0.1.3-windows-x64-setup.exe"
        );
        assert_eq!(
            file_for(&release, Owner::Deb).unwrap().name,
            "strikebrain-viewer_0.1.3_amd64.deb"
        );
        assert!(file_for(&release, Owner::ReleasePage).is_none());
    }

    #[test]
    fn a_file_the_release_lacks_is_none() {
        let release = parse(br#"{"tag_name":"v0.1.3","html_url":"x","assets":[]}"#).unwrap();
        assert!(file_for(&release, Owner::MacBundle).is_none());
    }

    #[test]
    fn hex_decoding_reads_both_cases_and_refuses_the_rest() {
        let upper = "97375D994F4840E55A5492948F60BFE7C054958829E9EFE317A5E43BDA198927";
        assert_eq!(decode_sha256(upper), Some(SAMPLE_DMG_SHA256));
        assert_eq!(decode_sha256(""), None);
        assert_eq!(decode_sha256(&"0".repeat(63)), None);
        assert_eq!(decode_sha256(&"0".repeat(65)), None);
        assert_eq!(decode_sha256(&"g".repeat(64)), None);
        // Multi-byte characters never split a pair.
        assert_eq!(decode_sha256(&"é".repeat(32)), None);
    }
}
