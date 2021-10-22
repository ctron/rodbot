use crate::config::{Config, IfIssueComment, OnCommon, OnIssueComment, Step};
use crate::event::{CommonEvent, Event, IssueCommentEvent};
use cel_interpreter::objects::*;
use jsonpath::Selector;
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::HashMap;
use std::process::Command;
use std::rc::Rc;

pub trait Runner<'c> {
    type Payload;

    fn run(&self, payload: &Self::Payload) -> anyhow::Result<()>;
}

pub trait Eval {
    type Payload;

    fn eval(&self, payload: &Self::Payload) -> anyhow::Result<bool>;
}

pub struct Context<'c, T> {
    pub payload: &'c T,
    pub context: &'c serde_json::Value,
}

impl<'c> Runner<'c> for Config {
    type Payload = Context<'c, Event>;

    fn run(&self, context: &Self::Payload) -> anyhow::Result<()> {
        match &context.payload {
            Event::IssueComment(payload) => {
                if let Some(runner) = &self.on.issue_comment {
                    runner.run(&Context {
                        context: context.context,
                        payload: &payload,
                    })?;
                }
            }
        }
        Ok(()).into()
    }
}

impl<'c, T, P> Runner<'c> for Vec<T>
where
    T: Runner<'c, Payload = P>,
{
    type Payload = P;

    fn run(&self, payload: &Self::Payload) -> anyhow::Result<()> {
        for item in self {
            item.run(payload)?;
        }
        Ok(())
    }
}

impl<'c> Runner<'c> for OnIssueComment {
    type Payload = Context<'c, IssueCommentEvent>;

    fn run(&self, payload: &Self::Payload) -> anyhow::Result<()> {
        for i in &self.r#if {
            if !i.eval(payload.payload)? {
                log::debug!("Test rejected, aborting!");
                return Ok(());
            }
        }

        // running steps

        self.common.run(&Context {
            context: payload.context,
            payload: &payload.payload.common,
        })?;

        // done

        Ok(())
    }
}

impl<'c> Runner<'c> for OnCommon {
    type Payload = Context<'c, CommonEvent>;

    fn run(&self, payload: &Self::Payload) -> anyhow::Result<()> {
        self.steps.run(payload.context)?;
        Ok(())
    }
}

impl Runner<'_> for Step {
    type Payload = serde_json::Value;

    fn run(&self, payload: &Self::Payload) -> anyhow::Result<()> {
        match self {
            Self::Run(command) => run(command, payload)?,
        }

        Ok(())
    }
}

impl<T, P> Eval for Vec<T>
where
    T: Eval<Payload = P>,
{
    type Payload = P;

    fn eval(&self, payload: &Self::Payload) -> anyhow::Result<bool> {
        for s in self {
            if !s.eval(&payload)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl Eval for IfIssueComment {
    type Payload = IssueCommentEvent;

    fn eval(&self, payload: &Self::Payload) -> anyhow::Result<bool> {
        let r = match self {
            Self::IsPr => Ok(payload.issue.pull_request.is_some()),
            Self::UserIs(expected) => {
                let result = expected.contains(&payload.comment.author_association);
                log::debug!(
                    "UserIs({:?}) == {:?} => {}",
                    expected,
                    &payload.comment.author_association,
                    result
                );
                Ok(result)
            }
            Self::UserIn(expected) => Ok(expected.contains(&payload.comment.user.login)),
            Self::Command(expected) => is_command(&expected, &payload.comment.body),
        };

        log::debug!("{:?} => {:?}", self, r);

        r
    }
}

fn is_command(command: &str, body: &str) -> anyhow::Result<bool> {
    if let Some(line) = body.lines().next() {
        Ok(line.trim().starts_with(&format!("/{}", command)))
    } else {
        Ok(false)
    }
}

fn run(command: &str, context: &serde_json::Value) -> anyhow::Result<()> {
    let context = match context {
        Value::Object(m) => Cow::Borrowed(m),
        _ => Cow::Owned(Map::new()),
    };

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(eval(command, &context)?);

    log::info!("Running: {:?}", cmd);

    cmd.status()?;

    Ok(())
}

lazy_static! {
    static ref RE: Regex = Regex::new(r#"\$\{\{(.*?)\}\}"#).unwrap();
}

struct CelReplacer<'a> {
    pub context: cel_interpreter::context::Context,
    pub errors: &'a mut Vec<anyhow::Error>,
}

impl<'a> CelReplacer<'a> {
    pub fn new(context: &Map<String, Value>, errors: &'a mut Vec<anyhow::Error>) -> Self {
        let context = cel_interpreter::context::Context {
            variables: convert_map(context),
            functions: Default::default(),
        };
        Self { context, errors }
    }
}

impl<'a> regex::Replacer for CelReplacer<'a> {
    fn replace_append(&mut self, caps: &Captures<'_>, dst: &mut String) {
        log::debug!("Replace: {:#?}", caps);

        let expr = &caps[1];
        let p = match cel_interpreter::Program::compile(expr) {
            Ok(p) => p,
            Err(err) => {
                log::debug!("Failed to compile: '{}' -> {}", expr, err);
                self.errors.push(err.into());
                return;
            }
        };

        match p.execute(&self.context) {
            CelType::String(s) => dst.push_str(s.as_str()),
            CelType::Int(v) => dst.push_str(&format!("{}", v)),
            CelType::Bool(v) => dst.push_str(&format!("{}", v)),
            CelType::Float(v) => dst.push_str(&format!("{}", v)),
            CelType::UInt(v) => dst.push_str(&format!("{}", v)),
            CelType::Null => {}
            v => {
                self.errors
                    .push(anyhow::anyhow!("Unsupported return type: {:?}", v));
            }
        };
    }
}

struct JsonPathReplacer<'a> {
    pub context: Value,
    pub errors: &'a mut Vec<anyhow::Error>,
}
impl<'a> JsonPathReplacer<'a> {
    pub fn new(context: &Map<String, Value>, errors: &'a mut Vec<anyhow::Error>) -> Self {
        Self {
            context: Value::Object(context.clone()),
            errors,
        }
    }
}
impl<'a> regex::Replacer for JsonPathReplacer<'a> {
    fn replace_append(&mut self, caps: &Captures<'_>, dst: &mut String) {
        log::debug!("Replace: {:#?}", caps);

        let expr = &caps[1];

        match self.replace(expr) {
            Err(err) => self.errors.push(err),
            Ok(s) => {
                dst.push_str(s.as_str());
            }
        }
    }
}

impl<'a> JsonPathReplacer<'a> {
    fn replace(&self, expr: &str) -> anyhow::Result<String> {
        let expr = expr.trim();
        let path = format!("$.{}", expr);
        let sel = Selector::new(&path).map_err(|err| anyhow::anyhow!("{}", err))?;
        let val = sel
            .find(&self.context)
            .filter_map(|t| match t {
                Value::String(s) => Some(s.to_string()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();

        log::debug!("{} ({}) => {:?}", expr, path, val);
        // log::debug!("Context: {:#?}", self.context);

        match val.as_slice() {
            [] => Ok(String::new()),
            [n] => Ok(n.to_string()),
            n => Err(anyhow::anyhow!("More than one item found: {:?}", n)),
        }
    }
}

fn convert(value: &serde_json::Value) -> CelType {
    match value {
        Value::Null => CelType::Null,
        Value::String(s) => CelType::String(Rc::new(s.to_string())),
        Value::Bool(b) => CelType::Bool(*b),
        Value::Number(n) => {
            if let Some(n) = n.as_f64() {
                CelType::Float(n)
            } else if let Some(n) = n.as_u64() {
                CelType::UInt(n as u32)
            } else if let Some(n) = n.as_i64() {
                CelType::Int(n as i32)
            } else {
                CelType::Null
            }
        }
        Value::Array(a) => {
            // FIXME: handle arrays
            CelType::Null
        }
        Value::Object(m) => CelType::Map(CelMap {
            map: Rc::new(
                convert_map(m)
                    .into_iter()
                    .map(|(k, v)| (CelKey::String(Rc::new(k)), v))
                    .collect(),
            ),
        }),
    }
}

fn convert_map(map: &Map<String, Value>) -> HashMap<String, CelType> {
    let mut variables = HashMap::new();
    for (k, v) in map {
        variables.insert(k.to_string(), convert(v));
    }
    variables
}

fn eval(text: &str, context: &serde_json::Map<String, Value>) -> anyhow::Result<String> {
    let mut errors = Vec::<anyhow::Error>::new();

    // let replacer = CelReplacer::new(context, &mut errors);
    let replacer = JsonPathReplacer::new(context, &mut errors);

    let text = RE.replace_all(text, replacer);

    if !errors.is_empty() {
        if errors.len() == 1 {
            Err(errors.into_iter().next().unwrap())
        } else {
            Err(anyhow::anyhow!("Failed with multiple errors: {:?}", errors))
        }
    } else {
        Ok(text.into())
    }
}

#[cfg(test)]
mod test {

    use super::*;
    use serde_json::json;

    #[test]
    fn test_1() {
        env_logger::try_init().ok();

        let r = eval(
            "Hello ${{ foo.value }}!",
            json!({"foo": {"value": "World"}}).as_object().unwrap(),
        )
        .expect("To compile");
        assert_eq!(r, "Hello World!");
    }
}
