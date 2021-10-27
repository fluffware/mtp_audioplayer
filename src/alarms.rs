use std::ops::Range;

pub enum AlarmState
{
    Normal = 0,
    Raised = 1,
    RaisedCleared = 2,
    RaisedAcknowledged = 5,
    RaisedAcknowledgedCleared = 6,
    RaisedClearedAcknowledged = 7,
    Removed = 8,
}

pub enum AlarmEvent
{
    FirstRaised, // An alarm is raised when no other alarms are raised
    Raised, // An alarm is raised
    Cleared, // An alarm is cleared
    LastCleared, //The last raised alarm is cleared
    Acked, // An alarm is acknowledged
    LastAcked, //The last unacknowledged alarm is cleared
}

pub struct AlarmFilter
{
    pub class: Vec<u32>,
    pub id: Vec<u32>,
    pub priority: Range<u32>,
}
    
struct AlarmInstance
{
    id: u32,
    instance: u32,
    alarm_class: u32,
}

