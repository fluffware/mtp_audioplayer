use crate::actions::action::{Action, DynResult};
use std::sync::{Mutex, Arc};
use tokio::task::JoinHandle;

struct State {
    name: String,
    action: Option<Arc<dyn Action + Send + Sync>>,
}

struct StateMachineMut {
    states: Vec<State>,
    task: Option<JoinHandle<DynResult<()>>>,
    active_state: usize,    
}

pub struct StateMachine {
    pub name: String,
    current: Mutex<StateMachineMut>
}

impl StateMachine {
    pub fn new(name: &str) -> Arc<StateMachine> {
        Arc::new(StateMachine {
            name: name.to_string(),
            current: Mutex::new(StateMachineMut{
                states: Vec::new(),
                task: None,
                active_state: 0,
            })
                
        })
    }
    
    pub fn add_state(self: &Arc<Self>, name: &str) -> usize
    {
        let mut current = self.current.lock().unwrap();
        current.states.push(State{name: name.to_string(), action: None});
        current.states.len() - 1
    }

    /// Lookup state by name
    pub fn find_state_index(self: &Arc<Self>, name: &str) -> Option<usize>
    {
        log::debug!("Looking for {} in {}", name, self.name);
        let current = self.current.lock().unwrap();
        for (index, state) in current.states.iter().enumerate() {
            log::debug!("CHecking {} in {}", state.name, self.name);
            if state.name == name {
                return Some(index);
            }
        }
        None
    }
    
    pub fn set_action(self: &Arc<Self>, state_index: usize, action: Arc<dyn Action + Send + Sync>)
    {
        let mut current = self.current.lock().unwrap();
        current.states[state_index].action = Some(action);
    }
    
    pub async fn stop(self: &Arc<Self>) {
        let task_opt =  {
            let mut current = self.current.lock().unwrap();
            current.task.take()
        };
        if let Some(task) = task_opt {
            task.abort();
        }
    }
    
    pub async fn goto(self: &Arc<Self>, state_index: usize) {
        self.stop().await;
        let mut current = self.current.lock().unwrap();
        if current.states.len() <= state_index {
            return;
        }
        current.active_state = state_index;
        if let Some(action) = &current.states[state_index].action {
            current.task = Some(tokio::spawn(action.run()));
        }
    }

}

#[cfg(test)]
use test_log::test;

#[cfg(test)]
#[test(tokio::test)]
pub async fn test_state_machine()
{
    use crate::actions::*;
    let mut sm = StateMachine::new("SM1");
    let mut seq1 = sequence::SequenceAction::new();
    seq1.add_owned_action(debug::DebugAction::new("State 1".to_string()));
    seq1.add_owned_action(wait::WaitAction::new(tokio::time::Duration::from_millis(100)));
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
