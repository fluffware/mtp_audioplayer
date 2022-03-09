use super::alarm_functions::AlarmFunctions;
use crate::actions::action::{Action, ActionFuture};
use std::marker::PhantomData;

#[derive(Debug)]
pub enum AlarmOp {
    Ignore,
    Restore,
}

pub struct AlarmFunctionAction<S, T>
where
    S: AsRef<T>,
    T: AlarmFunctions,
{
    filter: String,
    alarm_functions: S,
    op: AlarmOp,
    phantom: PhantomData<T>,
}

impl<S, T> AlarmFunctionAction<S, T>
where
    S: AsRef<T>,
    T: AlarmFunctions,
{
    pub fn new(filter: String, alarm_functions: S, op: AlarmOp) -> AlarmFunctionAction<S, T> {
        AlarmFunctionAction {
            filter,
            alarm_functions,
            op,
            phantom: PhantomData,
        }
    }
}

impl<S, T> Action for AlarmFunctionAction<S, T>
where
    S: AsRef<T>,
    T: AlarmFunctions,
{
    fn run(&self) -> ActionFuture {
        match self.op {
            AlarmOp::Ignore => {
                self.alarm_functions
                    .as_ref()
                    .ignore_matched_alarms(&self.filter, false);
            }
            AlarmOp::Restore => {
                self.alarm_functions
                    .as_ref()
                    .restore_ignored_alarms(&self.filter);
            }
        }
        Box::pin(std::future::ready(Ok(())))
    }
}
