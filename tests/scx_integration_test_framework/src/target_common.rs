// Copyright (c) Meta Platforms, Inc. and affiliates.

// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.
pub enum WorkloadHandle {
    StressNg(StressNgWorkload),
}

pub struct StressNgWorkload {
    child: std::process::Child,
}

impl StressNgWorkload {

pub fn new(mut cmd: std::process::Command) -> anyhow::Result<StressNgWorkload> {
    let child = cmd.spawn()?;
    Ok(StressNgWorkload{ child })
}

}

impl Drop for StressNgWorkload {
fn drop(&mut self) {
    println!("killing my child innit");
    self.child.kill().unwrap();
    for _ in 0..10 {
        if let Some(_ec) = self.child.try_wait().unwrap() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    eprintln!("failed to kill stress-ng process");
}
}
