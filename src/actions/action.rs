use crate::util::error::DynResultFuture;

pub type ActionFuture = DynResultFuture<()>;

pub trait Action {
    fn run(&self) -> ActionFuture;
}
