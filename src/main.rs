// mackeys — macOS-style keymap semantics for Linux.
// Single binary. evdev -> uinput, clean-room rewrite of the proven core
// mechanisms of the Toshy project (see docs/).

mod adapters;
mod daemon;
mod engine;
mod map;
mod setup;
mod types;

use anyhow::Result;

const SUBCOMMANDS: &str = "run (default) | setup | doctor | start | stop | status | devices | dump | uninstall";

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut args = std::env::args().skip(1);
    let sub = args.next().unwrap_or_else(|| "run".into());

    match sub.as_str() {
        "run" => daemon::run(),
        "setup" => setup::setup(),
        "doctor" => setup::doctor(),
        "start" | "stop" | "status" => setup::service_ctl(&sub),
        "devices" => setup::devices(),
        "dump" => map::dump(),
        "uninstall" => setup::uninstall(),
        "help" | "--help" | "-h" => {
            println!("mackeys — macOS-style keymap daemon for Linux");
            println!("usage: mackeys <subcommand>   subcommands: {SUBCOMMANDS}");
            Ok(())
        }
        other => {
            eprintln!("mackeys: unknown subcommand: '{other}'");
            eprintln!("subcommands: {SUBCOMMANDS}");
            std::process::exit(2);
            #[allow(unreachable_code)]
            {
                Ok(())
            }
        }
    }
}
