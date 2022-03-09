use crate::actions::action::{Action, ActionFuture};
use crate::state_machine::StateMachine;

use std::sync::Weak;

pub struct GotoAction {
    state_index: usize,
    state_machine: Weak<StateMachine>,
}

impl GotoAction {
    pub fn new(state_index: usize, state_machine: Weak<StateMachine>) -> GotoAction {
        GotoAction {
            state_index,
            state_machine,
        }
    }
}

impl Action for GotoAction {
    fn run(&self) -> ActionFuture {
        let state_machine = Weak::upgrade(&self.state_machine);
        let state_index = self.state_index;
        Box::pin(async move {
            if let Some(state_machine) = state_machine {
                state_machine.goto(state_index).await;
            }
            Ok(())
        })
    }
}
