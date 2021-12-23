use std::error::Error;
use std::future::Future;
use std::pin::Pin;

type DynResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
pub type ActionFuture = Pin<Box<dyn Future<Output = DynResult<()>> + Send>>;
pub trait Action {
    fn run(&self) -> ActionFuture;
}
