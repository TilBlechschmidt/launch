#[cfg(feature = "client")]
mod client;
mod server;
mod shared;

use clap::Parser;
use shared::*;

#[derive(Parser)]
enum Command {
    Server,

    #[cfg(feature = "client")]
    #[command(flatten)]
    Client(client::Command),
}

fn main() -> anyhow::Result<()> {
    let command = Command::parse();

    match command {
        Command::Server => server::run(),
        #[cfg(feature = "client")]
        Command::Client(cmd) => client::run(cmd),
    }
}
