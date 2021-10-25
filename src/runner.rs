use crate::{
    config::{Config, IfIssueComment, OnCommon, OnIssueComment, Step},
    event::{CommonEvent, Event, IssueCommentEvent},
};
use jsonpath::Selector;
use lazy_static::lazy_static;
use regex::{Captures, Regex};
use serde_json::{Map, Value};
use std::{borrow::Cow, process::Command};

lazy_static! {
    static ref RE: Regex = Regex::new(r#"\$\{\{(.*?)\}\}"#).unwrap();
}

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
                        payload,
                    })?;
                }
            }
        }
        Ok(())
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

    /// if all checks eval to true, we return true too. Unless we have no checks at all.
    fn eval(&self, payload: &Self::Payload) -> anyhow::Result<bool> {
        if self.is_empty() {
            return Ok(false);
        }
        for s in self {
            if !s.eval(payload)? {
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
            Self::Not(expr) => Ok(!expr.eval(payload)?),
            Self::And(children) => children.eval(payload), // default is and
            Self::Or(children) => {
                // return true if at least one check returns true. No checks means false.
                let mut result = false;
                for c in children {
                    if c.eval(payload)? {
                        result = true;
                        break;
                    }
                }
                Ok(result)
            }
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
            Self::Command(expected) => is_command(expected, &payload.comment.body),
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

    let mut cmd = Command::new("bash");
    cmd.arg("--noprofile")
        .arg("--norc")
        .arg("-e")
        .arg("-o")
        .arg("pipefail")
        .arg("-c")
        .arg(eval(command, &context)?);

    log::info!("Running: {:?}", cmd);

    let status = cmd.status()?;

    if !status.success() {
        log::warn!("Failed to run command: {:?} = {:?}", cmd, status);
        anyhow::bail!("Failed run command");
    }

    Ok(())
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

        match val.as_slice() {
            [] => Ok(String::new()),
            [n] => Ok(n.to_string()),
            n => Err(anyhow::anyhow!("More than one item found: {:?}", n)),
        }
    }
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
