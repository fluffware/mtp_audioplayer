use log::info;
use systemd::daemon::notify;
use systemd::daemon::{STATE_READY, STATE_STOPPING};
use systemd::journal::JournalLog;

pub fn init_logging() {
    if let Err(e) = JournalLog::init() {
        eprintln!("Failed to start logging: {}", e);
    }
}

pub fn starting() {
    info!("Server starting");
}

pub fn ready() {
    notify(false, [(STATE_READY, "1")].iter());
}

pub fn exiting() {
    notify(false, [(STATE_STOPPING, "1")].iter());
}
