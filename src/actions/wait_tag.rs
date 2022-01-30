use crate::actions::action::{Action, ActionFuture};
use crate::actions::tag_dispatcher::TagDispatcher;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum TagCondition {
    Less(f64),
    LessEqual(f64),
    Greater(f64),
    GreaterEqual(f64),
    EqualNumber(f64),
    NotEqualNumber(f64),
    EqualString(String),
    NotEqualString(String),
    Changed,
}

impl TagCondition {
    pub fn check(&self, new_tag: &str, old_tag: Option<&String>) -> bool {
        use TagCondition::*;
        match self {
            Less(cmp) => new_tag.parse::<f64>().map_or(false, |v| v < *cmp),
            LessEqual(cmp) => new_tag.parse::<f64>().map_or(false, |v| v <= *cmp),
            Greater(cmp) => new_tag.parse::<f64>().map_or(false, |v| v > *cmp),
            GreaterEqual(cmp) => new_tag.parse::<f64>().map_or(false, |v| v >= *cmp),
            EqualNumber(cmp) => new_tag.parse::<f64>().map_or(false, |v| v == *cmp),
            NotEqualNumber(cmp) => new_tag.parse::<f64>().map_or(false, |v| v != *cmp),
            EqualString(cmp) => new_tag == cmp,
            NotEqualString(cmp) => new_tag != cmp,
            Changed => old_tag.map_or(false, |ref v| &new_tag != v),
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
