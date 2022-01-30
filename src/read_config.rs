use crate::actions::wait_tag::TagCondition;
use crate::alarm_filter;
use roxmltree::{Document, Node, TextPos};
use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::num::NonZeroU32;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug)]
pub enum ConfigErrorKind {
    WrongNamespace,
    UnexpectedElement,
    UnexpectedText,
    UnexpectedAttribute,
    MissingAttribute(String),
    ExclusiveAttributes(&'static [&'static str]),
    ParseAttribute(String, Box<dyn Error + Send + Sync>),
    ParseFilter(Box<dyn Error + Send + Sync>),
}

use ConfigErrorKind::*;

impl std::fmt::Display for ConfigErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            WrongNamespace => write!(f, "Incorrect namespace for element"),
            UnexpectedElement => write!(f, "Unexpected element"),
            UnexpectedText => write!(f, "Unexpected non-whitespace text"),
            UnexpectedAttribute => write!(f, "Unexpected attribute"),
            MissingAttribute(name) => write!(f, "Missing attribute '{}'", name),
	    ExclusiveAttributes(attrs) => write!(f, "Exactly one of the attributes '{}' is required", attrs.join("', '")),
            ParseAttribute(name, err) => write!(f, "Failed to parse attribute '{}': {}", name, err),
            ParseFilter(err) => write!(f, "Failed to parse alarm filter: {}", err),
        }
    }
}

#[derive(Debug)]
pub struct ConfigError {
    kind: ConfigErrorKind,
    pos: TextPos,
}

impl ConfigError {
    pub fn new(node: &Node, kind: ConfigErrorKind) -> ConfigError {
        ConfigError {
            pos: node.document().text_pos_at(node.range().start),
            kind,
        }
    }
}
impl std::error::Error for ConfigError {}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "{}:{}: {}", self.pos.row, self.pos.col, self.kind)
    }
}

#[derive(Debug)]
pub enum ClipType {
    File(String),
    Sine {
        amplitude: f64,
        frequency: f64,
        duration: Duration,
    },
}

#[derive(Debug)]
pub enum ActionType {
    Sequence(Vec<ActionType>),
    Parallel(Vec<ActionType>),
    Play {
        priority: i32,
        timeout: Option<Duration>,
        sound: String,
    },
    Wait(Duration),
    WaitTag {
        tag_name: String,
        condition: TagCondition,
    },
    Debug(String),
    Reference(String),
    // No count means forever.
    Repeat {
        count: Option<NonZeroU32>,
        action: Box<ActionType>,
    },
    AlarmRestart,
}

#[derive(Debug)]
pub struct StateConfig {
    pub id: String,
    pub action: ActionType,
}

#[derive(Debug)]
pub struct StateMachineConfig {
    pub id: String,
    pub states: Vec<StateConfig>,
}

#[derive(Debug)]
pub struct PlayerConfig {
    pub bind: String,
    pub playback_device: String,
    pub rate: u32,
    pub channels: u8,
    pub clip_root: String,
    pub clips: HashMap<String, ClipType>,
    pub named_actions: Vec<(String, ActionType)>,
    pub tags: Vec<String>,
    pub named_alarm_filters: HashMap<String, alarm_filter::BoolOp>,
    pub state_machines: Vec<StateMachineConfig>,
}

const NS: &str = "http://www.elektro-kapsel.se/audioplayer/v1";
type DynResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn required_attribute<T>(node: &Node, name: &str) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let attr_str = node
        .attribute(name)
        .ok_or_else(|| ConfigError::new(&node, MissingAttribute(name.to_string())))?;
    let res: Result<T, <T as FromStr>::Err> = attr_str.parse();
    res.map_err(|e| ConfigError::new(&node, ParseAttribute(name.to_string(), e.into())))
}

fn optional_attribute<T>(node: &Node, name: &str) -> Result<Option<T>, ConfigError>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let attr_str = match node.attribute(name) {
        Some(v) => v,
        None => return Ok(None),
    };
    let res: Result<T, <T as FromStr>::Err> = attr_str.parse();
    match res {
        Ok(res) => Ok(Some(res)),
        Err(e) => Err(ConfigError::new(
            &node,
            ParseAttribute(name.to_string(), e.into()),
        )),
    }
}

/// Get text content of an element with no element children.
/// Non-text nodes are ignored
fn text_content(node: &Node) -> Result<String, ConfigError> {
    let mut content = String::new();
    for child in node.children() {
        if child.is_element() {
            return Err(ConfigError::new(&child, UnexpectedElement));
        }
        if child.is_text() {
            content.push_str(&child.text().unwrap());
        }
    }
    Ok(content)
}

fn parse_duration(time_str: &str) -> DynResult<Duration> {
    let time_str = time_str.trim();
    let (value_str, unit_str) = time_str.split_at(time_str.len() - 1);
    let value: f64 = value_str.trim().parse()?;
    if value < 0.0 {
        return Err("Negative duration not allowed".into());
    }
    let scale = match unit_str {
        "s" => 1.0,
        "m" => 60.0,
        "h" => 60.0 * 60.0,
        u => return Err(format!("Unknown time unit '{}'", u).into()),
    };
    Ok(Duration::from_secs_f64(value * scale))
}

fn parse_bind(node: &Node) -> Result<String, ConfigError> {
    text_content(node)
}

fn parse_file_clip(node: &Node) -> Result<(String, ClipType), ConfigError> {
    let id: String = required_attribute(&node, "id")?;
    let file_name = text_content(&node)?;
    Ok((id, ClipType::File(file_name)))
}

fn parse_sine_clip(node: &Node) -> DynResult<(String, ClipType)> {
    let id = required_attribute(&node, "id")?;
    let amplitude = required_attribute(&node, "amplitude")?;
    let frequency = required_attribute(&node, "frequency")?;
    let dur_str: String = required_attribute(&node, "duration")?;
    let duration = parse_duration(&dur_str)
        .map_err(|e| ConfigError::new(&node, ParseAttribute("duration".to_string(), e.into())))?;
    Ok((
        id,
        ClipType::Sine {
            amplitude,
            frequency,
            duration,
        },
    ))
}

fn parse_clips(parent: &Node) -> DynResult<HashMap<String, ClipType>> {
    let mut clips = HashMap::new();
    for node in parent.children() {
        if check_element_ns(&node)? {
            match node.tag_name().name() {
                "file" => {
                    let (id, clip) = parse_file_clip(&node)?;
                    clips.insert(id, clip);
                }
                "sine" => {
                    let (id, clip) = parse_sine_clip(&node)?;
                    clips.insert(id, clip);
                }
                _ => return Err(ConfigError::new(&node, UnexpectedElement).into()),
            }
        }
    }
    Ok(clips)
}

fn parse_action(node: &Node) -> DynResult<ActionType> {
    let action;
    match node.tag_name().name() {
        "sequence" => {
            action = parse_sequence(node)?;
        }
        "parallel" => {
            action = parse_parallel(node)?;
        }
        "play" => {
            action = parse_play(node)?;
        }
        "wait" => {
            action = parse_wait(node)?;
        }
	"wait_tag" => {
	    action = parse_wait_tag(node)?;
	}
        "alarm_restart" => {
            action = ActionType::AlarmRestart;
        }
        "repeat" => {
            action = parse_repeat(node)?;
        }
        "action" => {
            action = parse_action_ref(node)?;
        },
	"debug" => {
	    action = parse_debug(node)?;
	}
        _ => return Err(ConfigError::new(&node, UnexpectedElement).into()),
    }
    Ok(action)
}

fn parse_play(node: &Node) -> DynResult<ActionType> {
    let priority = optional_attribute(&node, "priority")?.unwrap_or(0);

    let timeout_str: Option<String> = optional_attribute(&node, "timeout")?;
    let timeout = timeout_str.map_or(Ok(None), |s| Some(parse_duration(&s)).transpose())?;
    let sound = text_content(&node)?;
    Ok(ActionType::Play {
        priority,
        timeout,
        sound,
    })
}

fn parse_wait(node: &Node) -> DynResult<ActionType> {
    let time_str = text_content(&node)?;

    Ok(ActionType::Wait(parse_duration(&time_str)?))
}

const ConditionAttributes: &[&str] = &["eq","ne","lt","le", "gt", "ge", "eq_str", "ne_str"];
fn set_tag_condition(node: &Node, var: &mut Option<TagCondition>, cond: TagCondition)
    -> DynResult<()>
{
    if var.is_some() {
	return Err(ConfigError::new(node, ExclusiveAttributes(ConditionAttributes)).into())
    }
    *var = Some(cond);
    Ok(())
}
    
fn parse_wait_tag(node: &Node) -> DynResult<ActionType> {

    let mut condition = None;
    if let Some(v) = optional_attribute::<f64>(&node, "eq")? {
	set_tag_condition(node, &mut condition, TagCondition::EqualNumber(v))?;
    }
    if let Some(v) = optional_attribute::<f64>(&node, "ne")? {
	set_tag_condition(node, &mut condition, TagCondition::NotEqualNumber(v))?;
    }
     if let Some(v) = optional_attribute::<f64>(&node, "lt")? {
	set_tag_condition(node, &mut condition, TagCondition::Less(v))?;
     }
     if let Some(v) = optional_attribute::<f64>(&node, "le")? {
	set_tag_condition(node, &mut condition, TagCondition::LessEqual(v))?;
     }
     if let Some(v) = optional_attribute::<f64>(&node, "gt")? {
	set_tag_condition(node, &mut condition, TagCondition::Greater(v))?;
     }
     if let Some(v) = optional_attribute::<f64>(&node, "ge")? {
	set_tag_condition(node, &mut condition, TagCondition::GreaterEqual(v))?;
     }
     if let Some(v) = optional_attribute::<String>(&node, "eq_str")? {
	set_tag_condition(node, &mut condition, TagCondition::EqualString(v))?;
     }
     if let Some(v) = optional_attribute::<String>(&node, "ne_str")? {
	set_tag_condition(node, &mut condition, TagCondition::NotEqualString(v))?;
     }
    let condition = match condition {
	Some(cond) => cond,
	None => return Err(ConfigError::new(node, ExclusiveAttributes(ConditionAttributes)).into())
    };
	
    
    let tag_name = text_content(&node)?;

    Ok(ActionType::WaitTag{tag_name, condition})
}


fn parse_repeat(node: &Node) -> DynResult<ActionType> {
    let count = optional_attribute(&node, "count")?;
    let action = parse_sequence(&node)?;
    Ok(ActionType::Repeat {
        count,
        action: Box::new(action),
    })
}

fn parse_sequence(parent: &Node) -> DynResult<ActionType> {
    let mut actions = Vec::new();
    for child in parent.children() {
        if check_element_ns(&child)? {
            let action = parse_action(&child)?;
            actions.push(action);
        }
    }
    if actions.is_empty() {
        return Err("No action in sequence".into());
    }
    if actions.len() == 1 {
        Ok(actions.pop().unwrap())
    } else {
        Ok(ActionType::Sequence(actions))
    }
}

fn parse_parallel(parent: &Node) -> DynResult<ActionType> {
    let mut actions = Vec::new();
    for child in parent.children() {
        if check_element_ns(&child)? {
            let action = parse_action(&child)?;
            actions.push(action);
        }
    }
    if actions.is_empty() {
        return Err("No action in parallel".into());
    }
    if actions.len() == 1 {
        Ok(actions.pop().unwrap())
    } else {
        Ok(ActionType::Parallel(actions))
    }
}

fn parse_action_ref(node: &Node) -> DynResult<ActionType> {
    let action_ref: String = required_attribute(node, "use")?;
    Ok(ActionType::Reference(action_ref))
}

fn parse_debug(node: &Node) -> DynResult<ActionType> {
    let text = text_content(&node)?;
    Ok(ActionType::Debug(text))
}

fn parse_actions(parent: &Node, actions: &mut Vec<(String, ActionType)>) -> DynResult<()> {
    for child in parent.children() {
        if check_element_ns(&child)? {
            let id = required_attribute(&child, "id")?;
            let action = parse_action(&child)?;
            actions.push((id, action));
        }
    }
    Ok(())
}

fn parse_tag(node: &Node) -> DynResult<String> {
    Ok(text_content(&node)?)
}

fn parse_tags(parent: &Node) -> DynResult<Vec<String>> {
    let mut tags = Vec::new();
    for child in parent.children() {
        if check_element_ns(&child)? {
            match child.tag_name().name() {
                "tag" => {
                    let tag_name = parse_tag(&child)?;
                    tags.push(tag_name);
                }
                _ => return Err(ConfigError::new(&child, UnexpectedElement).into()),
            }
        }
    }
    Ok(tags)
}

#[derive(Debug, Copy, Clone)]
pub enum AlarmTriggerType {
    WhileAnyActive,
    WhileNoneActive,
    WhenRaised,
    WhenFirstRaised,
    WhenCleared,
    WhenLastCleared,
}

#[derive(Debug)]
pub struct AlarmTriggerConfig {
    pub trigger_type: AlarmTriggerType,
    pub filter_id: String,
    pub action: ActionType,
}

fn parse_alarms(
    parent: &Node,
    named_filters: &mut HashMap<String, alarm_filter::BoolOp>,
) -> DynResult<()> {
    let mut named_filters: HashMap<String, alarm_filter::BoolOp> = HashMap::new();
    for child in parent.children() {
        if check_element_ns(&child)? {
            match child.tag_name().name() {
                "filter" => {
                    let filter_id = required_attribute(&child, "id")?;
                    let filter_def = text_content(&child)?.trim().to_owned();
                    let op = match alarm_filter::parse_filter(&filter_def) {
                        Ok(op) => op,
                        Err(e) => {
                            let text_node = child.children().next();
                            let text_node_ref = match text_node {
                                Some(ref node) => node,
                                None => &child,
                            };
                            return Err(
                                ConfigError::new(text_node_ref, ParseFilter(e.to_string().into())).into()
                            );
                        }
                    };
                    named_filters.insert(filter_id, op);
                }
                _ => {
                    return Err(ConfigError::new(&child, UnexpectedElement).into());
                }
            }
        }
    }

    Ok(())
}

fn parse_state(parent: &Node) -> DynResult<StateConfig> {
    let id = required_attribute(&parent, "id")?;
    let mut actions = Vec::new();
    for child in parent.children() {
        if check_element_ns(&child)? {
            let action = parse_action(&child)?;
            actions.push(action);
        }
    }
    let action = if actions.len() == 1 {
        actions.pop().unwrap()
    } else {
        ActionType::Parallel(actions)
    };

    Ok(StateConfig { id, action })
}

fn parse_state_machine(parent: &Node) -> DynResult<StateMachineConfig> {
    let id = required_attribute(&parent, "id")?;
    let mut states = Vec::new();
    for child in parent.children() {
        if check_element_ns(&child)? {
            match child.tag_name().name() {
                "state" => {
                    let state = parse_state(&child)?;
                    states.push(state);
                }
                _ => return Err(ConfigError::new(&child, UnexpectedElement).into()),
            }
        }
    }
    Ok(StateMachineConfig { id, states })
}

fn parse_playback_device(node: &Node, player: &mut PlayerConfig) -> DynResult<()> {
    player.rate = required_attribute(&node, "rate")?;
    player.channels = required_attribute(&node, "channels")?;

    player.playback_device = text_content(&node)?;

    Ok(())
}

fn check_element_ns(node: &Node) -> Result<bool, ConfigError> {
    if node.is_element() {
        if node.tag_name().namespace() != Some(NS) {
            return Err(ConfigError::new(&node, WrongNamespace));
        }
        return Ok(true);
    } else if node.is_text() {
        if let Some(text) = node.text() {
            // Don't allow non-whitespace around elements
            if text.find(|c: char| !c.is_whitespace()).is_some() {
                return Err(ConfigError::new(&node, UnexpectedText));
            }
        }
    }
    Ok(false)
}
pub fn read_file<P: AsRef<Path>>(path: P) -> DynResult<PlayerConfig> {
    let mut file = File::open(path)?;
    let mut file_content = String::new();
    file.read_to_string(&mut file_content)?;
    let document = Document::parse(&file_content)?;

    let mut player = PlayerConfig {
        bind: "/tmp/siemens/automation/HmiRunTime".to_string(),
        playback_device: "".to_string(),
        rate: 44100,
        channels: 2,
        clip_root: String::new(),
        clips: HashMap::new(),
        named_actions: Vec::new(),
        tags: Vec::new(),
        named_alarm_filters: HashMap::new(),
        state_machines: Vec::new(),
    };

    let root = document.root_element();
    if !root.has_tag_name((NS, "audioplayer")) {
        return Err("The root node must be 'audioplayer'".into());
    }

    for node in root.children() {
        if check_element_ns(&node)? {
            match node.tag_name().name() {
                "bind" => {
                    player.bind = parse_bind(&node)?;
                }
                "playback_device" => {
                    parse_playback_device(&node, &mut player)?;
                }
                "clips" => {
                    player.clip_root = required_attribute(&node, "path")?;
                    player.clips = parse_clips(&node)?;
                }
                "actions" => {
                    parse_actions(&node, &mut player.named_actions)?;
                }
                "tags" => {
                    player.tags = parse_tags(&node)?;
                }
                "alarms" => {
                    parse_alarms(&node, &mut player.named_alarm_filters)?;
                }
                "state_machine" => {
                    player.state_machines.push(parse_state_machine(&node)?);
                }

                _ => return Err(ConfigError::new(&node, UnexpectedElement).into()),
            }
        }
    }
    Ok(player)
}

#[test]
fn test_parser() {
    let doc = r#"
<?xml version="1.0" encoding="UTF-8"?>
<audioplayer xmlns="http://www.elektro-kapsel.se/audioplayer/v1">
  <bind>/tmp/siemens/automation/HmiRunTime</bind>
  <playback_device rate="44100" channels="2">plughw:SoundBar</playback_device>
  <clips> 
    <file id="SoundAlarm">Alarm.wav</file>
    <file id="SoundInfo">Info.wav</file>
    <file id="SoundAccept">Knapp4.wav</file>
    <file id="SoundExe">Knapp2.wav</file>
    <file id="SoundInc">Knapp3.wav</file>
    <file id="SoundDec">Knapp4.wav</file>
  </clips>
  <tags>
    <toggle tag="SoundAlarm">
      <play>SoundAlarm</play>
    </toggle>
    <toggle tag="SoundInc">
      <play>SoundInfo</play>
    </toggle>
    <toggle tag="SoundExe">
      <play>SoundExe</play>
    </toggle>
    <toggle tag="SoundDec">
      <play>SoundInc</play>
    </toggle>
    <toggle tag="SoundDec">
      <play>SoundInc</play>
    </toggle>
    <toggle tag="AlarmRestart">
      <alarm_restart/>
    </toggle>
    <equals tag="AlarmProfile" value="0">
      <set_profile>Normal</set_profile>
    </equals>
    <equals tag="AlarmProfile" value="1">
      <set_profile>Operation</set_profile>
    </equals>
    <toggle tag="SequenceTest">
      <play>SoundExe</play>
      <wait>3s</wait>
      <repeat count="3">
        <play>SoundInc</play>
      </repeat>
    </toggle>
  </tags>
  <actions>
    <sequence id="AlarmRepeat">
      <repeat>
	<repeat count="20">
	  <play>SoundAlarm</play>
	  <wait>5s</wait>
	</repeat>
	<wait>6h</wait>
      </repeat>
    </sequence>
    <sequence id="AlarmDelayed">
      <repeat>
	<wait>5m</wait>
	<repeat count="20">
	  <play>SoundAlarm</play>
	  <wait>5s</wait>
	</repeat>
      </repeat>
    </sequence>
  <sequence id="AlarmOnce">
      <play>SoundAlarm</play>
    </sequence>
    <sequence id="InfoOnce">
      <play>SoundInfo</play>
    </sequence>
  </actions> 
  <alarms>
    <filter id="AlarmsUnacked">
      AlarmClassName = 'Larm' AND State = 'in' AND State = 'in out'
    </filter>
    <filter id="AlarmsRaised">
      AlarmClassName = 'Larm' AND State = 'in' AND State = 'in ack'
    </filter>
    <filter id="Warnings">
      AlarmClassName = 'Varning' AND State = 'in' AND State = 'in ack'
    </filter>
      
    <profile id="Normal">
      <while filter="AlarmUnacked">
	<action use="AlarmRepeat"/>
      </while>
      <while filter="AlarmsRaised">
	<action use="AlarmDelayed"/>
      </while>
	
      <when filter="Varning" event="raised">
	<action use="AlarmDelayed"/>
      </when>
    </profile>
  </alarms>
</audioplayer>
"#;
    read_file(str::as_bytes(doc)).unwrap();
}
