use clap::{ArgMatches, Command};
use log::info;

pub fn add_args<'a>(app_args: Command<'a>) -> Command<'a> {
    app_args
}

pub fn start(_args: &ArgMatches) {
    tracing_subscriber::fmt::init();
    info!("Server starting");
}

pub fn ready() {
    info!("Server ready");
}

pub fn exiting() {
    info!("Server exiting");
}
