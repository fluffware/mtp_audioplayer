use crate::actions::action::{Action, ActionFuture};
use log::debug;

pub struct DebugAction {
    text: String,
}

impl DebugAction {
    pub fn new(text: String) -> DebugAction {
        DebugAction { text }
    }
}

impl Action for DebugAction {
    fn run(&self) -> ActionFuture {
        let text = self.text.clone();
        Box::pin(async move {
            debug!("{}", text);
            Ok(())
        })
    }
}
