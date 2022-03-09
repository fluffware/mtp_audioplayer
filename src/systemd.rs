use clap::{Arg, ArgMatches, Command};
use log::{info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use systemd::daemon::notify;
use systemd::daemon::{STATE_READY, STATE_STOPPING};
use systemd::journal::JournalLog;

static DAEMON: AtomicBool = AtomicBool::new(true);

pub fn add_args<'a>(app_args: Command<'a>) -> Command<'a> {
    app_args.arg(
        Arg::new("no_systemd")
            .long("no_systemd")
            .help("Don't expect to be run from systemd"),
    )
}

pub fn start(args: &ArgMatches) {
    DAEMON.store(!args.is_present("no_systemd"), Ordering::Relaxed);
    if DAEMON.load(Ordering::Relaxed) {
        if let Err(e) = JournalLog::init() {
            eprintln!("Failed to start logging: {}", e);
        }
        info!("Server starting");
    } else {
        tracing_subscriber::fmt::init();
        info!("Server starting");
    }
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

pub fn exiting() {
    if DAEMON.load(Ordering::Relaxed) {
        if let Err(e) = notify(false, [(STATE_STOPPING, "1")].iter()) {
            warn!("Failed to notify systemd of stopping: {}", e);
        }
    } else {
        info!("Server exiting");
    }
}
