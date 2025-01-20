// Copyright (c) Meta Platforms, Inc. and affiliates.

// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.
pub mod runner_common;
pub mod target_common;

pub use builder::Builder;

mod builder;

use serde::{Deserialize, Serialize};

use std::collections::HashMap;

const fn default_u8<const V: u8>() -> u8 {
    V
}
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

#[derive(Serialize, Deserialize)]
pub struct Topology {
    #[serde(default = "default_u8::<1>")]
    pub sockets: u8,
    #[serde(default = "default_u8::<1>")]
    pub llcs_per_socket: u8,
    #[serde(default = "default_u8::<2>")]
    pub cores_per_llc: u8,
    #[serde(default = "default_u8::<2>")]
    pub threads_per_core: u8,
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

impl Topology {
    pub(crate) fn num_cpus(&self) -> u8 {
        self.sockets * self.llcs_per_socket * self.cores_per_llc
    }
}

fn codegen_vec_of_strings(strings: &[String]) -> String {
    let mut codegen = String::new();

    codegen.push_str("vec![ ");
    for s in strings {
        codegen.push('"');
        for c in s.chars() {
            if c == '"' {
                codegen.push('\\');
            }
            codegen.push(c);
        }
        codegen.push_str("\", ");
    }
    codegen.push(']');

    codegen
}

fn generate_workload(workload: &Workload) -> String {
    match workload {
        Workload::StressNg { args } => {
            let mut codegen = String::new();

            codegen.push_str("  let args: Vec<&str> = ");
            codegen.push_str(&codegen_vec_of_strings(args));
            codegen.push_str(";\n");

            // TODO: replace stress-ng with something that isn't constant, maybe an env var? it
            // depends how we do the mounting in the VM really.
            codegen.push_str("  let mut c = std::process::Command::new(\"stress-ng\");\n");
            codegen.push_str("  c.args(args);\n");
            codegen.push_str("  Ok(target_common::WorkloadHandle::StressNg(target_common::StressNgWorkload::new(c).expect(\"failed to start stress-ng\")))\n");

            codegen
        }
    }
}

fn generate_scheduler(scheduler: &Scheduler) -> anyhow::Result<String> {
    match scheduler {
        Scheduler::Layered { args, config } => {
            // TODO: change me
            const LAYERED: &str = "/data/users/jakehillion/scx/target/release/scx_layered";

            let mut codegen = String::new();
            codegen.push_str("  use std::io::Write;\n");
            codegen.push_str("  use std::os::fd::AsRawFd;\n");

            codegen.push_str("  let args: Vec<&str> = ");
            codegen.push_str(&codegen_vec_of_strings(args));
            codegen.push_str(";\n");

            codegen.push_str("  let cfg = r#\"");
            codegen.push_str(&serde_json::to_string(config)?);
            codegen.push_str("\"#;\n");

            codegen.push_str("  let mut cfg_file = target_common::tempfile::tempfile()?;\n");
            codegen.push_str("  cfg_file.write_all(cfg.as_bytes())?;\n");

            codegen.push_str("  let child = std::process::Command::new(\"");
            codegen.push_str(LAYERED);
            codegen.push_str("\").args(args).arg(format!(\"f:/proc/{}/fd/{}\", unsafe { target_common::libc::getpid() }, cfg_file.as_raw_fd())).spawn()?;\n");

            // TODO: this is racy as anything - is there a better way to check the scheduler has
            // started? the scheduler has to start before the tempfile is dropped.
            codegen.push_str("  std::thread::sleep(std::time::Duration::from_millis(100));\n");

            codegen.push_str("  Ok(target_common::SchedulerHandle::new(child))\n");

            Ok(codegen)
        }
    }
}

/// Generate target, which will go in their crate/main.rs.
pub fn generate_target(cfgs: &[(String, TestConfig)]) -> anyhow::Result<String> {
    let mut codegen = String::new();

    codegen.push_str("use scx_integration_test_framework::target_common;\n\n");

    // Generate setup functions
    for (suite_name, cfg) in cfgs {
        codegen.push_str(&format!(
            "fn run_workload_{}() -> anyhow::Result<target_common::WorkloadHandle> {{\n",
            suite_name
        ));
        codegen.push_str(&generate_workload(&cfg.workload));
        codegen.push_str("}\n");

        codegen.push_str(&format!(
            "fn run_scheduler_{}() -> anyhow::Result<target_common::SchedulerHandle> {{\n",
            suite_name
        ));
        codegen.push_str(&generate_scheduler(&cfg.scheduler)?);
        codegen.push_str("}\n");
    }

    // Generate per-case targets
    for (suite_name, cfg) in cfgs {
        for (case_name, case) in &cfg.cases {
            let case_full_name = format!("{}_{}", suite_name, case_name);
            codegen.push_str(&format!("fn {}_target() {{\n", case_full_name));

            codegen.push_str(&format!(
                "  let mut workload = run_workload_{}().expect(\"failed to start workload\");\n",
                suite_name
            ));
            codegen.push_str(&format!(
                "  let mut scheduler = run_scheduler_{}().expect(\"failed to start workload\");\n",
                suite_name
            ));

            let delay_ms = std::cmp::max(case.delay_s * 1000, 100);
            codegen.push_str(&format!("  println!(\"workload & scheduler started, sleeping for {}ms while they warm up\");\n", delay_ms));
            codegen.push_str(&format!(
                "  std::thread::sleep(std::time::Duration::from_millis({}));\n",
                delay_ms
            ));

            codegen.push_str(
                "  assert!(workload.is_alive().unwrap(), \"workload stopped prematurely\");\n",
            );
            codegen.push_str(
                "  assert!(scheduler.is_alive().unwrap(), \"workload stopped prematurely\");\n",
            );

            // TODO: run test

            codegen.push_str("  workload.cleanup().expect(\"workload failed to clean up\");\n");
            codegen.push_str("  scheduler.cleanup().expect(\"scheduler failed to clean up\");\n");
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
    codegen.push_str("const TARGET_BINARY: &str = env!(concat!(\"CARGO_BIN_EXE_\", env!(\"CARGO_PKG_NAME\")));\n");

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

codegen.push_str("  let _ = env_logger::builder().is_test(true).try_init();\n");

        // codegen.push_str("  let _ = setup();\n");

        let case_full_name = format!("{}_{}", suite_name, case_name);
        // codegen.push_str(&format!("  let target_status = std::process::Command::new(TARGET_BINARY).arg(\"{}\").spawn().expect(\"failed to spawn test target\").wait().expect(\"failed to wait for test target\");\n", case_full_name));

        codegen.push_str("  let topo = runner_common::decode_topology(r#\"");
        codegen.push_str(&serde_json::to_string(&cfg.topology).unwrap());
        codegen.push_str("\"#).unwrap();\n");

        codegen.push_str(&format!("  let target_status = runner_common::run_target_in_vm(&topo, TARGET_BINARY.into(), \"{}\").unwrap();\n", case_full_name));
        // codegen.push_str("  assert!(target_status.success(), \"target failed with exit code {:?}\", target_status.code());\n");
        codegen.push_str("  assert!(target_status == 0, \"target failed with exit code {:?}\", target_status);\n");

        // codegen.push_str("  todo!(\"parse and assert the results from the target\");\n");

        codegen.push_str("}\n\n");
    }

    codegen.push_str("}\n");
    Ok(codegen)
}
