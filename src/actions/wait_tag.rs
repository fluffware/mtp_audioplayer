use crate::actions::action::{Action, ActionFuture};
use crate::actions::tag_dispatcher::TagDispatcher;
use std::sync::Arc;
use std::num::ParseFloatError;

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

/// Parse float and map true and false to 1 and 0 respectively
fn parse_number(num_str: &str) -> Result<f64, ParseFloatError>
{
    let lower = num_str.to_lowercase();
    match lower.as_str() {
        "true" => Ok(1.0),
        "false" => Ok(0.0),
        _ => num_str.parse::<f64>()
    }
}

impl TagCondition {
    pub fn check(&self, new_tag: &str, old_tag: Option<&String>) -> bool {
        use TagCondition::*;
        match self {
            Less(cmp) => parse_number(new_tag).map_or(false, |v| v < *cmp),
            LessEqual(cmp) => parse_number(new_tag).map_or(false, |v| v <= *cmp),
            Greater(cmp) => parse_number(new_tag).map_or(false, |v| v > *cmp),
            GreaterEqual(cmp) => parse_number(new_tag).map_or(false, |v| v >= *cmp),
            EqualNumber(cmp) => parse_number(new_tag).map_or(false, |v| v == *cmp),
            NotEqualNumber(cmp) => parse_number(new_tag).map_or(false, |v| v != *cmp),
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
                    if cond.check(value, prev.as_ref()) {
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
