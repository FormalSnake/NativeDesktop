//! Publishers for the feed directory (`[updates.publish]`). Each one copies static files with
//! relative names; none talks to a service of ours. Packages go first and the feed plus its
//! signature last, so a client never sees a feed listing a package that is not there yet. A
//! client that reads the new feed with the old signature in between refuses it and retries later.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use nd_config::Publish;
use rusty_s3::{Bucket, Credentials, S3Action as _, UrlStyle};

/// Files of `dir` in upload order: everything else, then `releases.*.json`, then `*.sig`.
pub fn upload_order(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    let rank = |p: &PathBuf| {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.ends_with(".sig") {
            2
        } else if name.starts_with("releases.") && name.ends_with(".json") {
            1
        } else {
            0
        }
    };
    files.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));
    Ok(files)
}

pub fn publish(publish: &Publish, dir: &Path, project_root: &Path) -> Result<()> {
    let files = upload_order(dir)?;
    match publish {
        Publish::Dir { path } => to_dir(&files, &project_root.join(path)),
        Publish::Rsync { target } => rsync(dir, target),
        Publish::S3 { endpoint, endpoint_env, bucket, prefix, region } => {
            let endpoint = match (endpoint, endpoint_env) {
                (Some(url), _) => url.clone(),
                (None, Some(var)) => {
                    std::env::var(var).with_context(|| format!("{var} (updates.publish.endpoint_env)"))?
                }
                (None, None) => bail!("updates.publish: s3 needs endpoint or endpoint_env"),
            };
            let credentials = Credentials::new(
                std::env::var("AWS_ACCESS_KEY_ID").context("AWS_ACCESS_KEY_ID for the s3 publisher")?,
                std::env::var("AWS_SECRET_ACCESS_KEY").context("AWS_SECRET_ACCESS_KEY for the s3 publisher")?,
            );
            let target = S3Target {
                endpoint,
                bucket: bucket.clone(),
                prefix: prefix.clone(),
                region: region.clone().unwrap_or_else(|| "auto".into()),
            };
            to_s3(&files, &target, &credentials)
        }
        Publish::Github { repo } => github(&files, repo.as_deref(), &GhCommand::default()),
    }
}

fn to_dir(files: &[PathBuf], target: &Path) -> Result<()> {
    std::fs::create_dir_all(target).with_context(|| format!("creating {}", target.display()))?;
    for file in files {
        let name = file.file_name().context("file name")?;
        // Copy under a temporary name and rename, so a reader never sees half a file.
        let tmp = target.join(format!(".{}.part", name.to_string_lossy()));
        std::fs::copy(file, &tmp).with_context(|| format!("copying {}", file.display()))?;
        std::fs::rename(&tmp, target.join(name))?;
    }
    Ok(())
}

fn rsync(dir: &Path, target: &str) -> Result<()> {
    // Two passes keep the order: packages, then the feed and signature.
    let source = format!("{}/", dir.display());
    for pass in
        [&["--exclude", "releases.*", "--exclude", ".*"][..], &["--include", "releases.*", "--exclude", "*"][..]]
    {
        let status = Command::new("rsync")
            .args(["-a", "--delay-updates"])
            .args(pass)
            .arg(&source)
            .arg(target)
            .status()
            .context("running rsync")?;
        if !status.success() {
            bail!("rsync to {target} failed ({status})");
        }
    }
    Ok(())
}

pub struct S3Target {
    pub endpoint: String,
    pub bucket: String,
    pub prefix: Option<String>,
    pub region: String,
}

/// Path-style presigned PUTs, which R2, S3 and MinIO all accept.
pub fn to_s3(files: &[PathBuf], target: &S3Target, credentials: &Credentials) -> Result<()> {
    let endpoint = target.endpoint.parse().with_context(|| format!("s3 endpoint `{}`", target.endpoint))?;
    let bucket = Bucket::new(endpoint, UrlStyle::Path, target.bucket.clone(), target.region.clone())
        .map_err(|e| anyhow::anyhow!("s3 bucket: {e}"))?;
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).build().into();
    for file in files {
        let name = file.file_name().context("file name")?.to_string_lossy().into_owned();
        let key = match target.prefix.as_deref().map(|p| p.trim_matches('/')) {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}/{name}"),
            _ => name.clone(),
        };
        let url = bucket.put_object(Some(credentials), &key).sign(Duration::from_secs(3600));
        let body = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        let content_type = if name.ends_with(".json") { "application/json" } else { "application/octet-stream" };
        agent
            .put(url.as_str())
            .header("content-type", content_type)
            .send(&body[..])
            .with_context(|| format!("uploading {key}"))?;
    }
    Ok(())
}

/// The `gh` binary; tests point it at a stand-in.
#[derive(Default)]
pub struct GhCommand {
    pub program: Option<PathBuf>,
}

pub const GITHUB_TAG: &str = "updates";

pub fn github(files: &[PathBuf], repo: Option<&str>, gh: &GhCommand) -> Result<()> {
    let program = gh.program.clone().unwrap_or_else(|| "gh".into());
    let run = |args: &[&str], extra: &[PathBuf]| -> Result<bool> {
        let mut command = Command::new(&program);
        command.args(args).args(extra);
        if let Some(repo) = repo {
            command.args(["--repo", repo]);
        }
        Ok(command.status().with_context(|| format!("running {}", program.display()))?.success())
    };
    if !run(&["release", "view", GITHUB_TAG], &[])?
        && !run(
            &[
                "release",
                "create",
                GITHUB_TAG,
                "--title",
                "Update feed",
                "--notes",
                "Static update feed written by nd release.",
            ],
            &[],
        )?
    {
        bail!("gh release create {GITHUB_TAG} failed");
    }
    // One upload per group keeps the packages-before-feed order.
    let (packages, feed): (Vec<PathBuf>, Vec<PathBuf>) = files.iter().cloned().partition(|f| {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        !(name.starts_with("releases.") || name.ends_with(".sig"))
    });
    for group in [packages, feed] {
        if !group.is_empty() && !run(&["release", "upload", GITHUB_TAG, "--clobber"], &group)? {
            bail!("gh release upload failed");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in [
            ("releases.mac-stable.json", "{}"),
            ("releases.mac-stable.json.sig", "sig"),
            ("app-0.1.1-mac-stable-full.nupkg", "full"),
            ("app-0.1.1-mac-stable-delta.nupkg", "delta"),
            (".hidden", "x"),
        ] {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        dir
    }

    fn names(files: &[PathBuf]) -> Vec<String> {
        files.iter().map(|f| f.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn packages_upload_before_the_feed_and_its_signature() {
        let dir = feed_dir();
        assert_eq!(
            names(&upload_order(dir.path()).unwrap()),
            [
                "app-0.1.1-mac-stable-delta.nupkg",
                "app-0.1.1-mac-stable-full.nupkg",
                "releases.mac-stable.json",
                "releases.mac-stable.json.sig"
            ]
        );
    }
}
