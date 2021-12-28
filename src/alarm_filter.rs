use crate::open_pipe::connection::NotifyAlarm;
use nom::branch::alt;
use nom::bytes::complete::tag;
use nom::character::complete::alpha1;
use nom::character::complete::char;
use nom::character::complete::digit1;
use nom::character::complete::multispace0;
use nom::character::complete::none_of;
use nom::combinator::map;
use nom::multi::fold_many0;
use nom::sequence::{preceded, terminated, tuple};
use nom::IResult;
use std::fmt::Debug;
use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
enum AlarmState {
    Normal = 0,
    Raised = 1,
    RaisedCleared = 2,
    RaisedAcknowledged = 5,
    RaisedAcknowledgedCleared = 6,
    RaisedClearedAcknowledged = 7,
    Removed = 8,
}

#[derive(Debug)]
pub enum StringCriterion {
    AlarmClassName,
    AlarmName,
}

impl StringCriterion {
    pub fn evaluate<'a>(&self, alarm: &'a NotifyAlarm) -> &'a str {
        match self {
            StringCriterion::AlarmClassName => &alarm.alarm_class_name,
            StringCriterion::AlarmName => &alarm.name,
        }
    }

    pub fn as_str<'a>(&self) -> &'a str {
        match self {
            StringCriterion::AlarmClassName => &"AlarmClassName",
            StringCriterion::AlarmName => &"AlarmName",
        }
    }
}

#[derive(Debug)]
pub enum IntCriterion {
    Id,
    InstanceId,
    Priority,
    AlarmState,
}

impl IntCriterion {
    pub fn evaluate(&self, alarm: &NotifyAlarm) -> i32 {
        match self {
            IntCriterion::Id => alarm.id.parse().unwrap_or(0),
            IntCriterion::InstanceId => alarm.instance_id.parse().unwrap_or(0),
            IntCriterion::Priority => alarm.priority.parse().unwrap_or(0),
            IntCriterion::AlarmState => alarm.state.parse().unwrap_or(0),
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

#[derive(Debug)]
pub enum BoolOp {
    Not(Box<BoolOp>),
    And(Box<BoolOp>, Box<BoolOp>),
    Or(Box<BoolOp>, Box<BoolOp>),
    StringEqual(Box<StringCriterion>, String),
    IntEqual(Box<IntCriterion>, i32),
    IntLess(Box<IntCriterion>, i32),
    IntLessEqual(Box<IntCriterion>, i32),
}

use BoolOp::*;

impl BoolOp {
    pub fn evaluate(&self, alarm: &NotifyAlarm) -> bool {
        match self {
            Not(arg) => !arg.evaluate(alarm),
            And(arg1, arg2) => arg1.evaluate(alarm) && arg2.evaluate(alarm),
            Or(arg1, arg2) => arg1.evaluate(alarm) || arg2.evaluate(alarm),
            StringEqual(criterion, value) => criterion.evaluate(alarm) == value,
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
    InvalidCriterionName,
    IllegalCheckOpereation,
    Nom(nom::error::ErrorKind),
    Error(Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl Display for FilterErrorKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            FilterErrorKind::InvalidCriterionName => {
                write!(f, "Name of filter criterion not recognized")
            }
            FilterErrorKind::IllegalCheckOpereation => {
                write!(f, "Illegal comparison operator")
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
    fn nom<'a>(err: nom::error::Error<&'a str>) -> FilterError<'a> {
        FilterError {
            input: err.input,
            kind: FilterErrorKind::Nom(err.code),
        }
    }

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

impl<'a> From<nom::error::Error<&'a str>> for FilterError<'a> {
    fn from(nom_err: nom::error::Error<&'a str>) -> Self {
        FilterError {
            input: nom_err.input,
            kind: FilterErrorKind::Nom(nom_err.code),
        }
    }
}

fn string_literal(input: &str) -> IResult<&str, String> {
    preceded(
        char('\''),
        terminated(
            fold_many0(
                alt((none_of("'"), map(tag("''"), |_| '\''))),
                String::new,
                |mut string, ch| {
                    string.push(ch);
                    string
                },
            ),
            char('\''),
        ),
    )(input)
}

fn string_criterion(input: &str) -> IResult<&str, BoolOp, FilterError> {
    let (input, (field, _, op, _, value)) = tuple((
        alpha1,
        multispace0,
        alt((tag("="), tag("!="))),
        multispace0,
        string_literal,
    ))(input)
    .map_err(|e| match e {
        nom::Err::Error(e) => nom::Err::Error(FilterError::nom(e)),
        nom::Err::Failure(e) => nom::Err::Failure(FilterError::nom(e)),
        nom::Err::Incomplete(needed) => nom::Err::Incomplete(needed),
    })?;
    let criterion = Box::new(match field {
        "AlarmClassName" => StringCriterion::AlarmClassName,
        "AlarmName" => StringCriterion::AlarmName,
        _ => {
            return Err(nom::Err::Error(FilterError {
                input,
                kind: FilterErrorKind::InvalidCriterionName,
            }))
        }
    });
    Ok((
        input,
        match op {
            "=" => BoolOp::StringEqual(criterion, value),
            "!=" => BoolOp::Not(Box::new(BoolOp::StringEqual(criterion, value))),
            _ => {
                return Err(nom::Err::Error(FilterError {
                    input,
                    kind: FilterErrorKind::IllegalCheckOpereation,
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
        digit1,
    ))(input)
    .map_err(|e| match e {
        nom::Err::Error(e) => nom::Err::Error(FilterError::nom(e)),
        nom::Err::Failure(e) => nom::Err::Failure(FilterError::nom(e)),
        nom::Err::Incomplete(needed) => nom::Err::Incomplete(needed),
    })?;
    let criterion = Box::new(match field {
        "ID" => IntCriterion::Id,
        "InstanceID" => IntCriterion::InstanceId,
        "Priority" => IntCriterion::Priority,
        "State" => IntCriterion::AlarmState,
        _ => {
            return Err(nom::Err::Error(FilterError {
                input,
                kind: FilterErrorKind::InvalidCriterionName,
            }))
        }
    });
    let value = FilterError::map_failure(input, value.parse())?;
    Ok((
        input,
        match op {
            "=" => BoolOp::IntEqual(criterion, value),
            "!=" => BoolOp::Not(Box::new(BoolOp::IntEqual(criterion, value))),
            "<" => BoolOp::IntLess(criterion, value),
            "<=" => BoolOp::IntLessEqual(criterion, value),
            ">=" => BoolOp::Not(Box::new(BoolOp::IntLess(criterion, value))),
            ">" => BoolOp::Not(Box::new(BoolOp::IntLessEqual(criterion, value))),
            _ => {
                return Err(nom::Err::Error(FilterError {
                    input,
                    kind: FilterErrorKind::IllegalCheckOpereation,
                }))
            }
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
    alt((int_criterion, string_criterion))(input)
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

pub fn parse_filter(input: &str) -> IResult<&str, BoolOp, FilterError> {
    parse_or(input)
}

#[test]
fn test_criterion_parser() {
    assert_eq!(
        string_criterion("AlarmName!='djkss'")
            .unwrap()
            .1
            .to_string(),
        "NOT (AlarmName = 'djkss')"
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
        int_criterion("State = 4").unwrap().1.to_string(),
        "State = 4"
    );
    assert_eq!(
        int_criterion("State< 6").unwrap().1.to_string(),
        "State < 6"
    );
    assert_eq!(
        int_criterion("State<= 6").unwrap().1.to_string(),
        "State <= 6"
    );
    assert_eq!(
        int_criterion("State  > 9").unwrap().1.to_string(),
        "NOT (State <= 9)"
    );
    assert_eq!(
        int_criterion("State >= 6").unwrap().1.to_string(),
        "NOT (State < 6)"
    );
}

#[test]
fn test_filter_parser() {
    assert_eq!(
        parse_filter("AlarmClassName = 'adjk' AND Priority < 8")
            .unwrap()
            .1
            .to_string(),
        "(AlarmClassName = 'adjk') AND (Priority < 8)"
    );
    assert_eq!(
        parse_filter("AlarmClassName = 'ad' AND State = 8 OR State = 4")
            .unwrap()
            .1
            .to_string(),
        "((AlarmClassName = 'ad') AND (State = 8)) OR (State = 4)"
    );
    assert_eq!(
        parse_filter("AlarmClassName = 'ad' AND (State = 8 OR State = 4)")
            .unwrap()
            .1
            .to_string(),
        "(AlarmClassName = 'ad') AND ((State = 8) OR (State = 4))"
    );
    assert_eq!(
        parse_filter("AlarmClassName = 'ad' OR NOT State = 8 AND State = 4")
            .unwrap()
            .1
            .to_string(),
        "(AlarmClassName = 'ad') OR ((NOT (State = 8)) AND (State = 4))"
    );
}
