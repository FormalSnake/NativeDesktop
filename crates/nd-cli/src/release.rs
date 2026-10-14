//! `nd release`: pack (with deltas against the previous release), sign
//! `releases.<channel>.json`, write the static feed directory, publish it.
//! `nd release serve` serves that directory; `nd release keygen` makes the signing key.
//!
//! The feed directory is `target/nd/releases`, one per app, shared by every OS and channel.
//! Velopack's channel is `<os>-<channel>` (`mac-stable`), so a mac and a Windows release on
//! `stable` never overwrite each other's `releases.*.json`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use nd_update::feed::{self, PublicKey, SecretKey};

use crate::package::{self, Options, Os};
use crate::project::Project;
use crate::{publish, vpk};

#[derive(Args, Debug)]
#[command(args_conflicts_with_subcommands = true)]
pub struct ReleaseArgs {
    #[command(subcommand)]
    pub command: Option<ReleaseCommand>,
    #[command(flatten)]
    pub release: ReleaseRun,
}

#[derive(Args, Debug, Clone)]
pub struct ReleaseRun {
    /// App directory.
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,
    /// Target OS; defaults to this one.
    #[arg(long, value_enum)]
    pub os: Option<Os>,
    /// One of `updates.channels`; defaults to the first.
    #[arg(long)]
    pub channel: Option<String>,
    /// Markdown release notes, shown in the app's "what's new".
    #[arg(long)]
    pub notes: Option<PathBuf>,
    /// Signing key file (base64 seed). Else `ND_UPDATE_SIGNING_KEY` (the key itself), else
    /// the file `nd release keygen` wrote.
    #[arg(long)]
    pub key: Option<PathBuf>,
    /// Write and sign the feed directory without uploading it.
    #[arg(long)]
    pub no_publish: bool,
    /// Prebuilt binary to pack (cross-packing).
    #[arg(long)]
    pub binary: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum ReleaseCommand {
    /// Serve the feed directory over HTTP for update tests.
    Serve(ServeArgs),
    /// Make an ed25519 signing key and print the public key for `[updates] public_key`.
    Keygen(KeygenArgs),
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,
    /// Serve this directory instead of the app's `target/nd/releases`.
    #[arg(long)]
    pub root: Option<PathBuf>,
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    #[arg(long, default_value_t = 8790)]
    pub port: u16,
}

#[derive(Args, Debug)]
pub struct KeygenArgs {
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,
    /// Where to write the private key; defaults to `~/.config/nd/keys/<app id>.key`.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

pub fn run(args: ReleaseArgs) -> Result<()> {
    match args.command {
        Some(ReleaseCommand::Serve(serve_args)) => serve(serve_args),
        Some(ReleaseCommand::Keygen(keygen_args)) => keygen(keygen_args),
        None => release(&args.release).map(|_| ()),
    }
}

/// `mac-stable`, `linux-beta`, `win-stable`.
pub fn velopack_channel(os: Os, channel: &str) -> String {
    let os = match os {
        Os::Mac => "mac",
        Os::Linux => "linux",
        Os::Windows => "win",
    };
    format!("{os}-{channel}")
}

pub fn feed_dir(project: &Project) -> PathBuf {
    project.nd_dir().join("releases")
}

/// Packs, signs and (unless `no_publish`) publishes; returns the feed directory.
pub fn release(args: &ReleaseRun) -> Result<PathBuf> {
    let project = Project::load(&args.dir)?;
    let updates = &project.config.updates;
    let public = updates
        .public_key
        .as_deref()
        .context("nd release needs [updates] feed and public_key in nativedesktop.toml (see nd release keygen)")?;
    let public = PublicKey::parse(public)?;
    let key = signing_key(&project, args.key.as_deref())?;
    if key.public_key().to_base64() != public.to_base64() {
        bail!("the signing key does not match updates.public_key; the app would refuse this feed");
    }
    let os = args.os.unwrap_or_else(Os::host);
    let channel = velopack_channel(os, &project.channel(args.channel.as_deref())?);
    let dir = feed_dir(&project);
    std::fs::create_dir_all(&dir)?;
    seed_previous_release(&project, &dir, &channel)?;

    let opts = Options {
        channel: channel.clone(),
        releases: dir.clone(),
        notes: args.notes.clone(),
        binary: args.binary.clone(),
        runtime: None,
    };
    let packed = package::pack(&project, os, &opts)?;
    sign_feed(&dir, &channel, &key)?;
    println!("feed: {} ({channel}, signed)", dir.display());
    if let Some(installer) = &packed.installer {
        println!("installer: {}", installer.display());
    }
    if args.no_publish {
        return Ok(dir);
    }
    match &updates.publish {
        Some(target) => {
            publish::publish(target, &dir, &project.root)?;
            println!("published");
        }
        None => println!("no [updates.publish]: serve or upload {} yourself", dir.display()),
    }
    Ok(dir)
}

/// Writes `releases.<channel>.json.sig` and checks it back.
pub fn sign_feed(dir: &Path, channel: &str, key: &SecretKey) -> Result<()> {
    let json_path = dir.join(feed::feed_file(channel));
    let json = std::fs::read(&json_path).with_context(|| format!("vpk wrote no {}", json_path.display()))?;
    let signature = key.sign(channel, &json);
    key.public_key().verify(channel, &json, &signature)?;
    std::fs::write(dir.join(feed::signature_file(channel)), signature)?;
    Ok(())
}

/// Deltas are computed against the newest full package in the output directory. A fresh
/// checkout (CI) has none, so the latest one is fetched from the live feed first when it is
/// served over HTTP.
fn seed_previous_release(project: &Project, dir: &Path, channel: &str) -> Result<()> {
    let has_full = package::listing(dir).iter().any(|n| n.contains(&format!("-{channel}-full.nupkg")));
    let Some(feed) = project.config.updates.feed.as_deref() else { return Ok(()) };
    if has_full || !(feed.starts_with("http://") || feed.starts_with("https://")) {
        return Ok(());
    }
    let vpk = vpk::locate()?;
    let args: Vec<String> =
        ["download", "http", "--url", feed, "--channel", channel, "--allowEmptyChannel", "--outputDir"]
            .iter()
            .map(|s| s.to_string())
            .chain([dir.display().to_string()])
            .collect();
    if let Err(e) = vpk::run(&vpk, &args) {
        eprintln!("nd: no previous release fetched from {feed}, this one ships without deltas: {e:#}");
    }
    Ok(())
}

fn default_key_path(project: &Project) -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let config = std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from);
    config.join("nd/keys").join(format!("{}.key", project.id()))
}

fn signing_key(project: &Project, path: Option<&Path>) -> Result<SecretKey> {
    if let Some(path) = path {
        return read_key(path);
    }
    if let Ok(text) = std::env::var("ND_UPDATE_SIGNING_KEY")
        && !text.is_empty()
    {
        return SecretKey::parse(&text).context("ND_UPDATE_SIGNING_KEY");
    }
    let default = default_key_path(project);
    if default.exists() {
        return read_key(&default);
    }
    bail!("no signing key: pass --key, set ND_UPDATE_SIGNING_KEY, or run nd release keygen")
}

fn read_key(path: &Path) -> Result<SecretKey> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    SecretKey::parse(&text).with_context(|| format!("{}", path.display()))
}

fn keygen(args: KeygenArgs) -> Result<()> {
    let project = Project::load(&args.dir)?;
    let out = args.out.unwrap_or_else(|| default_key_path(&project));
    if out.exists() {
        bail!("{} exists; refusing to replace a signing key", out.display());
    }
    let key = SecretKey::generate();
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_private(&out, &key.to_base64())?;
    eprintln!("private key: {} (keep it out of the repo; CI reads ND_UPDATE_SIGNING_KEY)", out.display());
    println!("public_key = \"{}\"", key.public_key().to_base64());
    Ok(())
}

fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).with_context(|| format!("creating {}", path.display()))?;
    writeln!(file, "{text}")?;
    Ok(())
}

fn serve(args: ServeArgs) -> Result<()> {
    let root = match args.root {
        Some(root) => root,
        None => feed_dir(&Project::load(&args.dir)?),
    };
    let server = tiny_http::Server::http((args.host.as_str(), args.port))
        .map_err(|e| anyhow::anyhow!("binding {}:{}: {e}", args.host, args.port))?;
    let addr = server.server_addr().to_ip().context("listening address")?;
    println!("serving {} at http://{addr}", root.display());
    serve_requests(&server, &root);
    Ok(())
}

/// Plain static GETs of top-level files, one log line per request on stdout.
pub fn serve_requests(server: &tiny_http::Server, root: &Path) {
    use std::io::Write as _;
    for request in server.incoming_requests() {
        let path = request.url().split('?').next().unwrap_or("/").trim_start_matches('/').to_owned();
        let plain = !path.is_empty() && !path.contains(['/', '\\']) && path != "..";
        let file = plain.then(|| root.join(&path)).filter(|p| p.is_file());
        let (status, result) = match (request.method(), file) {
            (tiny_http::Method::Get, Some(file)) => match std::fs::File::open(&file) {
                Ok(f) => (200, request.respond(tiny_http::Response::from_file(f))),
                Err(_) => (500, request.respond(tiny_http::Response::empty(500))),
            },
            (tiny_http::Method::Get, None) => (404, request.respond(tiny_http::Response::empty(404))),
            _ => (405, request.respond(tiny_http::Response::empty(405))),
        };
        let _ = result;
        println!("GET /{path} {status}");
        let _ = std::io::stdout().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_are_scoped_per_os() {
        assert_eq!(velopack_channel(Os::Mac, "stable"), "mac-stable");
        assert_eq!(velopack_channel(Os::Windows, "beta"), "win-beta");
        assert_eq!(velopack_channel(Os::Linux, "stable"), "linux-stable");
    }

    #[test]
    fn signed_feed_verifies_and_a_tampered_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let json = br#"{"Assets":[]}"#;
        std::fs::write(dir.path().join("releases.mac-stable.json"), json).unwrap();
        let key = SecretKey::generate();
        sign_feed(dir.path(), "mac-stable", &key).unwrap();
        let sig = std::fs::read_to_string(dir.path().join("releases.mac-stable.json.sig")).unwrap();
        assert!(key.public_key().verify("mac-stable", json, &sig).is_ok());
        assert!(key.public_key().verify("mac-stable", br#"{"Assets":[1]}"#, &sig).is_err());
    }

    #[test]
    fn serve_answers_files_and_refuses_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("releases.mac-stable.json"), "{}").unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let root = dir.path().to_owned();
        let server = std::sync::Arc::new(server);
        let serving = server.clone();
        let handle = std::thread::spawn(move || serve_requests(&serving, &root));
        let get = |path: &str| match ureq::get(format!("http://127.0.0.1:{port}{path}")).call() {
            Ok(mut r) => (200, r.body_mut().read_to_string().unwrap()),
            Err(ureq::Error::StatusCode(code)) => (code, String::new()),
            Err(e) => panic!("{e}"),
        };
        assert_eq!(get("/releases.mac-stable.json?localVersion=0.1.0"), (200, "{}".into()));
        assert_eq!(get("/missing.json").0, 404);
        assert_eq!(get("/../Cargo.toml").0, 404);
        server.unblock();
        handle.join().unwrap();
    }
}
