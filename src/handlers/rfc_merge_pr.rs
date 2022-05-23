//! Purpose: Allow team members to merge RFC's via a comment.
//!
//! Parsing is done in the `parser::command::rfc_merge_pr` module.
//!
//! Merging RFC Pull Request #NNN should have three effects:
//!
//! 1. open a tracking issue on rust-lang/rust for the RFC (call this #MMM)
//! 2. update the header of the RFC text to embed #MMM and #NNN.
//! 3. rename the RFC text file to replace 0000 with NNN.

use crate::{config::RfcMergePrConfig, github::Event, handlers::Context};

use parser::command::rfc_merge_pr::RfcMergePrCommand;

pub(super) async fn handle_command(
    ctx: &Context,
    _config: &RfcMergePrConfig,
    event: &Event,
    cmd: RfcMergePrCommand,
) -> anyhow::Result<()> {
    let rfc_issue = event.issue().unwrap();
    let user = event.user();

    let mut in_flight = RfcMergePrInFlight::new(ctx, rfc_issue);
    in_flight.extract_feature_name_from_rfc_text().await?;
    in_flight.create_tracking_issue().await?;
    in_flight.update_rfc_header_text().await?;
    in_flight.embed_issue_number_in_rfc_filename().await?;
    in_flight.post_final_steps_for_caller_to_follow().await?;

    Ok(())
}

enum FeatureName {
    Uninitialized,
    Absent,
    Present(String),
}

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

impl<'a> RfcMergePrInFlight<'a> {
    fn new(ctx: &'a Context, rfc_issue: &'a Issue) -> Self {
        RfcMergePrInFlight {
            ctx,
            rfc_issue,
            response_comment: String::new(),
            feature_name: FeatureName::Uninitialized,
            tracking_issue_number: None,
        }
    }

    async fn extract_feature_name_from_rfc_text(&mut self) -> anyhow::Result<()> {
        assert!(self.feature_name.is_none());

        let opt_feature_name = unimplemented!();

        if let Some(feature_name) = opt_feature_name {
            self.feature_name = FeatureName::Present(feature_name.clone());
            self.response_comment.push_str("Extracted feature name: `{}`\n", feature_name);
        }
        Ok(feature_name)
    }

    async fn create_tracking_issue(&mut self) -> anyhow::Result<()> {
        let octocrab = &ctx.octocrab;
        let rust_issues = octocrab.issues("rust-lang", "rust");

        let body = self.format_tracking_issue_body(self.rfc_issue, self.feature_name).await;
        let issue = rust_issues
            .create(format!("Tracking Issue for {XXX}", XXX=self.rfc_issue.title))
            .body(body)
        // FIXME: it might be nice to infer the initial set of labels based on the
        // labels on the original RFC PR.
            .labels(None)
            .send()
            .await?;

        self.tracking_issue_number = Some(issue.number);

        Ok(())
    }

    async fn update_rfc_header_text(&self) {
        let octocrab = &self.ctx.octocrab;

        let rfcs_fork = octocrab.repos("rustbot", "rfcs");
        let rfcs_base = octocrab.repos("rust-lang", "rfcs");

        unimplemented!()
    }

    async fn embed_issue_number_in_rfc_filename(&self) {
        unimplemented!()
    }

    /// Remaining steps: report these to the person merging the RFC itself in
    /// response to their comment.
    async fn post_final_steps_for_caller_to_follow(&self) {
        let message_to_caller = format!(r#"
Remember to:

 [ ] add team labels to the tracking issue #{MMM}, e.g. `T-lang`, `T-libs`, etc,
 [ ] add feature label `F-{FFF}` ot the the tracking issue #{MMM},
 [ ] transcribe all the "unresolved questions" from RFC into body of tracking issue #{MMM},

"#, FFF=self.feature_name, MMM=self.tracking_issue_number);

        self.rfc_issue.post_comment(&ctx.github, message_to_caller).await?;
    }

    fn format_tracking_issue_body(&self) -> String {
        let feature_line = match self.feature_name {
            FeatureName::Absent => String::new();
            FeatureName::Present(fff) => format!("\
                The feature gate for the issue is `#![feature({FFF})]`.\
                ", FFF=fff),
            FeatureName::Uninitialized =>
                panic!("should have called extract_feature_name_from_rfc_text already."),
        };

        format!(r#"
This is a tracking issue for the RFC "{XXX}" (rust-lang/rfcs#{NNN}).
{FEATURE_LINE}

### About tracking issues

Tracking issues are used to record the overall progress of implementation.
They are also used as hubs connecting to other relevant issues, e.g., bugs or open design questions.
A tracking issue is however *not* meant for large scale discussion, questions, or bug reports about a feature.
Instead, open a dedicated issue for the specific matter and add the relevant feature gate label.

### Steps
<!--
Include each step required to complete the feature. Typically this is a PR
implementing a feature, followed by a PR that stabilises the feature. However
for larger features an implementation could be broken up into multiple PRs.
-->

- [ ] Implement the RFC (cc @rust-lang/{XXX} -- can anyone write up mentoring
      instructions?)
- [ ] Adjust documentation ([see instructions on rustc-dev-guide][doc-guide])
- [ ] Stabilization PR ([see instructions on rustc-dev-guide][stabilization-guide])

[stabilization-guide]: https://rustc-dev-guide.rust-lang.org/stabilization_guide.html#stabilization-pr
[doc-guide]: https://rustc-dev-guide.rust-lang.org/stabilization_guide.html#documentation-prs

### Unresolved Questions
<!--
Include any open questions that need to be answered before the feature can be
stabilised.
-->

XXX --- list all the "unresolved questions" found in the RFC to ensure they are
not forgotten

### Implementation history

<!--
Include a list of all the PRs that were involved in implementing the feature.
-->

"#,
                XXX=rfc_issue.title,
                NNN=rfc_issue.number,
                FEATURE_LINE=feature_line,
        )
    }
}
