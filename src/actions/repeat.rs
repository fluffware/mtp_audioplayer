use crate::actions::action::{Action, ActionFuture};
use std::num::NonZeroU32;
use std::sync::Arc;

pub struct RepeatAction {
    action: Arc<dyn Action + Send + Sync>,
    count: Option<NonZeroU32>,
}

impl RepeatAction {
    pub fn new(action: Arc<dyn Action + Send + Sync>, count: Option<NonZeroU32>) -> RepeatAction {
        RepeatAction { action, count }
    }
}

impl Action for RepeatAction {
    fn run(&self) -> ActionFuture {
        let action = self.action.clone();
        let count = self.count;
        Box::pin(async move {
            if let Some(count) = count {
                for _ in 0..u32::from(count) {
                    action.run().await?;
                }
            } else {
                loop {
                    action.run().await?;
                }
            }
            Ok(())
        })
    }
}
