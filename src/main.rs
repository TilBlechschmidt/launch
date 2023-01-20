mod client;
mod server;
mod shared;

use clap::Parser;
use shared::*;

#[derive(Parser)]
enum Command {
    Server,

    #[command(flatten)]
    Client(client::Command),
}

fn main() -> anyhow::Result<()> {
    let command = Command::parse();

    match command {
        Command::Server => server::run(),
        Command::Client(cmd) => client::run(cmd),
    }
}
