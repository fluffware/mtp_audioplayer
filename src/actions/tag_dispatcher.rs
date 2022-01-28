use std::future::Future;
use std::pin::Pin;

#[derive(Debug)]
pub enum Error {
    TagNotFound,
    DispatcherNotAvailable,
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use Error::*;
        f.write_str(match self {
            TagNotFound => "Can't subscribe to tag, not found",
            DispatcherNotAvailable => "Tag dispatcher not available",
        })
    }
}
pub type TagDispatched = Pin<Box<dyn Future<Output = Result<String, Error>> + Send>>;

pub trait TagDispatcher {
    /// Get the current value of a tag and a future that will be ready when the value changes.
    /// The future may be ready even if the value doesn't change
    fn wait_value(&self, tag: &str) -> Result<(Option<String>, TagDispatched), Error>;

    /// Get the current value of a tag. None is returned if the value is unknown
    fn get_value(&self, tag: &str) -> Option<String>;
}
