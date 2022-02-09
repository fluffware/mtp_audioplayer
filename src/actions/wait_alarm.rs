use crate::actions::action::{Action, ActionFuture};
use crate::actions::alarm_dispatcher::AlarmDispatcher;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum AlarmCondition {
    None,
    Any,
    Inc,
    Dec,
}

impl AlarmCondition {
    pub fn check(&self, new_count: u32, old_count: Option<u32>) -> bool {
        use AlarmCondition::*;
        //log::debug!("Alarm counts: {} {:?}", new_count, old_count);
        match self {
            None => new_count == 0,
            Any => new_count > 0,
            Inc => old_count.map_or(false, |v| new_count > v),
            Dec => old_count.map_or(false, |v| new_count < v),
        }
    }
}
pub struct WaitAlarmAction<D>
where
    D: AlarmDispatcher + Send,
{
    filter_name: String,
    dispatcher: Arc<D>,
    condition: AlarmCondition,
}

impl<D> WaitAlarmAction<D>
where
    D: AlarmDispatcher + Send,
{
    pub fn new(filter_name: String, condition: AlarmCondition, dispatcher: Arc<D>) -> WaitAlarmAction<D> {
        WaitAlarmAction {
            filter_name,
            dispatcher,
            condition,
        }
    }
}

impl<D> Action for WaitAlarmAction<D>
where
    D: AlarmDispatcher + Send + Sync + 'static,
{
    fn run(&self) -> ActionFuture {
        let filter_name = self.filter_name.clone();
        let dispatcher = self.dispatcher.clone();
        let cond = self.condition.clone();
        Box::pin(async move {
            let mut prev = None;
            loop {
                let (value, wait) = dispatcher.wait_alarm_filter(&filter_name)?;
                if cond.check(value, prev) {
                    return Ok(());
                }
                prev = Some(value);
                let value = wait.await?;
                if cond.check(value, prev) {
                    return Ok(());
                }
                prev = Some(value);
            }
        })
    }
}
