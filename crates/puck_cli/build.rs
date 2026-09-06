use std::env;

fn main() {
    let version = env::var("PUCK_VERSION").unwrap_or_else(|_| {
        env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into())
    });
    let git_sha = env::var("PUCK_GIT_SHA").unwrap_or_else(|_| "dev".into());
    let build_date = env::var("PUCK_BUILD_DATE").unwrap_or_else(|_| "unknown".into());

    // clap prints: `puck <version>` — embed `0.1.0 (<sha> <date>)`.
    println!(
        "cargo:rustc-env=PUCK_FULL_VERSION={version} ({git_sha} {build_date})"
    );
    println!("cargo:rerun-if-env-changed=PUCK_VERSION");
    println!("cargo:rerun-if-env-changed=PUCK_GIT_SHA");
    println!("cargo:rerun-if-env-changed=PUCK_BUILD_DATE");
    println!("cargo:rerun-if-changed=../../dist/minisign/minisign.pub");
}
