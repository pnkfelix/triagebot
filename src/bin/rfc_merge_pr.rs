use triagebot::{github, rfc_merge_pr};

use anyhow::{Context};
use reqwest::{Client};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let oc = octocrab::OctocrabBuilder::new()
        .personal_token(github::default_token_from_env())
        .build()
        .expect("Failed to build octocrab");

    let f = "rfc_merge_pr::main";
    let arg: String = std::env::args().skip(1).next().unwrap_or_else(|| {
        panic!("{f} expected first argument, an RFC PR # to merge.", f=f);
    });
    let arg: u64 = arg.parse().unwrap_or_else(|e| {
        panic!("{f} expected numeric first argument, but it failed to parse; {e:?}", f=f, e=e);
    });

    rfc_merge_pr::merge(arg).await.unwrap_or_else(|e| {
        panic!("{f} failure during merge: {e:?}", f=f, e=e);
    });
}
