// Copyright (c) Meta Platforms, Inc. and affiliates.

// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.
pub use crate::Topology;

use std::collections::HashMap;
use std::sync::mpsc::channel;

use anyhow::Context;
use log::info;
use vmtest::output::Output as VmOutput;
use vmtest::vmtest::Vmtest;

pub fn run_target_in_vm(
    topo: &Topology,
    target_binary: std::path::PathBuf,
    test_case: &str,
) -> anyhow::Result<i64> {
    let cfg = vmtest::config::Config {
        target: vec![vmtest::config::Target {
            name: "scx_test_runner".into(),
            kernel: Some("/data/users/jakehillion/downloads/assets/bzImage".into()),
            command: format!("{} {}", target_binary.display(), test_case),
            vm: vmtest::config::VMConfig {
                memory: "4G".into(),
                // num_cpus: topo.num_cpus(),
                // extra_args: vec![format!(
                //     "-smp sockets={},modules={},cores={},threads={}",
                //     topo.sockets, topo.llcs_per_socket, topo.cores_per_llc, topo.threads_per_core
                // )],
                ..Default::default()
            },
            ..Default::default()
        }],
    };
    let vm = Vmtest::new("/", cfg)?;

    let (tx, rx) = channel();
    vm.run_one(0 /* target index */, tx);

    for msg in rx.into_iter() {
        match msg {
            VmOutput::BootStart => info!("boot started"),
            VmOutput::Boot(output) => info!("boot output: {}", output),
            VmOutput::BootEnd(res) => {
                res.context("failed to boot vm")?;
                info!("boot succeeded");
            }

            VmOutput::SetupStart => info!("setup started"),
            VmOutput::Setup(output) => info!("setup output: {}", output),
            VmOutput::SetupEnd(res) => {
                res.context("failed to setup vm")?;
                info!("setup succeeded");
            }

            VmOutput::CommandStart => info!("command started"),
            VmOutput::Command(output) => info!("command output: {}", output),
            VmOutput::CommandEnd(res) => {
                let exit_code = res?;
                info!("command succeeded");
                return Ok(exit_code);
            }
        }
    }

    unreachable!("loop terminated without sending exit message")
}

pub fn decode_topology(json: &str) -> anyhow::Result<Topology> {
    Ok(serde_json::from_str(json)?)
}
