use std::sync::Arc;
use crate::actions::action::{Action, AsyncAction};

pub struct SequenceAction
{
    actions: Vec<Arc<dyn Action>>
}

impl SequenceAction
{
    pub fn new() -> SequenceAction
    {
	SequenceAction{actions: Vec::new()}
    }

    pub fn add_arc_action(&mut self, action: Arc<dyn Action>) {
	self.actions.push(action);
    }
    pub fn add_owned_action<T>(&mut self, action: T)
	where T: Action + 'static
    {
	self.actions.push(Arc::new(action));
    }
}

impl Action for SequenceAction
{
    fn run(&self) -> AsyncAction
    {
	let actions = self.actions.clone(); // Make a snapshot of the actions
	Box::pin(async move {
	    for a in actions {
		a.run().await?;
	    }
	    Ok(())
	})
    }
}

impl Default for SequenceAction
{
    fn default() ->SequenceAction
    {
	SequenceAction::new()
    }
}
