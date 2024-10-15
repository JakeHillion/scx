// Copyright (c) Meta Platforms, Inc. and affiliates.

// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.
pub mod runner_common;
pub mod target_common;

pub use builder::Builder;

mod builder;

use serde::Deserialize;

use std::collections::HashMap;

const fn default_u32<const V: u32>() -> u32 {
    V
}

#[derive(Deserialize)]
pub struct TestConfig {
    pub topology: Topology,
    pub workload: Workload,
    pub scheduler: Scheduler,
    pub cases: HashMap<String, Case>,
}

#[derive(Deserialize)]
pub struct Topology {
    #[serde(default = "default_u32::<1>")]
    pub sockets: u32,
    #[serde(default = "default_u32::<1>")]
    pub llcs_per_socket: u32,
    #[serde(default = "default_u32::<4>")]
    pub cores_per_llc: u32,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum Workload {
    #[serde(rename = "stress-ng")]
    StressNg { args: Vec<String> },
}

#[derive(Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
pub enum Scheduler {
    Layered {
        #[serde(default)]
        args: Vec<String>,

        config: scx_layered::LayerConfig,
    },
}

#[derive(Deserialize)]
pub struct Case {
    /// Time to delay the test for after starting the scheduler and workload.
    #[serde(default = "default_u32::<5>")]
    pub delay_s: u32,

    #[serde(flatten)]
    pub test: CaseTest,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum CaseTest {
    Bpftrace { script: String, expect_json: String },
}

fn generate_workload(workload: &Workload) -> String {
    match workload {
        Workload::StressNg { args } => {
            let mut codegen = String::new();

            codegen.push_str("  let args = vec![ ");
            for arg in args {
                codegen.push('"');
                for c in arg.chars() {
                    if c == '"' {
                        codegen.push('\\');
                    }
                    codegen.push(c);
                }
                codegen.push_str("\", ");
            }
            codegen.push_str("];\n");

            // TODO: replace stress-ng with something that isn't constant, maybe an env var? it
            // depends how we do the mounting in the VM really.
            codegen.push_str("  let mut c = std::process::Command::new(\"stress-ng\");\n");
            codegen.push_str("  c.args(args);\n");
            codegen.push_str("  Ok(target_common::WorkloadHandle::StressNg(target_common::StressNgWorkload::new(c).expect(\"failed to start stress-ng\")))\n");

            codegen
        }
    }
}

/// Generate target, which will go in their crate/main.rs.
pub fn generate_target(cfgs: &[(String, TestConfig)]) -> anyhow::Result<String> {
    let mut codegen = String::new();

    codegen.push_str("use scx_integration_test_framework::target_common;\n\n");

    // Generate setup functions
    for (suite_name, cfg) in cfgs {
        codegen.push_str(&format!("fn setup_{}() -> anyhow::Result<target_common::WorkloadHandle> {{\n", suite_name));
        codegen.push_str(&generate_workload(&cfg.workload));
        codegen.push_str("}\n\n");
    }

    // Generate per-case targets
    for (suite_name, cfg) in cfgs {
        for (case_name, case) in &cfg.cases {
            let case_full_name = format!("{}_{}", suite_name, case_name);
            codegen.push_str(&format!("fn {}_target() {{\n", case_full_name));

            codegen.push_str(&format!("  let _workload_guard = setup_{}().expect(\"failed to run setup\");\n", suite_name));
            // TODO: start scheduler
            codegen.push_str("println!(\"workload & scheduler started, sleeping while it warms up\");\n");
            codegen.push_str(&format!("  std::thread::sleep(std::time::Duration::from_secs({}));\n", case.delay_s));

            codegen.push_str("}\n");
        }
    }

    codegen.push('\n');

    // Generate main function distributing by argument
    codegen.push_str("fn main() -> anyhow::Result<()> {\n");
    codegen.push_str("  let arg = std::env::args().nth(1).expect(\"case argument required\");\n");
    codegen.push_str("  match arg.as_str() {\n");
    for (suite_name, cfg) in cfgs {
        for (case_name, _) in &cfg.cases {
            let case_full_name = format!("{}_{}", suite_name, case_name);
            codegen.push_str(&format!("    \"{0}\" => {0}_target(),\n", case_full_name));
        }
    }
    codegen.push_str("    &_ => anyhow::bail!(\"invalid case name: {}\", arg.as_str()),\n");
    codegen.push_str("  }\n  Ok(())\n}\n");

    Ok(codegen)
}

/// Generate runner, which will go in their crate/test/integration.rs with a module per suite (toml
/// file).
pub fn generate_runner(suite_name: &str, cfg: &TestConfig) -> anyhow::Result<String> {
    let mut codegen = String::new();

    codegen.push_str(&format!("mod {} {{", suite_name));
    codegen.push_str("use scx_integration_test_framework::runner_common;\n\n");

    // Constants
    // TODO: we need to provide the substring as an argument somehow
    const TARGET_BINARY_NAME: &str = "scx_layered_tests";
    codegen.push_str(&format!("const TARGET_BINARY: &str = env!(\"CARGO_BIN_EXE_{}\");\n", TARGET_BINARY_NAME));

    // Generate runner function for full suite
    codegen.push_str("fn run() -> anyhow::Result<()> {\n");
    // TODO: setup VM and run the `target` in it, returning the result which probably needs to be
    // an enum
    codegen.push_str("  todo!(\"run\")\n");
    codegen.push_str("}\n\n");

    // Generate test cases
    for (case_name, case) in &cfg.cases {
        codegen.push_str("#[test]\n");

        codegen.push_str(&format!("fn {}() {{\n", case_name));

        // codegen.push_str("  let _ = setup();\n");

        // TODO: should start the test in a VM to be able to respect topology/have a clean shutdown
        // codegen.push_str("  todo!(\"start a VM and execute the target in it\");\n");
        // codegen.push_str("  todo!(\"parse and assert the results from the target\");\n");

        let case_full_name = format!("{}_{}", suite_name, case_name);
        codegen.push_str(&format!("  let child = std::process::Command::new(TARGET_BINARY).arg(\"{}\").spawn().expect(\"failed to spawn test target\").wait();\n", case_full_name));

        codegen.push_str("}\n\n");
    }

    codegen.push_str("}\n");
    Ok(codegen)
}
