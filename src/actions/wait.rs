use crate::actions::action::{Action, ActionFuture};
use tokio::time;
use tokio::time::Duration;

pub struct WaitAction {
    timeout: Duration,
}

impl WaitAction {
    pub fn new(timeout: Duration) -> WaitAction {
        WaitAction { timeout }
    }
}

impl Action for WaitAction {
    fn run(&self) -> ActionFuture {
        let dur = self.timeout;
        Box::pin(async move {
            time::sleep(dur).await;
            Ok(())
        })
    }
}
