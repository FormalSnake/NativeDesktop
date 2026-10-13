use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;
use nd_mcp::server::NdMcp;
use nd_mcp::session::{Bridge, Target};
use nd_test::{LaunchOptions, example_bin};
use rmcp::ServiceExt;

#[derive(Parser)]
#[command(name = "nd-mcp", version, about = "MCP bridge for a NativeDesktop app")]
struct Args {
    /// Attach to a host somebody else launched.
    #[arg(long, env = "ND_AUTOMATION_SOCKET")]
    socket: Option<PathBuf>,
    /// Launch this binary with NATIVE_AUTOMATION=1.
    #[arg(long, env = "ND_MCP_BIN", conflicts_with_all = ["socket", "example"])]
    bin: Option<PathBuf>,
    /// Launch a workspace binary by package name (`hello`), from `target/debug` or `$ND_<NAME>_BIN`.
    #[arg(long, conflicts_with = "socket")]
    example: Option<String>,
    /// Arguments passed to the launched binary.
    #[arg(last = true)]
    app_args: Vec<String>,
}

fn target(args: Args) -> Result<Target> {
    if let Some(socket) = args.socket {
        return Ok(Target::Socket(socket));
    }
    let bin = match (args.bin, args.example) {
        (Some(bin), _) => bin,
        (None, Some(name)) => example_bin(&name),
        (None, None) => bail!("no app to drive: set ND_AUTOMATION_SOCKET, or pass --bin <path> or --example <name>"),
    };
    let mut options = LaunchOptions::new(bin);
    options.args = args.app_args;
    Ok(Target::Launch(options))
}

#[tokio::main]
async fn main() -> Result<()> {
    let bridge = Bridge::spawn(target(Args::parse())?);
    let service = NdMcp::new(bridge).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
