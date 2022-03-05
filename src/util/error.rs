use std::error::Error;
use std::future::Future;
use std::pin::Pin;

pub type DynResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub type DynResultFuture<T> = Pin<Box<dyn Future<Output = DynResult<T>> + Send>>;
