use crate::actions::action::{Action, ActionFuture};
use crate::actions::tag_dispatcher::TagDispatcher;
use crate::clip_queue::ClipQueue;
use std::sync::Arc;
use tokio::time::Duration;

#[derive(Debug, Clone)]
pub enum TagCondition {
    Less(i32),
    LessEqual(i32),
    Greater(i32),
    GreaterEqual(i32),
    EqualInt(i32),
    EqualString(String),
    Changed(String),
}

impl TagCondition {
    pub fn check(&self, new_tag: &str, old_tag: Option<&String>) -> bool {
        use TagCondition::*;
        match self {
            Less(cmp) => new_tag.parse::<i32>().map_or(false, |v| v < *cmp),
            LessEqual(cmp) => new_tag.parse::<i32>().map_or(false, |v| v <= *cmp),
            Greater(cmp) => new_tag.parse::<i32>().map_or(false, |v| v > *cmp),
            GreaterEqual(cmp) => new_tag.parse::<i32>().map_or(false, |v| v >= *cmp),
            EqualInt(cmp) => new_tag.parse::<i32>().map_or(false, |v| v == *cmp),
            EqualString(cmp) => new_tag == cmp,
            Changed(cmp) => old_tag.map_or(false, |ref v| &cmp != v),
        }
    }
}
pub struct WaitTagAction<D>
where
    D: TagDispatcher + Send,
{
    tag: String,
    dispatcher: Arc<D>,
    condition: TagCondition,
}

impl<D> WaitTagAction<D>
where
    D: TagDispatcher + Send,
{
    pub fn new(tag: String, condition: TagCondition, dispatcher: Arc<D>) -> WaitTagAction<D> {
        WaitTagAction {
            tag,
            dispatcher,
            condition,
        }
    }
}

impl<D> Action for WaitTagAction<D>
where
    D: TagDispatcher + Send + Sync + 'static,
{
    fn run(&self) -> ActionFuture {
        let tag = self.tag.clone();
        let dispatcher = self.dispatcher.clone();
        let cond = self.condition.clone();
        Box::pin(async move {
            let mut prev = None;
            loop {
                let (value, wait) = dispatcher.wait_value(&tag)?;
                if let Some(value) = value.as_ref() {
                    if cond.check(value, None) {
                        return Ok(());
                    }
                }
                prev = value;
                let value = wait.await?;
                if cond.check(&value, prev.as_ref()) {
                    return Ok(());
                }
                prev = Some(value);
            }
        })
    }
}
