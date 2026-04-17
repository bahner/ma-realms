use std::{env, path::PathBuf};

use anyhow::{anyhow, Result};

mod daemon;
mod mcp_stdio;

#[derive(Debug)]
struct Args {
    help: bool,
    daemon: bool,
    mcp: bool,
    config: Option<String>,
    listen: Option<String>,
    kubo_key_alias: Option<String>,
    agentd_url: Option<String>,
}

fn print_usage() {
    println!(
        "ma-agent usage:\n\
    ma-agent [options]\n\
\n\
Modes:\n\
    --daemon                    Run local ma-agentd API/admin daemon\n\
    --mcp                       Run MCP stdio server (proxying ma-agentd API)\n\
\n\
Daemon options:\n\
    --config <path>             Use explicit config file path (instead of XDG_CONFIG_HOME/ma/agentd.yaml)\n\
    --listen <host:port>        Daemon listen address (default from config or 127.0.0.1:5003)\n\
    --kubo-key-alias <alias>    Required Kubo key alias for world DID root publish\n\
\n\
MCP options:\n\
    --agentd-url <url>          ma-agentd API base URL (default: http://127.0.0.1:5003)\n\
\n\
Help:\n\
    -h, --help                  Show this help text\n"
    );
}

fn parse_args() -> Args {
    let mut help = false;
    let mut daemon = false;
    let mut mcp = false;
    let mut config: Option<String> = None;
    let mut listen: Option<String> = None;
    let mut kubo_key_alias: Option<String> = None;
    let mut agentd_url: Option<String> = None;

    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                help = true;
            }
            "--daemon" => {
                daemon = true;
            }
            "--mcp" => {
                mcp = true;
            }
            "--config" => {
                config = iter.next();
            }
            "--listen" => {
                listen = iter.next();
            }
            "--kubo-key-alias" => {
                kubo_key_alias = iter.next();
            }
            "--agentd-url" => {
                agentd_url = iter.next();
            }
            _ => {}
        }
    }

    Args {
        help,
        daemon,
        mcp,
        config,
        listen,
        kubo_key_alias,
        agentd_url,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();

    if args.help {
        print_usage();
        return Ok(());
    }

    if args.mcp && args.daemon {
        return Err(anyhow!("choose one mode: --daemon or --mcp"));
    }

    if args.mcp {
        return mcp_stdio::run_mcp(args.agentd_url.clone()).await;
    }

    // Default mode is daemon for ergonomic local operation.
    daemon::run_daemon(
        args.listen.clone(),
        args.kubo_key_alias.clone(),
        args.config.clone().map(PathBuf::from),
    )
    .await
}
