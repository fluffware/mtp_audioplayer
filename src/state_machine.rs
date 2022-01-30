use crate::actions::action::{Action, ActionFuture};
use std::sync::Arc;

pub struct State {
    action: Arc<dyn Action + Send + Sync>,
}

pub struct StateMachine {
    running: Option<ActionFuture>,
    active_state: usize,
    states: Vec<State>,
}

impl StateMachine {
    pub fn new() -> StateMachine {
        StateMachine {
            running: None,
            active_state: 0,
            states: Vec::new(),
        }
    }
}
