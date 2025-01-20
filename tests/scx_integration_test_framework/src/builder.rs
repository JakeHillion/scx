// Copyright (c) Meta Platforms, Inc. and affiliates.

// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.
use anyhow::bail;

use std::env;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub struct Builder {
    out_dir: PathBuf,
    target_filename: Option<PathBuf>,
    runner_filename: Option<PathBuf>,

    test_paths: Vec<PathBuf>,
}

fn read_test_config_from_toml(path: &Path) -> anyhow::Result<crate::TestConfig> {
    let content = std::fs::read_to_string(&path)?;
    Ok(toml::from_str(&content)?)
}

impl Builder {
    pub fn new() -> anyhow::Result<Builder> {
        let out_dir = PathBuf::from(env::var("OUT_DIR")?);
        Ok(Builder {
            out_dir,
            target_filename: None,
            runner_filename: None,

            test_paths: vec![],
        })
    }

    pub fn register_test(&mut self, path: PathBuf) -> &mut Self {
        println!("cargo::rerun-if-changed={}", path.display());
        self.test_paths.push(path);
        self
    }

    pub fn register_test_dir(&mut self, test_dir: &Path) -> anyhow::Result<&mut Self> {
        println!("cargo::rerun-if-changed={}", test_dir.display());

        let mut added = false;
        for entry in fs::read_dir(test_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && path.extension().is_some_and(|e| e == "toml") {
                self.test_paths.push(path);
                added = true;
            }
        }

        if !added {
            println!(
                "cargo::warning=Test directory `{}` provided doesn't contain any .toml files",
                test_dir.display()
            );
        }

        Ok(self)
    }

    pub fn enable_target(&mut self, path: PathBuf) -> anyhow::Result<&mut Self> {
        if !path.is_relative() {
            bail!("path supplied must be relative: `{}`", path.display());
        }
        self.target_filename = Some(path);
        Ok(self)
    }

    pub fn enable_runner(&mut self, path: PathBuf) -> anyhow::Result<&mut Self> {
        if !path.is_relative() {
            bail!("path supplied must be relative: `{}`", path.display());
        }
        self.runner_filename = Some(path);
        Ok(self)
    }

    pub fn build(&self) -> anyhow::Result<()> {
        let configs = self
            .test_paths
            .iter()
            .map(|p| {
                let cfg = read_test_config_from_toml(p)?;
                let suite_name = p.file_stem().unwrap().to_string_lossy().into_owned();
                Ok((suite_name, cfg))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        if let Some(target_path) = &self.target_filename {
            let src = crate::generate_target(&configs)?;

            let target_path = Path::join(&self.out_dir, target_path);
            fs::write(target_path, src)?;
        };
        if let Some(runner_path) = &self.runner_filename {
            let runner_path = Path::join(&self.out_dir, runner_path);

            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(runner_path)?;
            for (suite_name, cfg) in &configs {
                let src = crate::generate_runner(suite_name, cfg)?;
                f.write_all(src.as_bytes())?;
            }
        };

        Ok(())
    }
}
