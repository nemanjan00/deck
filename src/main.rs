//! deck — handheld ham-radio RX machine for SDR cyberdecks.

mod audio;
mod config;
mod device;
mod doctor;
mod dsp;
mod freq;
mod gui;
mod modes;
mod parse;
mod pipeline;
mod rec;
mod session;
mod sim;
mod sys;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "deck",
    version,
    about = "Handheld ham-radio RX machine: fullscreen SDR GUI wrapping dsd-neo, multimon-ng, dump1090 & friends",
    long_about = None
)]
struct Cli {
    /// alternative config file (default: ~/.config/deck/config.toml)
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// start windowed instead of fullscreen
    #[arg(long)]
    windowed: bool,

    /// skip the splash screen
    #[arg(long)]
    no_splash: bool,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Built-in signal simulator (IQ bands, decoder audio, decoded lines)
    Simgen(sim::SimArgs),
    /// Environment report: devices, tools, per-mode support
    Doctor {
        /// pipe sim signals through the installed decoders and verify
        #[arg(long)]
        selftest: bool,
    },
    /// Show config path; --write creates an annotated default file
    Config {
        #[arg(long)]
        write: bool,
    },
    /// Render UI screenshots headlessly (no GPU needed)
    Shot {
        /// output directory
        #[arg(long, default_value = "docs/shots")]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Some(Cmd::Simgen(args)) => sim::run(args),
        Some(Cmd::Doctor { selftest }) => {
            let (cfg, err) = config::load_config(cli.config.as_deref());
            if let Some(e) = err {
                eprintln!("warning: {e}");
            }
            print!("{}", doctor::report(&cfg));
            if selftest {
                print!("{}", doctor::selftest());
            }
            Ok(())
        }
        Some(Cmd::Config { write }) => {
            let path = config::config_path(cli.config.as_deref());
            if write {
                if path.exists() {
                    println!("config already exists at {}", path.display());
                } else {
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    std::fs::write(&path, config::default_config_toml())?;
                    println!("wrote {}", path.display());
                }
            } else {
                println!(
                    "config path: {} ({})",
                    path.display(),
                    if path.exists() { "exists" } else { "not created yet — `deck config --write`" }
                );
            }
            Ok(())
        }
        Some(Cmd::Shot { out }) => gui::shot::run(&out),
        None => run_gui(cli),
    }
}

fn run_gui(cli: Cli) -> Result<()> {
    let session = session::Session::new(cli.config.as_deref());
    let splash = session.cfg.ui.splash && !cli.no_splash;
    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title("deck")
        .with_app_id("deck")
        .with_inner_size([880.0, 520.0])
        .with_min_inner_size([320.0, 320.0])
        .with_fullscreen(!cli.windowed);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "deck",
        options,
        Box::new(move |_cc| Ok(Box::new(gui::DeckApp::new(session, splash)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI failed to start: {e}"))
}
