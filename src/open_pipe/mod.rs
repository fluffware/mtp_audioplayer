#[cfg(target_os="linux")]
mod connection_unix;
#[cfg(target_os="linux")]
pub use connection_unix::ConnectionUnix as ConnectionLowLevel;

#[cfg(target_os="windows")]
mod connection_windows;
#[cfg(target_os="windows")]
pub use connection_windows::ConnectionWindows as ConnectionLowLevel;


pub mod connection;
pub mod tag_server;
pub mod alarm_server;
pub mod alarm_data;
