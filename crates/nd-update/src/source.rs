//! The Velopack `UpdateSource` that refuses a feed whose signature does not verify.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use velopack::bundle::Manifest;
use velopack::sources::{FileSource, HttpSource, UpdateSource};
use velopack::{Error, VelopackAsset, VelopackAssetFeed};

use crate::feed::{self, PublicKey};

/// Where the static feed directory lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    Http(String),
    Dir(PathBuf),
}

impl Location {
    /// `http(s)://` URLs, `file://` URLs and plain paths.
    pub fn parse(feed: &str) -> Self {
        let lower = feed.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            Self::Http(feed.trim_end_matches('/').to_owned())
        } else if let Some(path) = feed.strip_prefix("file://") {
            Self::Dir(PathBuf::from(path))
        } else {
            Self::Dir(PathBuf::from(feed))
        }
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, Error> {
        match self {
            Self::Dir(dir) => Ok(std::fs::read(dir.join(name))?),
            Self::Http(base) => {
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_global(Some(std::time::Duration::from_secs(30)))
                    .build()
                    .into();
                let mut response = agent.get(format!("{base}/{name}")).call()?;
                Ok(response.body_mut().read_to_vec()?)
            }
        }
    }
}

/// Fetches `releases.<channel>.json` and its `.sig`, verifies them against the app's key, and only
/// then hands the feed to Velopack. Downloads go through Velopack's own sources, which check each
/// package's size and SHA-256 against the (now trusted) feed.
pub struct VerifiedSource {
    location: Location,
    key: PublicKey,
    inner: Box<dyn UpdateSource>,
}

impl VerifiedSource {
    pub fn new(location: Location, key: PublicKey) -> Self {
        let inner: Box<dyn UpdateSource> = match &location {
            Location::Http(url) => Box::new(HttpSource::new(url)),
            Location::Dir(dir) => Box::new(FileSource::new(dir)),
        };
        Self { location, key, inner }
    }

    /// The verified, parsed feed. Exposed for tests and `nd release` checks.
    pub fn fetch(&self, channel: &str, app_id: &str) -> Result<VelopackAssetFeed, Error> {
        let json = self.location.read(&feed::feed_file(channel))?;
        let signature = self
            .location
            .read(&feed::signature_file(channel))
            .map_err(|e| Error::Other(format!("{}: {e}; refusing an unsigned feed", feed::signature_file(channel))))?;
        let signature = String::from_utf8_lossy(&signature);
        self.key
            .verify(channel, &json, &signature)
            .map_err(|e| Error::Other(format!("{}: {e}", feed::feed_file(channel))))?;
        let feed: VelopackAssetFeed = serde_json::from_slice(&json)?;
        for asset in &feed.Assets {
            check_asset(asset, app_id)?;
        }
        Ok(feed)
    }
}

/// A signed feed is trusted for content, but its file names still become paths under the
/// packages directory, so they must be plain names of this app's packages.
fn check_asset(asset: &VelopackAsset, app_id: &str) -> Result<(), Error> {
    let name = &asset.FileName;
    let plain = !name.is_empty()
        && Path::new(name).file_name().is_some_and(|n| n == name.as_str())
        && !name.contains(['/', '\\'])
        && name != ".."
        && name.ends_with(".nupkg");
    if !plain {
        return Err(Error::Other(format!("feed lists an unsafe package file name `{name}`")));
    }
    if !asset.PackageId.eq_ignore_ascii_case(app_id) {
        return Err(Error::Other(format!("feed lists package `{}`, this app is `{app_id}`", asset.PackageId)));
    }
    Ok(())
}

impl UpdateSource for VerifiedSource {
    fn get_release_feed(
        &self,
        channel: &str,
        app: &Manifest,
        _staged_user_id: &str,
    ) -> Result<VelopackAssetFeed, Error> {
        self.fetch(channel, &app.id)
    }

    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        local_file: &Path,
        progress: Option<Sender<i16>>,
    ) -> Result<(), Error> {
        self.inner.download_release_entry(asset, local_file, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::SecretKey;

    const FEED: &str = r#"{"Assets":[{"PackageId":"dev.example.demo","Version":"0.1.1","Type":"Full","FileName":"dev.example.demo-0.1.1-stable-full.nupkg","SHA1":"A","SHA256":"B","Size":3}]}"#;

    fn dir_with(feed: &str, sig: Option<String>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("releases.stable.json"), feed).unwrap();
        if let Some(sig) = sig {
            std::fs::write(dir.path().join("releases.stable.json.sig"), sig).unwrap();
        }
        dir
    }

    #[test]
    fn a_signed_feed_parses() {
        let key = SecretKey::generate();
        let dir = dir_with(FEED, Some(key.sign("stable", FEED.as_bytes())));
        let source = VerifiedSource::new(Location::Dir(dir.path().into()), key.public_key());
        let feed = source.fetch("stable", "dev.example.demo").unwrap();
        assert_eq!(feed.Assets[0].Version, "0.1.1");
    }

    #[test]
    fn tampered_unsigned_and_foreign_feeds_are_refused() {
        let key = SecretKey::generate();
        let tampered = FEED.replace("\"Size\":3", "\"Size\":4");
        let dir = dir_with(&tampered, Some(key.sign("stable", FEED.as_bytes())));
        let source = VerifiedSource::new(Location::Dir(dir.path().into()), key.public_key());
        let err = source.fetch("stable", "dev.example.demo").unwrap_err().to_string();
        assert!(err.contains("does not match"), "{err}");

        let dir = dir_with(FEED, None);
        let source = VerifiedSource::new(Location::Dir(dir.path().into()), key.public_key());
        assert!(source.fetch("stable", "dev.example.demo").unwrap_err().to_string().contains("unsigned"));

        let dir = dir_with(FEED, Some(key.sign("stable", FEED.as_bytes())));
        let source = VerifiedSource::new(Location::Dir(dir.path().into()), key.public_key());
        assert!(source.fetch("stable", "dev.example.other").is_err());
    }

    #[test]
    fn signed_feeds_still_cannot_name_paths() {
        let key = SecretKey::generate();
        let evil = FEED.replace("dev.example.demo-0.1.1-stable-full.nupkg", "../../evil.nupkg");
        let dir = dir_with(&evil, Some(key.sign("stable", evil.as_bytes())));
        let source = VerifiedSource::new(Location::Dir(dir.path().into()), key.public_key());
        assert!(source.fetch("stable", "dev.example.demo").unwrap_err().to_string().contains("unsafe"));
    }

    #[test]
    fn locations_parse() {
        assert_eq!(Location::parse("https://u.example/app/"), Location::Http("https://u.example/app".into()));
        assert_eq!(Location::parse("file:///srv/feed"), Location::Dir("/srv/feed".into()));
        assert_eq!(Location::parse("/srv/feed"), Location::Dir("/srv/feed".into()));
    }
}
