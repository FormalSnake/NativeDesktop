//! `cargo run -p discord-rpc [client_id]`: sets a presence for a few seconds. Works, quietly
//! idle, when Discord is not running.

use std::time::Duration;

use discord_rpc::{Activity, DiscordPlugin, DiscordStatus, DiscordStatusFeed, PresenceFeed, now_unix};
use nd_plugin::{PluginHost, Services};
use tokio::sync::watch;

#[tokio::main]
async fn main() {
    let client_id = std::env::args().nth(1).or_else(|| std::env::var("DISCORD_CLIENT_ID").ok()).unwrap_or_default();

    // docs:begin host
    let (activity_tx, activity_rx) = watch::channel(None);
    let services = Services::new();
    services.insert(PresenceFeed(activity_rx));

    let mut host = PluginHost::new("discord-rpc-demo", tokio::runtime::Handle::current(), services.clone());
    host.register(DiscordPlugin::new());

    // What the app's own config file would carry.
    let mut config: toml::Table =
        format!("[plugins.discord]\nclient_id = \"{client_id}\"\n").parse().expect("valid toml");
    let report = host.load(&mut config);
    for failure in &report.failed {
        eprintln!("plugin {} not started: {}", failure.plugin, failure.error);
    }
    // docs:end host

    let _ = activity_tx.send(Some(Activity {
        details: Some("Editing notes".into()),
        state: Some("nd demo".into()),
        started_at: Some(now_unix()),
        ..Activity::default()
    }));

    let status = services.get::<DiscordStatusFeed>().expect("the plugin publishes its status").0.clone();
    tokio::time::sleep(Duration::from_secs(2)).await;
    match *status.borrow() {
        DiscordStatus::Idle => println!("idle: pass a Discord application id to enable presence"),
        DiscordStatus::Searching => println!("Discord is not running; still looking, the app carries on"),
        DiscordStatus::Connected => println!("presence is live in Discord"),
    }
    host.shutdown();
}
