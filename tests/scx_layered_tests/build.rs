// Copyright (c) Meta Platforms, Inc. and affiliates.
//
// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.

fn main() -> anyhow::Result<()> {
    scx_integration_test_framework::Builder::new()?
        .register_test_dir(&std::path::PathBuf::from("tests"))?
        .enable_runner("generated_tests.rs".into())?
        .enable_target("target.rs".into())?
        .build()?;

    Ok(())
}
