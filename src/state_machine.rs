use crate::actions::action::Action;
use crate::event_limit::EventLimit;
use crate::util::error::DynResult;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

struct State {
    name: String,
    action: Option<Arc<dyn Action + Send + Sync>>,
}

struct StateMachineMut {
    states: Vec<State>,
    active_state: Option<usize>,
    restart: bool,            // Restart the state if it's already running
    change_limit: EventLimit, // Limit how fast the state may change
}

pub struct StateMachine {
    pub name: String,
    current_changed: Notify,
    current: Mutex<StateMachineMut>,
}

impl StateMachine {
    pub fn new(name: &str, change_limit: EventLimit) -> Arc<StateMachine> {
        Arc::new(StateMachine {
            name: name.to_string(),
            current_changed: Notify::new(),
            current: Mutex::new(StateMachineMut {
                states: Vec::new(),
                active_state: None,
                restart: false,
                change_limit,
            }),
        })
    }

    pub fn add_state(self: &Arc<Self>, name: &str) -> usize {
        let mut current = self.current.lock().unwrap();
        current.states.push(State {
            name: name.to_string(),
            action: None,
        });
        current.states.len() - 1
    }

    /// Lookup state by name
    pub fn find_state_index(self: &Arc<Self>, name: &str) -> Option<usize> {
        let current = self.current.lock().unwrap();
        for (index, state) in current.states.iter().enumerate() {
            if state.name == name {
                return Some(index);
            }
        }
        None
    }

    pub fn set_action(self: &Arc<Self>, state_index: usize, action: Arc<dyn Action + Send + Sync>) {
        let mut current = self.current.lock().unwrap();
        current.states[state_index].action = Some(action);
    }

    pub async fn stop(self: &Arc<Self>) {
        let mut current = self.current.lock().unwrap();
        current.active_state = None;
        self.current_changed.notify_one();
    }

    pub async fn run(self: &Arc<Self>) -> DynResult<()> {
        log::debug!("State machine {} running", self.name);
        {
            let mut current = self
                .current
                .lock()
                .map_err(|_| "Failed to lock state-machine")?;
            current.active_state = Some(0);
        }
        let mut running_action = None;
        let mut running_state = None;
        loop {
            {
                let mut current = self
                    .current
                    .lock()
                    .map_err(|_| "Failed to lock state-machine")?;
                if running_state != current.active_state || current.restart {
                    if !current.change_limit.count() {
                        return Err(format!(
                            "State changed too fast for state machine {}",
                            self.name
                        )
                        .into());
                    }
                    if let Some(active_state) = current.active_state {
                        if let Some(action) = &current.states[active_state].action {
                            running_action = Some(action.run());
                            /*
                            log::debug!(
                                "Running action for state {}",
                                &current.states[active_state].name
                            );*/
                        }
                    } else {
                        break;
                    }
                    running_state = current.active_state;
                    current.restart = false;
                }
            }
            if let Some(running) = &mut running_action {
                tokio::pin!(running);
                tokio::select! {
                        res = running => {
                            match res {
                                Ok(_) => {
                    running_action = None;
                                }
                                Err(e) => return Err(e),
                            }
                        }
                        _ = self.current_changed.notified() => {
                //log::debug!("Notified");


                        }
                    }
            } else {
                self.current_changed.notified().await;
            }
            //log::debug!("Loop done");
        }
        Ok(())
    }

    pub async fn goto(self: &Arc<Self>, state_index: usize) {
        let mut current = self.current.lock().unwrap();
        if current.states.len() <= state_index {
            return;
        }
        log::debug!("Goto {}", current.states[state_index].name);
        current.restart = true;
        current.active_state = Some(state_index);
        self.current_changed.notify_one();
    }
}

#[cfg(test)]
use test_log::test;

#[cfg(test)]
#[test(tokio::test)]
pub async fn test_state_machine() {
    use crate::actions::*;
    let mut sm = StateMachine::new("SM1");
    let mut seq1 = sequence::SequenceAction::new();
    seq1.add_owned_action(debug::DebugAction::new("State 1".to_string()));
    seq1.add_owned_action(wait::WaitAction::new(tokio::time::Duration::from_millis(
        100,
    )));
    seq1.add_owned_action(debug::DebugAction::new("State 1 again".to_string()));
    seq1.add_owned_action(goto::GotoAction::new(1, Arc::downgrade(&sm)));

    let state = sm.add_state("State 1");
    sm.set_action(state, Arc::new(seq1));

    let mut seq2 = sequence::SequenceAction::new();
    seq2.add_owned_action(debug::DebugAction::new("State 2".to_string()));
    seq2.add_owned_action(goto::GotoAction::new(0, Arc::downgrade(&sm)));
    let state = sm.add_state("State 2");
    sm.set_action(state, Arc::new(seq2));

    sm.goto(0).await;
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
}
