use crate::{github, logger};

use anyhow::{Context};
use reqwest::{Client};

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

pub enum RfcMergePrError {
    Octocrab(octocrab::Error),
    Anyhow(anyhow::Error)
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
            RfcMergePrError::Anyhow(e) => write!(w, "rfc merge error: {}", e),
            RfcMergePrError::Octocrab(e) => write!(w, "rfc merge error: {}", e),
        }
    }
}

pub async fn merge(pr_num: u64) -> Result<(), RfcMergePrError> {
    dbg!(pr_num);

    let client = Client::new();
    let gh = github::GithubClient::new_with_default_token(client.clone());
    let oc = octocrab::OctocrabBuilder::new()
        .personal_token(github::default_token_from_env())
        .build()
        .expect("Failed to build octocrab.");

    let branch_repo = dbg!(find_branch_repo(&gh, &oc, pr_num).await)?;

    // First, gather all the information we will need
    let mut extract = ExtractInfo::new(gh, oc, pr_num, branch_repo);
    let rfc_title = extract.find_rfc_title().await?;
    let filename = extract.find_text_filename().await?;
    let header = extract.extract_rfc_header(&filename).await?;

    let mut in_flight = extract.prepare_to_fly(rfc_title, filename, header);

    in_flight.create_tracking_issue().await?;
    /*
    in_flight.update_rfc_header_text().await?;
    in_flight.embed_issue_number_in_rfc_filename().await?;
    in_flight.post_final_steps_for_caller_to_follow().await?;
     */

    Err(anyhow::anyhow!("unfinished business").into())
}

#[derive(Debug)]
struct BranchRepo {
    repo_full_name: String,
    branch: String,
}

async fn find_branch_repo(
    gh: &github::GithubClient,
    oc: &octocrab::Octocrab,
    pr_num: u64)
    -> anyhow::Result<BranchRepo>
{
    let pr_handler = oc.pulls("rust-lang", "rfcs");
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

impl ExtractInfo {
    fn new(gh: github::GithubClient,
           oc: octocrab::Octocrab,
           pr_num: u64,
           branch_repo: BranchRepo,
    ) -> Self
    {
        let pr = github::PullRequestId {
            repo: github::Repository { full_name: "rust-lang/rfcs".to_string() },
            pull_number: pr_num,
        };
        Self { gh, oc, pr, branch_repo }
    }

    async fn find_rfc_title(&mut self) -> anyhow::Result<String> {
        Ok(self.pr.get_title(&self.gh).await?)
    }

    async fn find_text_filename(&mut self) -> anyhow::Result<String> {
        let file_diff_list = self.pr.get_file_list(&self.gh).await?;
        let mut candidates = file_diff_list.iter().filter(|d|d.filename.starts_with("text/0000-"));
        let candidate = match candidates.clone().count() {
            1 => candidates.next().unwrap().filename.clone(),
            count =>
                return Err(anyhow::anyhow!("expected one rfc file, found {}", count)),
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
    async fn create_tracking_issue(&mut self) -> anyhow::Result<()> {
        // let issues = self.oc.repos("rust-lang", "rust");
        let issues = self.oc.issues("pnkfelix", "triagebot-playpen");
        let title = format!("Tracking Issue for RFC {NNN}: {XXX}",
                            NNN=self.pr.pull_number, XXX=self.rfc_title);
        let mut context = tera::Context::new();
        context.insert("FEATURE", &self.header.feature_name);
        context.insert("PR_NUM", &self.pr.pull_number);
        context.insert("TITLE", &self.rfc_title);
        let body = crate::actions::TEMPLATES.render("tracking_issue.tt", &context)?;
        let issue = issues.create(title).body(body).send().await?;
        Ok(())
    }
}
