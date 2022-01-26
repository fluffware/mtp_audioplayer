use crate::open_pipe::connection::NotifyAlarm;
use chrono::{DateTime, NaiveDateTime, Utc};
use std::cmp::Ordering;

pub struct AlarmData {
    pub name: String,
    pub id: i32,
    pub alarm_class_name: String,
    pub alarm_class_symbol: String,
    pub event_text: String,
    pub instance_id: i32,
    pub priority: i32,
    pub state: i32,
    pub state_text: String,
    pub state_machine: i32,
    pub modification_time: DateTime<Utc>,
}

impl AlarmData {
    pub fn cmp_id(&self, other: &Self) -> Ordering {
        self.id
            .cmp(&other.id)
    }
}

impl From<NotifyAlarm> for AlarmData {
    fn from(notify: NotifyAlarm) -> AlarmData {
        let modification_time = match NaiveDateTime::parse_from_str(
            &notify.modification_time,
            "%Y-%m-%d %H:%M:%S%.f",
        ) {
            Ok(t) => DateTime::from_utc(t, Utc),
            Err(_) => Utc::now(),
        };
        AlarmData {
            name: notify.name,
            id: notify.id.parse().unwrap_or(0),
            alarm_class_name: notify.alarm_class_name,
            alarm_class_symbol: notify.alarm_class_symbol,
            event_text: notify.event_text,
            instance_id: notify.instance_id.parse().unwrap_or(0),
            priority: notify.priority.parse().unwrap_or(0),
            state: notify.state.parse().unwrap_or(0),
            state_text: notify.state_text,
            state_machine: notify.state_machine.parse().unwrap_or(0),
            modification_time,
        }
    }
}

impl From<AlarmData> for NotifyAlarm {
    fn from(alarm_data: AlarmData) -> NotifyAlarm {
        NotifyAlarm {
            name: alarm_data.name,
            id: alarm_data.id.to_string(),
            alarm_class_name: alarm_data.alarm_class_name,
            alarm_class_symbol: alarm_data.alarm_class_symbol,
            event_text: alarm_data.event_text,
            instance_id: alarm_data.instance_id.to_string(),
            priority: alarm_data.priority.to_string(),
            state: alarm_data.state.to_string(),
            state_text: alarm_data.state_text,
            state_machine: alarm_data.state_machine.to_string(),
            modification_time: alarm_data
                .modification_time
                .format("%Y-%m-%d %H:%M:%S%.f")
                .to_string(),
        }
    }
}

impl From<&AlarmData> for NotifyAlarm {
    fn from(alarm_data: &AlarmData) -> NotifyAlarm {
        NotifyAlarm {
            name: alarm_data.name.clone(),
            id: alarm_data.id.to_string(),
            alarm_class_name: alarm_data.alarm_class_name.clone(),
            alarm_class_symbol: alarm_data.alarm_class_symbol.clone(),
            event_text: alarm_data.event_text.clone(),
            instance_id: alarm_data.instance_id.to_string(),
            priority: alarm_data.priority.to_string(),
            state: alarm_data.state.to_string(),
            state_text: alarm_data.state_text.clone(),
            state_machine: alarm_data.state_machine.to_string(),
            modification_time: alarm_data
                .modification_time
                .format("%Y-%m-%d %H:%M:%S%.f")
                .to_string(),
        }
    }
}

/*
impl Into<NotifyAlarm> for AlarmData {
    fn into(self) -> NotifyAlarm {
    NotifyAlarm{
        name: self.name,
        id: self.id.to_string(),
        alarm_class_name: self.alarm_class_name,
        alarm_class_symbol: self.alarm_class_symbol,
        event_text: self.event_text,
        instance_id: self.instance_id.to_string(),
        priority: self.priority.to_string(),
        state: self.state.to_string(),
        state_text: self.state_text,
        state_machine: self.state_machine.to_string(),
        modification_time: "".to_string(),
    }
    }
}
*/
