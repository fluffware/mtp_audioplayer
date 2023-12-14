use crate::flexi_setup::{add_flexi_args, setup_flexi_loggger};
use clap::{Arg, ArgMatches, Command};
use flexi_logger::LoggerHandle;
use log::{info, warn, LevelFilter};
use std::sync::atomic::{AtomicBool, Ordering};
use systemd::daemon::notify;
use systemd::daemon::{STATE_READY, STATE_STOPPING};
use systemd::journal::JournalLog;

static DAEMON: AtomicBool = AtomicBool::new(true);

pub fn add_args<'a>(app_args: Command<'a>) -> Command<'a> {
    let app_args = app_args.arg(
        Arg::new("no_systemd")
            .long("no_systemd")
            .help("Don't expect to be run from systemd"),
    );
    let app_args = add_flexi_args(app_args);
    app_args
}

pub enum LogCtxt {
    None,                // No logging available
    Journal,             // Logging through journald
    Flexi(LoggerHandle), // Logging with flexi logger
}

pub fn start(args: &ArgMatches) -> LogCtxt {
    let ctxt;
    DAEMON.store(!args.is_present("no_systemd"), Ordering::Relaxed);
    if !DAEMON.load(Ordering::Relaxed) || args.is_present("log_file") {
        match setup_flexi_loggger(args) {
            Ok(handle) => {
                ctxt = LogCtxt::Flexi(handle);
            }
            Err(e) => {
                eprintln!("Failed to start logging: {}", e);
                ctxt = LogCtxt::None;
            }
        }
    } else {
        if let Err(e) = JournalLog::init() {
            eprintln!("Failed to start logging: {}", e);
            ctxt = LogCtxt::None;
        } else {
            ctxt = LogCtxt::Journal;
        }
        log::set_max_level(LevelFilter::Info);
    }
    info!("Server starting");
    ctxt
}

pub fn ready() {
    if DAEMON.load(Ordering::Relaxed) {
        if let Err(e) = notify(false, [(STATE_READY, "1")].iter()) {
            warn!("Failed to notify systemd of ready state: {}", e);
        }
    } else {
        info!("Server ready");
    }
}

pub fn exiting(_ctxt: LogCtxt) {
    if DAEMON.load(Ordering::Relaxed) {
        if let Err(e) = notify(false, [(STATE_STOPPING, "1")].iter()) {
            warn!("Failed to notify systemd of stopping: {}", e);
        }
    } else {
        info!("Server exiting");
    }
}
