// Copyright (c) Meta Platforms, Inc. and affiliates.

// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.
pub use libc;
pub use tempfile;

use anyhow::Context;

use std::os::unix::process::ExitStatusExt;

pub enum WorkloadHandle {
    StressNg(StressNgWorkload),
}

pub struct SchedulerHandle {
    child: std::process::Child,
}

pub struct StressNgWorkload {
    child: std::process::Child,
}

fn exit_status_to_error(ec: std::process::ExitStatus) -> anyhow::Result<()> {
    if ec.signal() == Some(15) /* SIGTERM */ || ec.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("bad exit code: {}", ec))
    }
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    frequency: std::time::Duration,
    attempts: u16,
) -> anyhow::Result<()> {
    for _ in 0..attempts {
        if let Some(ec) = child.try_wait().unwrap() {
            return exit_status_to_error(ec);
        }
        std::thread::sleep(frequency);
    }

    Err(anyhow::anyhow!("failed to kill stress-ng process"))
}

fn send_sigterm(child: &std::process::Child) -> std::io::Result<()> {
    if unsafe { libc::kill(child.id().try_into().unwrap(), libc::SIGTERM) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

impl WorkloadHandle {
    pub fn cleanup(self) -> anyhow::Result<()> {
        match self {
            WorkloadHandle::StressNg(w) => w.cleanup(),
        }
    }

    pub fn is_alive(&mut self) -> anyhow::Result<bool> {
        match self {
            WorkloadHandle::StressNg(w) => w.is_alive(),
        }
    }
}

impl StressNgWorkload {
    pub fn new(mut cmd: std::process::Command) -> anyhow::Result<StressNgWorkload> {
        let child = cmd.spawn()?;
        Ok(StressNgWorkload { child })
    }

    pub fn cleanup(mut self) -> anyhow::Result<()> {
        self.cleanup_impl()
    }

    pub fn is_alive(&mut self) -> anyhow::Result<bool> {
        Ok(self.child.try_wait()?.is_none())
    }

    fn cleanup_impl(&mut self) -> anyhow::Result<()> {
        if let Some(ec) = self.child.try_wait()? {
            return exit_status_to_error(ec).with_context(|| "workload terminated early");
        }

        send_sigterm(&self.child)?;
        wait_with_timeout(&mut self.child, std::time::Duration::from_millis(100), 10)
            .with_context(|| "workload failed to exit cleanly after SIGTERM sent")
    }
}

impl Drop for StressNgWorkload {
    fn drop(&mut self) {
        // prefer calling cleanup as panicking in Drop is not ideal
        self.cleanup_impl().unwrap();
    }
}

impl SchedulerHandle {
    pub fn new(child: std::process::Child) -> Self {
        Self { child }
    }

    pub fn cleanup(mut self) -> anyhow::Result<()> {
        self.cleanup_impl()
    }

    pub fn cleanup_impl(&mut self) -> anyhow::Result<()> {
        if let Some(ec) = self.child.try_wait()? {
            return exit_status_to_error(ec).with_context(|| "scheduler terminated early");
        }

        send_sigterm(&self.child)?;
        wait_with_timeout(&mut self.child, std::time::Duration::from_millis(100), 10)
            .with_context(|| "scheduler failed to exit cleanly after SIGTERM sent")
    }

    pub fn is_alive(&mut self) -> anyhow::Result<bool> {
        Ok(self.child.try_wait()?.is_none())
    }
}

impl Drop for SchedulerHandle {
    fn drop(&mut self) {
        // prefer calling cleanup as panicking in Drop is not ideal
        self.cleanup_impl().unwrap();
    }
}
