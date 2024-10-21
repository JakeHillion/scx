// Copyright (c) Meta Platforms, Inc. and affiliates.

// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.
use vmtest::vmtest::Vmtest;

use crate::Topology;

use std::sync::mpsc::channel;
use std::path::PathBuf;
use std::collections::HashMap;

pub fn run_target_in_vm(topo: &Topology, target_binary: std::path::PathBuf, test_case: &str) -> anyhow::Result<std::process::ExitStatus> {
    let cfg = vmtest::config::Config {
        target: vec![vmtest::config::Target {
            name: "TBD".into(),            
            image: None,
            uefi: false,
            kernel: Some("/data/users/jakehillion/linux/arch/x86/boot/bzImage".into()),
            kernel_args: None,
            rootfs: "TBD".into(),
            arch: "x86_64".into(),
            command: "TBD".into(),
            vm: vmtest::config::VMConfig {
                num_cpus: topo.num_cpus(),
                memory: "4G".into(),
                mounts: HashMap::new(),
                bios: None,
                extra_args: vec![
                    format!("-smp sockets={},modules={},cores={},threads={}", topo.sockets, topo.llcs_per_socket, topo.cores_per_llc, topo.threads_per_core),
                ],
            },
        }],
    };
    let vm = Vmtest::new("/", cfg)?;

    let (tx, rx) = channel();
    vm.run_one(0, tx);

    todo!("run_test_in_vm");
}
