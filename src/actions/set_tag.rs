use super::tag_setter::TagSetter;
use crate::actions::action::{Action, ActionFuture};
use std::marker::PhantomData;

pub struct SetTagAction<S, T>
where
    S: AsRef<T>,
    T: TagSetter,
{
    tag_name: String,
    value: String,
    tag_setter: S,
    phantom: PhantomData<T>,
}

impl<S, T> SetTagAction<S, T>
where
    S: AsRef<T>,
    T: TagSetter,
{
    pub fn new(tag_name: String, value: String, tag_setter: S) -> SetTagAction<S, T> {
        SetTagAction {
            tag_name,
            value,
            tag_setter,
            phantom: PhantomData,
        }
    }
}

impl<S, T> Action for SetTagAction<S, T>
where
    S: AsRef<T>,
    T: TagSetter,
{
    fn run(&self) -> ActionFuture {
        self.tag_setter
            .as_ref()
            .async_set_tag(&self.tag_name, &self.value)
    }
}
