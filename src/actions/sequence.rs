use crate::actions::action::{Action, ActionFuture};
use std::sync::Arc;

pub struct SequenceAction {
    actions: Vec<Arc<dyn Action + Send + Sync>>,
}

impl SequenceAction {
    pub fn new() -> SequenceAction {
        SequenceAction {
            actions: Vec::new(),
        }
    }

    pub fn add_arc_action(&mut self, action: Arc<dyn Action + Send + Sync>) {
        self.actions.push(action);
    }
    pub fn add_owned_action<T>(&mut self, action: T)
    where
        T: Action + Send + Sync + 'static,
    {
        self.actions.push(Arc::new(action));
    }
}

impl Action for SequenceAction {
    fn run(&self) -> ActionFuture {
        let actions = self.actions.clone(); // Make a snapshot of the actions
        Box::pin(async move {
            for a in actions {
		a.run().await?;
            }
            Ok(())
        })
    }
}

impl Default for SequenceAction {
    fn default() -> SequenceAction {
        SequenceAction::new()
    }
}
