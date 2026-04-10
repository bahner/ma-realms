use std::{env, path::PathBuf};

use anyhow::{Result, anyhow};

mod daemon;

#[derive(Debug)]
struct Args {
    help: bool,
    daemon: bool,
    config: Option<String>,
    slug: Option<String>,
    listen: Option<String>,
    kubo_key_alias: Option<String>,
}

fn print_usage() {
    println!(
        "ma-agent usage:\n\
    ma-agent [options]\n\
\n\
Modes:\n\
    --daemon                    Run local ma-agentd API/admin daemon\n\
\n\
Daemon options:\n\
    --config <path>             Use explicit config file path (instead of XDG_CONFIG_HOME/ma/<slug>.yaml)\n\
    --slug <slug>               Config slug (default: agent, loads ~/.config/ma/<slug>.yaml)\n\
    --listen <host:port>        Daemon listen address (default from config or 127.0.0.1:5003)\n\
    --kubo-key-alias <alias>    Required Kubo key alias for world DID root publish\n\
\n\
Help:\n\
    -h, --help                  Show this help text\n"
    );
}

fn parse_args() -> Args {
    let mut help = false;
    let mut daemon = false;
    let mut config: Option<String> = None;
    let mut slug: Option<String> = None;
    let mut listen: Option<String> = None;
    let mut kubo_key_alias: Option<String> = None;

    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                help = true;
            }
            "--daemon" => {
                daemon = true;
            }
            "--slug" => {
                slug = iter.next();
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
            _ => {}
        }
    }

    Args {
        help,
        daemon,
        config,
        slug,
        listen,
        kubo_key_alias,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();

    if args.help {
        print_usage();
        return Ok(());
    }

    if args.daemon {
        return daemon::run_daemon(
            args.slug.clone(),
            args.listen.clone(),
            args.kubo_key_alias.clone(),
            args.config.clone().map(PathBuf::from),
        )
        .await;
    }

    Err(anyhow!(
        "non-daemon agent mode has been removed; use --daemon"
    ))
}
