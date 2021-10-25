use crate::event::AuthorAssociation;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Config {
    pub on: On,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct On {
    #[serde(default)]
    pub issue: Option<Vec<OnIssue>>,
    #[serde(default)]
    pub issue_comment: Option<Vec<OnIssueComment>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct OnIssue {
    #[serde(flatten)]
    pub common: OnCommon,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct OnIssueComment {
    #[serde(flatten)]
    pub common: OnCommon,

    pub r#if: Vec<IfIssueComment>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct OnCommon {
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IfIssueComment {
    Not(Box<IfIssueComment>),
    And(Vec<IfIssueComment>),
    Or(Vec<IfIssueComment>),
    IsPr,
    UserIs(Vec<AuthorAssociation>),
    UserIn(Vec<String>),
    Command(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Run(String),
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::Step::Run;

    #[test]
    fn test_parse() {
        let yaml = r#"---
on:
  issue_comment:
    - if:
        - command: "test"
        - user_is: ["OWNER", "MEMBER"]
        - is_pr
        - user_in: ["foo", "bar"]
      steps:
        - run: |
            echo "${{ github.event.issue.number }}"
"#;

        let cfg: Config = serde_yaml::from_str(yaml).expect("Must parse");

        assert_eq!(
            cfg.on.issue_comment.unwrap()[0],
            OnIssueComment {
                common: OnCommon {
                    steps: vec![Run("echo \"${{ github.event.issue.number }}\"\n".into())]
                },
                r#if: vec![
                    IfIssueComment::Command("test".into()),
                    IfIssueComment::UserIs(vec![
                        AuthorAssociation::Owner,
                        AuthorAssociation::Member,
                    ]),
                    IfIssueComment::IsPr,
                    IfIssueComment::UserIn(vec!["foo".into(), "bar".into()])
                ],
            }
        )
    }
}
