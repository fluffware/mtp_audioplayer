use std::sync::Arc;
use std::num::NonZeroU32;
use crate::actions::action::{Action, AsyncAction};

pub struct RepeatAction
{
    action: Arc<dyn Action>,
    count: Option<NonZeroU32>
}

impl RepeatAction
{
    pub fn new(action: Arc<dyn Action>, count: Option<NonZeroU32>)
	       -> RepeatAction
    {
	RepeatAction{action, count}
    }

}

impl Action for RepeatAction
{
    fn run(&self) -> AsyncAction
    {
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
