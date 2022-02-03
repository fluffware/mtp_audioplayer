use std::future::Future;
use std::pin::Pin;

#[derive(Debug)]
pub enum Error {
    AlarmFilterNotFound,
    DispatcherNotAvailable,
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use Error::*;
        f.write_str(match self {
            AlarmFilterNotFound => "Alarm filter not found",
            DispatcherNotAvailable => "Alarm dispatcher not available",
        })
    }
}
pub type AlarmDispatched = Pin<Box<dyn Future<Output = Result<u32, Error>> + Send>>;

pub trait AlarmDispatcher {
    /// Get the current number of alarms matching a filter and a future that will be ready when the count changes.
    /// The future may be ready even if the value doesn't change
    fn wait_alarm_filter(&self, filter: &str) -> Result<(u32, AlarmDispatched), Error>;

    /// Get the current number of alarms matching a filter
    fn get_filter_count(&self, filter: &str) -> Result<u32, Error>;
}
