// use self::deployment::{RFCS_REPO, RUST_REPO};
use self::testing::{RFCS_REPO, RUST_REPO};

use crate::{github};

use anyhow::{Context};
use reqwest::{Client};

use std::convert::TryFrom;

#[cfg(not_now)]
struct RfcMergePrInFlight<'a> {
    ctx: &'a Context,
    rfc_issue: &'a Issue,

    /// As triagebot tries to do the steps to merge the RFC, it reports its
    /// findings here. At the end, it issues a comment to the RFC with the
    /// results of the steps, as well as the follow-on steps it expects the
    /// human caller to do.
    response_comment: String,

    /// The feature name for the RFC. We need to extract this from the RFC text;
    /// starts off as Uninitialized and then is updated based on that later
    /// extraction. If we are unable to find a feature name in the RFC, then it
    /// is marked as Absent.
    feature_name: FeatureName,

    /// Part of merging an RFC is creating a fresh tracking issue on rust-lang/rust.
    /// This is the number for that tracking issue, once it is created.
    tracking_issue_number: Option<u64>,
}

#[derive(Debug)]
pub enum RfcMergePrError {
    Octocrab(octocrab::Error),
    Anyhow(anyhow::Error),
    TryFrom(std::num::TryFromIntError),
}

impl From<std::num::TryFromIntError> for RfcMergePrError {
    fn from(e: std::num::TryFromIntError) -> Self { RfcMergePrError::TryFrom(e) }
}

impl From<anyhow::Error> for RfcMergePrError {
    fn from(e: anyhow::Error) -> Self { RfcMergePrError::Anyhow(e) }
}

impl From<octocrab::Error> for RfcMergePrError {
    fn from(e: octocrab::Error) -> Self { RfcMergePrError::Octocrab(e) }
}

impl std::fmt::Display for RfcMergePrError {
    fn fmt(&self, w: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            RfcMergePrError::Anyhow(e) => write!(w, "rfc merge error, anyhow:{:#}", e),
            RfcMergePrError::Octocrab(e) => write!(w, "rfc merge error, octocrab: {}", e),
            RfcMergePrError::TryFrom(e) => write!(w, "rfc merge error, try_from:{:#}", e),
        }
    }
}

pub async fn merge(pr_num: u64) -> Result<(), RfcMergePrError> {
    let client = Client::new();
    let gh = github::GithubClient::new_with_default_token(client.clone());
    let oc = octocrab::OctocrabBuilder::new()
        .personal_token(github::default_token_from_env())
        .build()
        .expect("Failed to build octocrab.");

    let branch_repo = find_branch_repo(&gh, &oc, pr_num).await?;

    // First, gather all the information we will need
    let mut extract = ExtractInfo::new(gh, oc, pr_num, branch_repo);
    let rfc_title = extract.find_rfc_title().await?;
    let filename = extract.find_text_filename().await?;
    let header = extract.extract_rfc_header(&filename).await?;

    let mut in_flight = extract.prepare_to_fly(rfc_title, filename, header);

    let tracking_issue = in_flight.create_tracking_issue().await?;
    in_flight.update_rfc_header_text(u64::try_from(tracking_issue.number)?).await?;
    /*
    in_flight.embed_rfc_issue_number_in_rfc_filename().await?;
    in_flight.post_final_steps_for_caller_to_follow().await?;
     */

    Err(anyhow::anyhow!("unfinished business").into())
}

#[derive(Debug)]
struct BranchRepo {
    repo_full_name: String,
    branch: String,
}

struct OrgRepo {
    org: &'static str,
    repo: &'static str,
}

mod deployment {
    use super::OrgRepo;
    pub(super) const RFCS_REPO: OrgRepo = OrgRepo { org: "rust-lang", repo: "rfcs" };
    pub(super) const RUST_REPO: OrgRepo = OrgRepo { org: "rust-lang", repo: "rust" };
}

mod testing {
    use super::OrgRepo;
    pub(super) const RFCS_REPO: OrgRepo = OrgRepo { org: "pnkfx", repo: "rfcs-play" };
    pub(super) const RUST_REPO: OrgRepo = OrgRepo { org: "pnkfelix", repo: "triagebot-playpen" };
}

impl OrgRepo {
    fn full_name(&self) -> String { format!("{}/{}", self.org, self.repo) }

    fn github_repo(&self) -> github::Repository {
        github::Repository { full_name: self.full_name() }
    }

    fn pull_request(&self, pr_num: u64) -> github::PullRequestId {
        github::PullRequestId {
            repo: self.github_repo(),
            pull_number: pr_num,
        }
    }

    fn pulls<'oc>(&self, oc: &'oc octocrab::Octocrab) -> octocrab::pulls::PullRequestHandler<'oc> {
        oc.pulls(self.org, self.repo)
    }

    fn issues<'oc>(&self, oc: &'oc octocrab::Octocrab) -> octocrab::issues::IssueHandler<'oc> {
        oc.issues(self.org, self.repo)
    }
}

async fn find_branch_repo(
    gh: &github::GithubClient,
    oc: &octocrab::Octocrab,
    pr_num: u64)
    -> anyhow::Result<BranchRepo>
{
    let pr_handler = RFCS_REPO.pulls(oc);
    let pr = pr_handler.get(pr_num).await?;
    let repo = if let Some(repo) = pr.head.repo { repo } else {
        return Err(anyhow::anyhow!("no remote repo found for PR {}", pr_num).into());
    };
    Ok(BranchRepo { repo_full_name: repo.full_name, branch: pr.head.ref_field })
}

struct ExtractInfo {
    gh: github::GithubClient,
    oc: octocrab::Octocrab,
    pr: github::PullRequestId,
    branch_repo: BranchRepo,
}

struct InFlight {
    gh: github::GithubClient,
    oc: octocrab::Octocrab,
    pr: github::PullRequestId,
    branch_repo: BranchRepo,
    rfc_title: String,
    text_filename: String,
    header: Header,
    // TODO: team
    // TODO: unresolved questions
}

#[derive(Debug)]
struct Header {
    feature_name: String,
    start_date: String,
    rfc_pr: String,
    rust_issue: String,
}

impl Header {
    fn feature_name(&self) -> anyhow::Result<Option<&str>> {
        // template: "- Feature Name: `FFF`"
        let msg = "header feature line did not match template";
        let prefix = "- Feature Name: ";
        let mut stripped: &str = self.feature_name
            .strip_prefix(prefix)
            .ok_or(anyhow::anyhow!("{}; needs to start with {}", msg, prefix))?;

        stripped = stripped.trim();
        stripped = stripped.strip_prefix("`").unwrap_or(stripped);
        stripped = stripped.strip_suffix("`").unwrap_or(stripped);
        stripped = stripped.trim();

        match stripped {
            "N/A" => Ok(None),
            "" => Ok(None),
            stripped => Ok(Some(stripped))
        }
    }
}

impl ExtractInfo {
    fn new(gh: github::GithubClient,
           oc: octocrab::Octocrab,
           pr_num: u64,
           branch_repo: BranchRepo,
    ) -> Self
    {
        let pr = RFCS_REPO.pull_request(pr_num);
        Self { gh, oc, pr, branch_repo }
    }

    async fn find_rfc_title(&mut self) -> anyhow::Result<String> {
        Ok(self.pr.get_title(&self.gh).await?)
    }

    async fn find_text_filename(&mut self) -> anyhow::Result<String> {
        let file_diff_list = self.pr.get_file_list(&self.gh).await?;
        let mut candidates = file_diff_list.iter().filter(|d|d.filename.starts_with("text/"));
        let candidate = match candidates.clone().count() {
            1 => candidates.next().unwrap().filename.clone(),
            count => {
                let filenames: Vec<_> = self
                    .pr
                    .get_file_list(&self.gh)
                    .await?
                    .iter()
                    .map(|d|d.filename.clone())
                    .collect();
                return Err(anyhow::anyhow!("expected one rfc file, found {}: {:?}",
                                           count, filenames));
            }
        };
        Ok(candidate)
    }

    async fn extract_rfc_header(&mut self, text_filename: &str) -> anyhow::Result<Header> {
        let repo = &self.branch_repo.repo_full_name;
        let branch = &self.branch_repo.branch;
        let path = text_filename;
        let text: String = self
            .gh
            .raw_file(repo, branch, path)
            .await?
            .ok_or(anyhow::anyhow!("RFC for {}/{}/{} not found", repo, branch, path))
            .and_then(|x|Ok(String::from_utf8_lossy(&x[..]).into_owned()))?;
        let mut header = text.lines().take(4).map(|x|x.to_owned());
        let feature_name = header.next().ok_or(anyhow::anyhow!("missing line 1"))?;
        let start_date = header.next().ok_or(anyhow::anyhow!("missing line 2"))?;
        let rfc_pr = header.next().ok_or(anyhow::anyhow!("missing line 3"))?;
        let rust_issue = header.next().ok_or(anyhow::anyhow!("missing line 4"))?;
        if !feature_name.starts_with("- Feature Name: `") {
            return Err(anyhow::anyhow!("malformed feature line: {}", feature_name));
        }
        if !start_date.starts_with("- Start Date: ") {
            return Err(anyhow::anyhow!("malformed start date line: {}", start_date));
        }
        if !rfc_pr.starts_with("- RFC PR: ") {
            return Err(anyhow::anyhow!("malformed rfc pr line: {}", rfc_pr));
        }
        if !rust_issue.starts_with("- Rust Issue: ") {
            return Err(anyhow::anyhow!("malformed rust issue line: {}", rust_issue));
        }
        let header = Header { feature_name, start_date, rfc_pr, rust_issue };
        Ok(header)
    }

    fn prepare_to_fly(
        self,
        rfc_title: String,
        text_filename: String,
        header: Header
    ) -> InFlight
    {
        InFlight {
            gh: self.gh,
            oc: self.oc,
            pr: self.pr,
            branch_repo: self.branch_repo,
            rfc_title,
            text_filename,
            header,
        }
    }
}

impl InFlight {
    async fn create_tracking_issue(&mut self) -> anyhow::Result<octocrab::models::issues::Issue> {
        let issues = RUST_REPO.issues(&self.oc);
        let title = format!("Tracking Issue for RFC {NNN}: {XXX}",
                            NNN=self.pr.pull_number, XXX=self.rfc_title);
        let mut context = tera::Context::new();
        if let Some(feature_name) = self.header.feature_name()? {
            context.insert("FEATURE", &feature_name);
        }
        context.insert("PR_NUM", &self.pr.pull_number);
        context.insert("TITLE", &self.rfc_title);
        let body = crate::actions::TEMPLATES.render("tracking_issue.tt", &context)?;
        let issue = issues.create(title).body(body).send().await?;
        Ok(issue)
    }

    async fn update_rfc_header_text(&mut self, tracking_issue: u64) -> anyhow::Result<String> {
        let feature_line = self.header
            .feature_name()?
            .map(|f|format!("- Feature Name: `{}`\n", f))
            .unwrap_or("".to_string());
        let body = format!("\
```suggestion
{FFFF_LINE}\
{START_DATE}
- RFC PR: [rust-lang/rfcs#{NNNN}](https://github.com/rust-lang/rfcs/pull/{NNNN})
- Rust Issue: [rust-lang/rust#{TTTT}](https://github.com/rust-lang/rust/issues/{TTTT})
```
",
                           START_DATE=self.header.start_date,
                           FFFF_LINE=feature_line,
                           TTTT=tracking_issue,
                           NNNN=self.pr.pull_number);
        let mut comment = github::ReviewCommentDiffAddress::MultiLine {
            // FIXME: the commit is required, despite what the Github API
            // documentation says, but obviously I should be extracting it or
            // feeding it in from up above rather than hard coding it.
            commit_id: "a8886a1a2d5edb9c247922e8058fb0a573f0755b".to_string(),
            path: self.text_filename.clone(),
            first: (1, github::DiffSide::Right),
            last: (4, github::DiffSide::Right),
        }.comment(body);
        self.pr
            .post_review_comment(&self.gh, comment)
            .await
            .map(|c|c.body)
    }
}
