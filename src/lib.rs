pub mod open_pipe;
pub mod clip_player;
pub mod read_config;
pub mod app_config;
pub mod alarm_filter;
pub mod priority_scheduler;
pub mod clip_queue;
pub mod actions;
pub mod state_machine;
pub mod sample_buffer;

#[cfg(feature = "systemd")]
mod systemd;

#[cfg(not(feature = "systemd"))]
mod no_systemd;


pub mod logging {
    #[cfg(feature = "systemd")]
    pub use crate::systemd::init_logging as init;
    #[cfg(not(feature = "systemd"))]
    pub use crate::no_systemd::init_logging as init;
}

pub mod daemon {
    #[cfg(feature = "systemd")]
    pub use crate::systemd::{starting, ready, exiting};
    #[cfg(not(feature = "systemd"))]
    pub use crate::no_systemd::{starting, ready, exiting};
}

    
