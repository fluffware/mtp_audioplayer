use futures::future::join_all;
use std::sync::Arc;

use crate::actions::action::{Action, ActionFuture};

pub struct ParallelAction {
    actions: Vec<Arc<dyn Action + Send + Sync>>,
}

impl ParallelAction {
    pub fn new() -> ParallelAction {
        ParallelAction {
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

impl Action for ParallelAction {
    fn run(&self) -> ActionFuture {
        let actions = self.actions.clone(); // Make a snapshot of the actions

        let mut action_futures = Vec::new();
        Box::pin(async move {
            for a in actions {
                action_futures.push(a.run())
            }
            join_all(action_futures).await;
            Ok(())
        })
    }
}

impl Default for ParallelAction {
    fn default() -> ParallelAction {
        Self::new()
    }
}
