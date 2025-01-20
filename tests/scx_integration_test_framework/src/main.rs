// Copyright (c) Meta Platforms, Inc. and affiliates.

// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.
use anyhow::bail;
use clap::{Parser, Subcommand};

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use scx_integration_test_framework::TestConfig;

#[derive(Parser)]
#[command(verbatim_doc_comment)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(subcommand)]
    Generate(Generate),
}

#[derive(Subcommand)]
enum Generate {
    Target {
        paths: Vec<PathBuf>,
    },
    Runner {
        /// TOML spec to build test cases from
        path: PathBuf,
    },
}

fn read_test_config_from_toml(path: &Path) -> anyhow::Result<TestConfig> {
    if !path.is_file() {
        bail!("path provided must be a filename!");
    }

    let content = std::fs::read_to_string(&path)?;
    Ok(toml::from_str(&content)?)
}

impl Generate {
    fn run(&self) -> anyhow::Result<()> {
        match self {
            Generate::Target { paths } => {
                let cfgs = paths
                    .iter()
                    .map(|p| {
                        let cfg = read_test_config_from_toml(p)?;
                        let suite_name = p.file_stem().unwrap().to_string_lossy().into_owned();
                        Ok((suite_name, cfg))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;

                let target = scx_integration_test_framework::generate_target(&cfgs)?;
                println!("{}", target);

                Ok(())
            }

            Generate::Runner { path } => {
                let cfg = read_test_config_from_toml(&path)?;
                let runner = scx_integration_test_framework::generate_runner(
                    &*path.file_stem().unwrap().to_string_lossy(),
                    &cfg,
                )?;
                println!("{}", runner);

                Ok(())
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let opts = Cli::parse();

    match opts.command {
        Commands::Generate(sub) => sub.run(),
    }
}
