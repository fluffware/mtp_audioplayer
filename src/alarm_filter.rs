use crate::open_pipe::alarm_data::AlarmData;
use const_str::convert_ascii_case;
use nom::branch::alt;
use nom::bytes::complete::tag;
use nom::character::complete::alpha1;
use nom::character::complete::char;
use nom::character::complete::digit1;
use nom::character::complete::multispace0;
use nom::character::complete::none_of;
use nom::combinator::{eof, map};
use nom::multi::fold_many0;
use nom::sequence::{delimited, preceded, terminated, tuple};
use nom::IResult;
use num_enum::TryFromPrimitive;
use paste::paste;
use std::fmt::Debug;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

#[derive(PartialEq, Debug, Clone, Copy, TryFromPrimitive)]
#[repr(u32)]
pub enum AlarmState {
    Normal = 0,
    Raised = 1,
    RaisedCleared = 2,
    RaisedAcknowledged = 5,
    RaisedAcknowledgedCleared = 6,
    RaisedClearedAcknowledged = 7,
    Removed = 8,
}
use AlarmState::*;

// Define a string constant and a lowercase version with _LC appended to the name
macro_rules! makelc {
    ($n: ident, $str: expr) => {
        const $n: &str = $str;
        paste! {const [<$n _LC>]: &str = convert_ascii_case!(lower, $str);}
    };
}

const INCOMING_LC: &str = "incoming";
const OUTGOING_LC: &str = "outgoing";
const ACKNOWLEDGED_LC: &str = "acknowledged";
const INCOMING_SHORT_LC: &str = "in";
const OUTGOING_SHORT_LC: &str = "out";
const ACKNOWLEDGED_SHORT_LC: &str = "ack";

makelc!(NORMAL, "Normal");
makelc!(REMOVED, "Removed");
makelc!(RAISED, "Raised");
makelc!(RAISED_CLEARED, "RaisedCleared");
makelc!(RAISED_ACKNOWLEDGED, "RaisedAcknowledged");
makelc!(RAISED_ACKNOWLEDGED_CLEARED, "RaisedAcknowledgedCleared");
makelc!(RAISED_CLEARED_ACKNOWLEDGED, "RaisedClearedAcknowledged");

#[derive(Debug, PartialEq)]
pub struct AlarmStateError(String);

impl std::error::Error for AlarmStateError {}

impl Display for AlarmStateError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for AlarmState {
    type Err = AlarmStateError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(state_num) = s.parse::<u32>() {
            return AlarmState::try_from(state_num).map_err(|_| {
                AlarmStateError(format!("Integer {} is not a valid alarm state", state_num))
            });
        }
        let lc = s.to_lowercase();
        let list: Vec<&str> = lc.split(|c| c == ' ' || c == ',' || c == '/').collect();

        match list.as_slice() {
            [NORMAL_LC] => Ok(AlarmState::Normal),
            [INCOMING_LC] | [INCOMING_SHORT_LC] | [RAISED_LC] => Ok(Raised),

            [INCOMING_LC, OUTGOING_LC]
            | [INCOMING_SHORT_LC, OUTGOING_SHORT_LC]
            | [RAISED_CLEARED_LC] => Ok(RaisedCleared),

            [INCOMING_LC, ACKNOWLEDGED_LC]
            | [INCOMING_SHORT_LC, ACKNOWLEDGED_SHORT_LC]
            | [RAISED_ACKNOWLEDGED_LC] => Ok(RaisedAcknowledged),

            [INCOMING_LC, ACKNOWLEDGED_LC, OUTGOING_LC]
            | [INCOMING_SHORT_LC, ACKNOWLEDGED_SHORT_LC, OUTGOING_SHORT_LC]
            | [RAISED_ACKNOWLEDGED_CLEARED_LC] => Ok(RaisedAcknowledgedCleared),

            [INCOMING_LC, OUTGOING_LC, ACKNOWLEDGED_LC]
            | [INCOMING_SHORT_LC, OUTGOING_SHORT_LC, ACKNOWLEDGED_SHORT_LC]
            | [RAISED_CLEARED_ACKNOWLEDGED_LC] => Ok(RaisedClearedAcknowledged),

            [REMOVED_LC] => Ok(AlarmState::Removed),
            _ => {
                return Err(AlarmStateError(format!(
                    "String \"{}\" is not a valid alarm state",
                    s
                )))
            }
        }
    }
}

impl AlarmState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Normal => NORMAL,
            Raised => RAISED,
            RaisedCleared => RAISED_CLEARED,
            RaisedAcknowledged => RAISED_ACKNOWLEDGED,
            RaisedAcknowledgedCleared => RAISED_ACKNOWLEDGED_CLEARED,
            RaisedClearedAcknowledged => RAISED_CLEARED_ACKNOWLEDGED,
            Removed => REMOVED,
        }
    }
}

#[derive(Debug, Clone)]
pub enum StringCriterion {
    AlarmClassName,
    AlarmName,
}

impl StringCriterion {
    pub fn evaluate<'a>(&self, alarm: &'a AlarmData) -> &'a str {
        match self {
            StringCriterion::AlarmClassName => &alarm.alarm_class_name,
            StringCriterion::AlarmName => &alarm.name,
        }
    }

    pub fn as_str<'a>(&self) -> &'a str {
        match self {
            StringCriterion::AlarmClassName => &"AlarmClassName",
            StringCriterion::AlarmName => &"Name",
        }
    }
}

#[derive(Debug, Clone)]
pub enum IntCriterion {
    Id,
    InstanceId,
    Priority,
    AlarmState,
}

impl IntCriterion {
    pub fn evaluate(&self, alarm: &AlarmData) -> i32 {
        match self {
            IntCriterion::Id => alarm.id,
            IntCriterion::InstanceId => alarm.instance_id,
            IntCriterion::Priority => alarm.priority,
            IntCriterion::AlarmState => alarm.state,
        }
    }

    pub fn as_str<'a>(&self) -> &'a str {
        match self {
            IntCriterion::Id => &"ID",
            IntCriterion::InstanceId => &"InstanceID",
            IntCriterion::Priority => &"Priority",
            IntCriterion::AlarmState => &"State",
        }
    }
}

#[derive(Debug, Clone)]
pub enum BoolOp {
    Not(Box<BoolOp>),
    And(Box<BoolOp>, Box<BoolOp>),
    Or(Box<BoolOp>, Box<BoolOp>),
    StringEqual(StringCriterion, String),
    StateEqual(IntCriterion, AlarmState),
    IntEqual(IntCriterion, i32),
    IntLess(IntCriterion, i32),
    IntLessEqual(IntCriterion, i32),
}

use BoolOp::*;

impl BoolOp {
    pub fn evaluate(&self, alarm: &AlarmData) -> bool {
        match self {
            Not(arg) => !arg.evaluate(alarm),
            And(arg1, arg2) => arg1.evaluate(alarm) && arg2.evaluate(alarm),
            Or(arg1, arg2) => arg1.evaluate(alarm) || arg2.evaluate(alarm),
            StringEqual(criterion, value) => criterion.evaluate(alarm) == value,
            StateEqual(criterion, state) => criterion.evaluate(alarm) == *state as i32,
            IntEqual(criterion, value) => criterion.evaluate(alarm) == *value,
            IntLess(criterion, value) => criterion.evaluate(alarm) < *value,
            IntLessEqual(criterion, value) => criterion.evaluate(alarm) <= *value,
        }
    }
}

impl ToString for BoolOp {
    fn to_string(&self) -> String {
        match self {
            Not(arg) => "NOT (".to_owned() + &arg.to_string() + ")",
            And(arg1, arg2) => {
                "(".to_owned() + &arg1.to_string() + ") AND (" + &arg2.to_string() + ")"
            }
            Or(arg1, arg2) => {
                "(".to_owned() + &arg1.to_string() + ") OR (" + &arg2.to_string() + ")"
            }

            StringEqual(criterion, value) => criterion.as_str().to_owned() + " = '" + &value + "'",
            StateEqual(criterion, state) => {
                criterion.as_str().to_owned() + " = '" + &state.as_str() + "'"
            }
            IntEqual(criterion, value) => {
                criterion.as_str().to_owned() + " = " + &value.to_string()
            }
            IntLess(criterion, value) => criterion.as_str().to_owned() + " < " + &value.to_string(),
            IntLessEqual(criterion, value) => {
                criterion.as_str().to_owned() + " <= " + &value.to_string()
            }
        }
    }
}

#[derive(Debug)]
pub enum FilterErrorKind {
    InvalidCriterionName(String),
    IllegalCheckOperation(String),
    InvalidState(String),
    Nom(nom::error::ErrorKind),
    Error(Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl Display for FilterErrorKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            FilterErrorKind::InvalidCriterionName(name) => {
                write!(f, "Name of filter criterion not recognized: {}", name)
            }
            FilterErrorKind::IllegalCheckOperation(op) => {
                write!(f, "Illegal comparison operator: {}", op)
            }
            FilterErrorKind::InvalidState(state) => {
                write!(f, "Invalid state descriptor: {}", state)
            }
            FilterErrorKind::Nom(err) => {
                write!(f, "{}", err.description())
            }
            FilterErrorKind::Error(err) => {
                write!(f, "{}", err)
            }
        }
    }
}

#[derive(Debug)]
pub struct FilterError<'a> {
    input: &'a str,
    kind: FilterErrorKind,
}

impl std::error::Error for FilterError<'_> {}

impl Display for FilterError<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        Display::fmt(&self.kind, f)
    }
}

impl FilterError<'_> {
    fn map_failure<'a, O, E>(
        input: &'a str,
        res: Result<O, E>,
    ) -> Result<O, nom::Err<FilterError<'a>>>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        match res {
            Ok(v) => Ok(v),
            Err(e) => Err(nom::Err::Failure(FilterError {
                input,
                kind: FilterErrorKind::Error(Box::new(e)),
            })),
        }
    }
}

impl<'a> nom::error::ParseError<&'a str> for FilterError<'a> {
    fn from_error_kind(input: &'a str, kind: nom::error::ErrorKind) -> Self {
        FilterError {
            input,
            kind: FilterErrorKind::Nom(kind),
        }
    }
    fn append(_input: &'a str, _kind: nom::error::ErrorKind, other: Self) -> Self {
        other
    }
}
macro_rules! build_error {
    ($input:expr, $kind: expr) => {{
        use FilterErrorKind::*;
        Err(nom::Err::Error(FilterError {
            input: $input,
            kind: $kind,
        }))
    }};
}
macro_rules! build_failure {
    ($input:expr, $kind: expr) => {{
        use FilterErrorKind::*;
        Err(nom::Err::Failure(FilterError {
            input: $input,
            kind: $kind,
        }))
    }};
}

fn string_literal(input: &str) -> IResult<&str, String, FilterError> {
    delimited(
        char('\''),
        fold_many0(
            alt((none_of("'"), map(tag("''"), |_| '\''))),
            String::new,
            |mut string, ch| {
                string.push(ch);
                string
            },
        ),
        char('\''),
    )(input)
}

fn string_criterion(input: &str) -> IResult<&str, BoolOp, FilterError> {
    let (input, (field, _, op, _, value)) = tuple((
        alpha1,
        multispace0,
        alt((tag("="), tag("!="))),
        multispace0,
        string_literal,
    ))(input)?;
    let criterion = match field {
        "AlarmClassName" => StringCriterion::AlarmClassName,
        "Name" => StringCriterion::AlarmName,
        _ => {
            return Err(nom::Err::Error(FilterError {
                input,
                kind: FilterErrorKind::InvalidCriterionName(field.to_string()),
            }))
        }
    };
    Ok((
        input,
        match op {
            "=" => BoolOp::StringEqual(criterion, value),
            "!=" => BoolOp::Not(Box::new(BoolOp::StringEqual(criterion, value))),
            _ => {
                return Err(nom::Err::Error(FilterError {
                    input,
                    kind: FilterErrorKind::IllegalCheckOperation(op.to_string()),
                }))
            }
        },
    ))
}
fn int_criterion(input: &str) -> IResult<&str, BoolOp, FilterError> {
    let (input, (field, _, op, _, value)) = tuple((
        alpha1,
        multispace0,
        alt((
            tag("!="),
            tag("="),
            tag("<="),
            tag(">="),
            tag("<"),
            tag(">"),
        )),
        multispace0,
        nom::character::complete::i32,
    ))(input)?;
    let criterion = match field {
        "ID" => IntCriterion::Id,
        "InstanceID" => IntCriterion::InstanceId,
        "Priority" => IntCriterion::Priority,
        _ => {
            return Err(nom::Err::Error(FilterError {
                input,
                kind: FilterErrorKind::InvalidCriterionName(field.to_string()),
            }))
        }
    };
    use BoolOp::*;
    Ok((
        input,
        match op {
            "=" => IntEqual(criterion, value),
            "!=" => Not(Box::new(BoolOp::IntEqual(criterion, value))),
            "<" => IntLess(criterion, value),
            "<=" => IntLessEqual(criterion, value),
            ">=" => Not(Box::new(BoolOp::IntLess(criterion, value))),
            ">" => Not(Box::new(BoolOp::IntLessEqual(criterion, value))),
            _ => {
                return Err(nom::Err::Error(FilterError {
                    input,
                    kind: FilterErrorKind::IllegalCheckOperation(op.to_string()),
                }))
            }
        },
    ))
}

fn state_criterion(input: &str) -> IResult<&str, BoolOp, FilterError> {
    let (input, (_, _, op, _, value)) = tuple((
        tag("State"),
        multispace0,
        alt((tag("!="), tag("="))),
        multispace0,
        map(
            alt((string_literal, map(digit1, |s: &str| s.to_owned()))),
            |v| AlarmState::from_str(&v),
        ),
    ))(input)?;
    let criterion = IntCriterion::AlarmState;
    let value = match value {
        Ok(v) => v,
        Err(e) => return build_failure!(input, Error(Box::new(e))),
    };
    Ok((
        input,
        match op {
            "=" => BoolOp::StateEqual(criterion, value),
            "!=" => BoolOp::Not(Box::new(BoolOp::StateEqual(criterion, value))),
            _ => return build_error!(input, IllegalCheckOperation(op.to_string())),
        },
    ))
}
/*
Left recursive
or := or "OR" or | and
and:= and "AND" and | not
not:= "NOT" not | arg
arg := "(" or ")" | comp

Right recursive
or := and or'
or' := "OR" or or' | empty

and := not and'
and' := "AND" and and' | empty

not:= "NOT" not | arg
arg := "(" expr ")" | comp

 */
fn parse_criterion(input: &str) -> IResult<&str, BoolOp, FilterError> {
    alt((state_criterion, int_criterion, string_criterion))(input)
}

fn parse_parenthesis(input: &str) -> IResult<&str, BoolOp, FilterError> {
    let (input, (_, res, _)) = tuple((tag("("), parse_or, tag(")")))(input)?;
    Ok((input, res))
}

fn parse_arg(input: &str) -> IResult<&str, BoolOp, FilterError> {
    alt((parse_parenthesis, parse_criterion))(input)
}

fn parse_not(input: &str) -> IResult<&str, BoolOp, FilterError> {
    alt((
        map(
            preceded(tuple((tag("NOT"), multispace0)), parse_arg),
            |op| BoolOp::Not(Box::new(op)),
        ),
        parse_arg,
    ))(input)
}

fn parse_or(input: &str) -> IResult<&str, BoolOp, FilterError> {
    let (input, (left, right)) = tuple((
        parse_and,
        fold_many0(
            preceded(tuple((multispace0, tag("OR"), multispace0)), parse_and),
            || None,
            |acc, op| {
                if let Some(acc) = acc {
                    Some(Box::new(BoolOp::Or(acc, Box::new(op))))
                } else {
                    Some(Box::new(op))
                }
            },
        ),
    ))(input)?;
    Ok((
        input,
        if let Some(right) = right {
            BoolOp::Or(Box::new(left), right)
        } else {
            left
        },
    ))
}

fn parse_and(input: &str) -> IResult<&str, BoolOp, FilterError> {
    let (input, (left, right)) = tuple((
        parse_not,
        fold_many0(
            preceded(tuple((multispace0, tag("AND"), multispace0)), parse_not),
            || None,
            |acc, op| {
                if let Some(acc) = acc {
                    Some(Box::new(BoolOp::And(acc, Box::new(op))))
                } else {
                    Some(Box::new(op))
                }
            },
        ),
    ))(input)?;
    Ok((
        input,
        if let Some(right) = right {
            BoolOp::And(Box::new(left), right)
        } else {
            left
        },
    ))
}

pub fn parse_filter<'a>(input: &'a str) -> Result<BoolOp, FilterError<'a>> {
    match terminated(parse_or, eof)(input) {
        Ok((_, op)) => Ok(op),
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => Err(e),
        Err(_) => unreachable!(),
    }
}

#[test]
fn test_criterion_parser() {
    assert_eq!(
        string_criterion("Name!='djkss'")
            .unwrap()
            .1
            .to_string(),
        "NOT (Name = 'djkss')"
    );
    assert_eq!(
        string_criterion("AlarmClassName = 'djkss'")
            .unwrap()
            .1
            .to_string(),
        "AlarmClassName = 'djkss'"
    );
    assert_eq!(
        int_criterion("Priority!=45").unwrap().1.to_string(),
        "NOT (Priority = 45)"
    );
    assert_eq!(
        state_criterion("State = 2").unwrap().1.to_string(),
        "State = 'RaisedCleared'"
    );
    assert_eq!(
        int_criterion("Priority< 6").unwrap().1.to_string(),
        "Priority < 6"
    );
    assert_eq!(
        int_criterion("Priority<= 6").unwrap().1.to_string(),
        "Priority <= 6"
    );
    assert_eq!(
        int_criterion("Priority  > 9").unwrap().1.to_string(),
        "NOT (Priority <= 9)"
    );
    assert_eq!(
        int_criterion("Priority >= 6").unwrap().1.to_string(),
        "NOT (Priority < 6)"
    );
}

#[test]
fn test_filter_parser() {
    assert_eq!(
        parse_filter("AlarmClassName = 'adjk' AND Priority < 8")
            .unwrap()
            .to_string(),
        "(AlarmClassName = 'adjk') AND (Priority < 8)"
    );
    assert_eq!(
        parse_filter("AlarmClassName = 'ad' AND State = 8 OR State = 5")
            .unwrap()
            .to_string(),
        "((AlarmClassName = 'ad') AND (State = 'Removed')) OR (State = 'RaisedAcknowledged')"
    );
    assert_eq!(
        parse_filter("AlarmClassName = 'ad' AND (State = 8 OR State = 'norMAL')")
            .unwrap()
            .to_string(),
        "(AlarmClassName = 'ad') AND ((State = 'Removed') OR (State = 'Normal'))"
    );
    assert_eq!(
        parse_filter("AlarmClassName = 'ad' OR NOT State = 'RaisedClearedAcknowledged' AND State = 'RaisedAcknowledgedCleared'")
            .unwrap()
            .to_string(),
        "(AlarmClassName = 'ad') OR ((NOT (State = 'RaisedClearedAcknowledged')) AND (State = 'RaisedAcknowledgedCleared'))"
    );
}

#[test]
fn test_alarm_state() {
    assert_eq!(AlarmState::from_str("NOrmal"), Ok(AlarmState::Normal));
    assert_eq!(
        AlarmState::from_str("in,ack"),
        Ok(AlarmState::RaisedAcknowledged)
    );
    assert_eq!(
        AlarmState::from_str("in,ack,out"),
        Ok(AlarmState::RaisedAcknowledgedCleared)
    );
    assert_eq!(
        AlarmState::from_str("incoming outgoing acknowledged"),
        Ok(AlarmState::RaisedClearedAcknowledged)
    );
    assert_eq!(AlarmState::from_str("REMOVED"), Ok(AlarmState::Removed));
    assert_eq!(
        AlarmState::from_str("7"),
        Ok(AlarmState::RaisedClearedAcknowledged)
    );
    assert_eq!(AlarmState::from_str("1").map(|s| s.as_str()), Ok("Raised"));
    assert_eq!(
        AlarmState::from_str("normal").map(|s| s.as_str()),
        Ok("Normal")
    );
    assert_eq!(
        AlarmState::from_str("raisedcleared").map(|s| s.as_str()),
        Ok("RaisedCleared")
    );
    assert_eq!(
        AlarmState::from_str("incoming,acknowledged").map(|s| s.as_str()),
        Ok("RaisedAcknowledged")
    );
    assert_eq!(
        AlarmState::from_str("in,out ack").map(|s| s.as_str()),
        Ok("RaisedClearedAcknowledged")
    );
    assert_eq!(
        AlarmState::from_str("in/ack/out").map(|s| s.as_str()),
        Ok("RaisedAcknowledgedCleared")
    );
    assert_eq!(AlarmState::from_str("8").map(|s| s.as_str()), Ok("Removed"));
}

#[test]
fn test_filter_parser_failure() {
    let res = parse_filter("AlarmClassName = 'ad' OR ");
    if let Err(FilterError {
        input: " OR ",
        kind: FilterErrorKind::Nom(Eof),
    }) = res
    {
        /* Nop */
    } else {
        panic!("Unexpected result: {:?}", res);
    }

    let res = parse_filter("AlarmClassName + 8");
    if let Err(FilterError {
        input: "+ 8",
        kind: FilterErrorKind::Nom(Tag),
    }) = res
    {
        /* Nop */
    } else {
        panic!("Unexpected result: {:?}", res);
    }
}

#[test]
fn test_filter_evaluate()
{
    let alarm_data = NotifyAlarm {
	name: "Foo".to_string(),
	id: "0".to_string(),
	alarm_class_name: "Warning".to_string(),
	alarm_class_symbol: "W".to_string(),
	event_text: "This is a warning".to_string(),
	instance_id: "52".to_string(),
	priority: "7".to_string(),
	state: "1".to_string(),
	state_text:"Incoming".to_string(),
	state_machine: "7".to_string(),
	modification_time: "019-01-30 11:25:39.9780320".to_string(),
    };
	
    let filter_text = "Name='Foo' AND ID=0 AND InstanceID=52 AND AlarmClassName ='Warning' AND Priority=7 AND State=1";
    let filter = parse_filter(filter_text).unwrap();
    assert_eq!(filter.evaluate(&alarm_data), true);
	
}
