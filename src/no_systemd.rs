use crate::flexi_setup::{add_flexi_args, setup_flexi_loggger};
use clap::{ArgMatches, Command};
use flexi_logger::LoggerHandle;
use log::info;

pub enum LogCtxt {
    None,                // No logging available
    Flexi(LoggerHandle), // Logging with flexi logger
}
pub fn add_args(app_args: Command) -> Command {
    add_flexi_args(app_args)
}

pub fn start(args: &ArgMatches) -> LogCtxt {
    let ctxt = match setup_flexi_loggger(args) {
        Ok(handle) => LogCtxt::Flexi(handle),
        Err(e) => {
            eprintln!("Failed to start logging: {}", e);
            LogCtxt::None
        }
    };
    info!("Server starting");
    ctxt
}

pub fn ready() {
    info!("Server ready");
}

pub fn exiting(_ctxt: LogCtxt) {
    info!("Server exiting");
}
