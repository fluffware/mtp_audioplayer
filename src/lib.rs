pub mod actions;
pub mod alarm_filter;
pub mod app_config;
pub mod clip_player;
pub mod clip_queue;
pub mod open_pipe;
pub mod priority_scheduler;
pub mod read_config;
pub mod sample_buffer;
pub mod state_machine;
pub mod util;

#[cfg(feature = "systemd")]
mod systemd;

#[cfg(not(feature = "systemd"))]
mod no_systemd;

pub mod logging {
    #[cfg(not(feature = "systemd"))]
    pub use crate::no_systemd::init_logging as init;
    #[cfg(feature = "systemd")]
    pub use crate::systemd::init_logging as init;
}

pub mod daemon {
    #[cfg(not(feature = "systemd"))]
    pub use crate::no_systemd::{exiting, ready, starting};
    #[cfg(feature = "systemd")]
    pub use crate::systemd::{exiting, ready, starting};
}
