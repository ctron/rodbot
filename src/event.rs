use anyhow::Context;
use derefable::Derefable;
use serde::Deserialize;
use std::fs::File;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    IssueComment(IssueCommentEvent),
}

impl Event {
    pub fn from_env() -> anyhow::Result<Self> {
        match std::env::var("GITHUB_EVENT_NAME").as_deref() {
            Ok("issue_comment") => Ok(Event::IssueComment(
                Self::parse_payload().context("Failed to parse event payload")?,
            )),
            Ok(name) => Err(anyhow::anyhow!(
                "Unknown or unsupported event type: {}",
                name
            )),
            Err(_) => Err(anyhow::anyhow!("Missing GITHUB_EVENT_NAME")),
        }
    }

    pub fn parse_payload<T>() -> anyhow::Result<T>
    where
        for<'de> T: Deserialize<'de>,
    {
        Ok(serde_json::from_reader(File::open(
            std::env::var("GITHUB_EVENT_PATH").context("Failed getting GITHUB_EVENT_PATH")?,
        )?)?)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct CommonEvent {
    pub action: String,
    pub sender: Sender,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Derefable)]
pub struct IssueCommentEvent {
    #[deref(mutable)]
    #[serde(flatten)]
    pub common: CommonEvent,
    pub comment: Comment,
    pub issue: Issue,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Comment {
    pub author_association: AuthorAssociation,
    pub body: String,
    pub id: u64,
    pub user: User,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct User {
    pub login: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Issue {
    pub author_association: AuthorAssociation,
    pub body: Option<String>,
    pub comments: u64,
    pub id: u64,
    pub labels: Vec<Label>,
    pub locked: bool,
    pub number: u64,

    pub pull_request: Option<PullRequest>,

    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Label {
    pub color: String,
    pub default: bool,
    pub description: Option<String>,
    pub id: u64,
    pub name: String,
    pub node_id: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct PullRequest {
    pub diff_url: String,
    pub html_url: String,
    pub patch_url: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum AuthorAssociation {
    Collaborator,
    Contributor,
    FirstTimer,
    FirstTimeContributor,
    Mannequin,
    Member,
    None,
    Owner,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Sender {
    pub login: String,
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs::File;

    #[test]
    fn test_parse() -> anyhow::Result<()> {
        let event: IssueCommentEvent =
            serde_json::from_reader(File::open("test/issue_comment_1.json")?)?;

        assert_eq!(event.action, "created");

        Ok(())
    }
}
