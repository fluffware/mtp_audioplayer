use std::future::Future;
use std::error::Error;
use std:: pin::Pin;

type DynResult<T> = Result<T, Box<dyn Error + Send +Sync>>;
pub type AsyncAction = Pin<Box<dyn Future<Output = DynResult<()>>>>;
pub trait Action
{
    fn run(&self) -> AsyncAction;
}
