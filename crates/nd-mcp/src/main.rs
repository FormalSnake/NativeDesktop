use anyhow::{Result, bail};
use clap::Parser;
use nd_mcp::driver::{Launcher, SocketLauncher};
use nd_mcp::server::NdMcp;
use nd_mcp::session::Session;
use rmcp::ServiceExt;

#[derive(Parser)]
#[command(name = "nd-mcp", version, about = "MCP bridge for a NativeDesktop app")]
struct Args {
    /// Attach to a host somebody else launched.
    #[arg(long, env = "ND_AUTOMATION_SOCKET")]
    socket: Option<String>,
    /// Drive the in-process stand-in tree of examples/hello instead of a real app.
    #[cfg(feature = "stub")]
    #[arg(long)]
    stub: bool,
}

fn launcher(args: &Args) -> Result<Box<dyn Launcher>> {
    if let Some(socket) = &args.socket {
        return Ok(Box::new(SocketLauncher { socket: socket.clone() }));
    }
    #[cfg(feature = "stub")]
    if args.stub {
        return Ok(Box::new(nd_mcp::stub::HelloStub));
    }
    bail!("no app to drive: set ND_AUTOMATION_SOCKET, or pass --stub for the examples/hello stand-in")
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let session = Session::spawn(launcher(&args)?);
    let service = NdMcp::new(session).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
