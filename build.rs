use std::{env, error::Error};
use vergen_gitcl::{BuildBuilder, Emitter, GitclBuilder};

pub fn run() -> Result<(), Box<dyn Error>> {
    // describe with tags=true so that v0.4.0 gets picked up (not annotated)
    let git = GitclBuilder::default()
        .all()
        .describe(true, true, None)
        .build()?;
    let mut build_builder = BuildBuilder::default();
    let profile = env::var("PROFILE").unwrap_or_default();
    let is_release = profile == "release";
    // In dev/test builds keep metadata stable so incremental builds stick.
    if !is_release && env::var_os("SURFER_ENABLE_BUILD_DATE").is_none() {
        env::set_var("VERGEN_BUILD_DATE", "VERGEN_IDEMPOTENT_OUTPUT");
    }
    build_builder.build_date(true);
    if !is_release && env::var_os("SURFER_ENABLE_BUILD_TIMESTAMP").is_none() {
        env::set_var("VERGEN_BUILD_TIMESTAMP", "VERGEN_IDEMPOTENT_OUTPUT");
    }
    build_builder.build_timestamp(true);
    let build = build_builder.build()?;
    Emitter::default()
        .add_instructions(&build)?
        .add_instructions(&git)?
        .emit()?;
    Ok(())
}

#[allow(dead_code)]
fn main() {
    if let Err(err) = run() {
        panic!("build script failed: {err}");
    }
}
