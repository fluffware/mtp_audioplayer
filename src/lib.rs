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

pub mod daemon {
    #[cfg(not(feature = "systemd"))]
    pub use crate::no_systemd::{add_args, exiting, ready, start};
    #[cfg(feature = "systemd")]
    pub use crate::systemd::{add_args, exiting, ready, start};
}
