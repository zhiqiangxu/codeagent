mod config;

use clap::Parser;
use config::{AppConfig, CliArgs};

fn main() {
    let args = CliArgs::parse();
    let config = AppConfig::resolve(&args);
    println!(
        "CodeForge — model: {}, profile: {:?}",
        config.model, config.profile
    );
}
