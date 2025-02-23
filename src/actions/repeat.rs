use crate::actions::action::{Action, ActionFuture};
use crate::event_limit::EventLimit;
use std::num::NonZeroU32;
use std::sync::Arc;

pub struct RepeatAction {
    action: Arc<dyn Action + Send + Sync>,
    count: Option<NonZeroU32>,
    repeat_limit: EventLimit,
}

impl RepeatAction {
    pub fn new(
        action: Arc<dyn Action + Send + Sync>,
        count: Option<NonZeroU32>,
        repeat_limit: EventLimit,
    ) -> RepeatAction {
        RepeatAction {
            action,
            count,
            repeat_limit,
        }
    }
}

impl Action for RepeatAction {
    fn run(&self) -> ActionFuture {
        let action = self.action.clone();
        let count = self.count;
        let mut limit = self.repeat_limit.clone();
        Box::pin(async move {
            if let Some(count) = count {
                for _ in 0..u32::from(count) {
                    action.run().await?;
                }
            } else {
                loop {
                    if !limit.count() {
                        return Err("Repetition too fast in repeat action".into());
                    }
                    action.run().await?;
                }
            }
            Ok(())
        })
    }
}
