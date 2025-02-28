// Copyright (c) Meta Platforms, Inc. and affiliates.
//
// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.

use scx_utils::compat;
use scxtop::bpf_intf::*;
use scxtop::bpf_skel::types::bpf_event;
use scxtop::bpf_skel::*;
use scxtop::cli::{generate_completions, Cli, Commands, TuiArgs};
use scxtop::config::get_config_path;
use scxtop::config::Config;
use scxtop::edm::{BpfEventActionPublisher, BpfEventHandler, EventDispatchManager};
use scxtop::read_file_string;
use scxtop::App;
use scxtop::Event;
use scxtop::Key;
use scxtop::KeyMap;
use scxtop::PerfEvent;
use scxtop::Tui;
use scxtop::APP;
use scxtop::SCHED_NAME_PATH;
use scxtop::STATS_SOCKET_PATH;
use scxtop::{
    Action, IPIAction, SchedCpuPerfSetAction, SchedSwitchAction, SchedWakeupAction,
    SchedWakingAction, SoftIRQAction,
};

use anyhow::anyhow;
use anyhow::Result;
use clap::{CommandFactory, Parser};
use libbpf_rs::skel::OpenSkel;
use libbpf_rs::skel::SkelBuilder;
use libbpf_rs::ProgramInput;
use libbpf_rs::RingBufferBuilder;
use libbpf_rs::UprobeOpts;
use ratatui::crossterm::event::KeyCode::Char;
use simplelog::{LevelFilter, WriteLogger};
use tokio::sync::mpsc;

use std::fs;
use std::fs::File;
use std::mem::MaybeUninit;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::Duration;

fn get_action(_app: &App, keymap: &KeyMap, event: Event) -> Action {
    match event {
        Event::Error => Action::None,
        Event::Tick => Action::Tick,
        Event::TickRateChange(tick_rate_ms) => {
            Action::TickRateChange(std::time::Duration::from_millis(tick_rate_ms))
        }
        Event::Key(key) => match key.code {
            Char(c) => keymap.action(&Key::Char(c)),
            _ => keymap.action(&Key::Code(key.code)),
        },
        _ => Action::None,
    }
}

fn run_tui(tui_args: &TuiArgs) -> Result<()> {
    if let Ok(log_path) = std::env::var("RUST_LOG_PATH") {
        let log_level = match std::env::var("RUST_LOG") {
            Ok(v) => LevelFilter::from_str(&v)?,
            Err(_) => LevelFilter::Info,
        };

        WriteLogger::init(
            log_level,
            simplelog::Config::default(),
            File::create(log_path)?,
        )?;

        log_panics::Config::new()
            .backtrace_mode(log_panics::BacktraceMode::Resolved)
            .install_panic_hook();
    };

    let config = Config::merge([
        Config::from(tui_args.clone()),
        Config::load().unwrap_or(Config::default_config()),
    ]);
    let keymap = config.active_keymap.clone();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(config.worker_threads() as usize)
        .build()
        .unwrap()
        .block_on(async {
            let (action_tx, mut action_rx) = mpsc::unbounded_channel();

            let mut open_object = MaybeUninit::uninit();
            let mut builder = BpfSkelBuilder::default();
            if config.debug() {
                builder.obj_builder.debug(true);
            }
            let bpf_publisher = BpfEventActionPublisher::new(action_tx.clone());
            let mut edm = EventDispatchManager::new(None, None);
            edm.register_bpf_handler(Box::new(bpf_publisher));

            let skel = builder.open(&mut open_object)?;
            skel.maps.rodata_data.long_tail_tracing_min_latency_ns =
                tui_args.experimental_long_tail_tracing_min_latency_ns;

            let skel = skel.load()?;

            skel.progs.scxtop_init.test_run(ProgramInput::default())?;

            // Attach probes
            let mut links = vec![
                skel.progs.on_sched_cpu_perf.attach()?,
                skel.progs.scx_sched_reg.attach()?,
                skel.progs.scx_sched_unreg.attach()?,
                skel.progs.on_sched_switch.attach()?,
                skel.progs.on_sched_wakeup.attach()?,
            ];

            // 6.13 compatability
            if let Ok(link) = skel.progs.scx_insert_vtime.attach() {
                links.push(link);
            }
            if let Ok(link) = skel.progs.scx_dispatch_vtime.attach() {
                links.push(link);
            }
            if let Ok(link) = skel.progs.scx_insert.attach() {
                links.push(link);
            }
            if let Ok(link) = skel.progs.scx_dispatch.attach() {
                links.push(link);
            }
            if let Ok(link) = skel.progs.scx_dispatch_from_dsq_set_vtime.attach() {
                links.push(link);
            }
            if let Ok(link) = skel.progs.scx_dsq_move_set_vtime.attach() {
                links.push(link);
            }
            if let Ok(link) = skel.progs.scx_dsq_move_set_slice.attach() {
                links.push(link);
            }
            if let Ok(link) = skel.progs.scx_dispatch_from_dsq_set_slice.attach() {
                links.push(link);
            }
            if let Ok(link) = skel.progs.scx_dispatch_from_dsq.attach() {
                links.push(link);
            }
            if let Ok(link) = skel.progs.scx_dsq_move.attach() {
                links.push(link);
            }
            if let Ok(link) = skel.progs.on_cpuhp_enter.attach() {
                links.push(link);
            }
            if compat::ksym_exists("gpu_memory_total").is_ok() {
                if let Ok(link) = skel.progs.on_gpu_memory_total.attach() {
                    links.push(link);
                }
            }

            if tui_args.experimental_long_tail_tracing {
                let binary = tui_args
                    .experimental_long_tail_tracing_binary
                    .clone()
                    .unwrap();
                let symbol = tui_args
                    .experimental_long_tail_tracing_symbol
                    .clone()
                    .unwrap();

                links.extend([
                    skel.progs.long_tail_tracker_exit.attach_uprobe_with_opts(
                        -1, /* pid, -1 == all */
                        binary.clone(),
                        0,
                        UprobeOpts {
                            retprobe: true,
                            func_name: symbol.clone(),
                            ..Default::default()
                        },
                    )?,
                    skel.progs.long_tail_tracker_entry.attach_uprobe_with_opts(
                        -1, /* pid, -1 == all */
                        binary.clone(),
                        0,
                        UprobeOpts {
                            retprobe: false,
                            func_name: symbol.clone(),
                            ..Default::default()
                        },
                    )?,
                ]);
            };

            let mut tui = Tui::new(keymap.clone(), config.tick_rate_ms())?;
            let mut event_rbb = RingBufferBuilder::new();
            let event_handler = move |data: &[u8]| {
                let mut event = bpf_event::default();
                plain::copy_from_bytes(&mut event, data).expect("Event data buffer was too short");
                let _ = edm.on_event(&event);
                0
            };
            event_rbb.add(&skel.maps.events, event_handler)?;
            let event_rb = event_rbb.build()?;
            let scheduler = read_file_string(SCHED_NAME_PATH).unwrap_or("".to_string());

            let mut app = App::new(
                config,
                scheduler,
                100,
                tui_args.process_id,
                action_tx.clone(),
                skel,
            )?;

            tui.enter()?;

            let shutdown = app.should_quit.clone();
            tokio::spawn(async move {
                loop {
                    let _ = event_rb.poll(Duration::from_millis(1));
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                }
            });

            loop {
                tokio::select! {
                    ev = tui.next() => {
                        let ev = ev?;
                        match ev {
                            Event::Quit => { action_tx.send(Action::Quit)?; },
                            Event::Tick => action_tx.send(Action::Tick)?,
                            Event::TickRateChange(tick_rate_ms) => action_tx.send(
                                Action::TickRateChange(std::time::Duration::from_millis(tick_rate_ms)),
                            )?,
                            Event::Render => {
                                if app.should_quit.load(Ordering::Relaxed) {
                                    break;
                                }
                                tui.draw(|f| app.render(f).expect("Failed to render application"))?;
                            }
                            Event::Key(_) => {
                                let action = get_action(&app, &keymap, ev);
                                action_tx.send(action)?;
                            }
                            _ => {}
                    }}

                    ac = action_rx.recv() => {
                        let ac = ac.ok_or(anyhow!("actions channel closed"))?;
                        app.handle_action(&ac)?;
                    }
                }
            }
            tui.exit()?;
            drop(links);

            Ok(())
        })
}

fn main() -> Result<()> {
    let args = Cli::parse();

    match &args.command.unwrap_or(Commands::Tui(args.tui)) {
        Commands::Tui(tui_args) => {
            run_tui(tui_args)?;
        }
        Commands::GenerateCompletions { shell, output } => {
            generate_completions(Cli::command(), *shell, output.clone())
                .unwrap_or_else(|_| panic!("Failed to generate completions for {}", shell));
        }
    }
    Ok(())
}
