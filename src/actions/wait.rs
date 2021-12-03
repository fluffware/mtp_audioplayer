use tokio::time::Duration;
use tokio::time;
use crate::actions::action::{Action, AsyncAction};

pub struct WaitAction
{
     timeout: Duration,
}

impl WaitAction
{
    pub fn new(timeout: Duration) -> WaitAction
    {
	WaitAction{timeout}
    }
}

impl Action for WaitAction
{
    fn run(&self) -> AsyncAction
    {
	let dur = self.timeout;
	Box::pin(async move {
	    time::sleep(dur).await;
	    Ok(())
	})
    }
}
