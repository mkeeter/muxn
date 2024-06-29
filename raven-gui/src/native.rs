use anyhow::{anyhow, Context};
use std::{io::Read, sync::mpsc};

use uxn::{JitRam, Uxn, UxnJit, UxnVm, VmRam};
use varvara::Varvara;

use anyhow::Result;
use eframe::egui;
use log::info;

use clap::Parser;

use crate::{audio_setup, Stage};

/// Uxn runner
#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// ROM to load and execute
    rom: std::path::PathBuf,

    /// Use the JIT-accelerated Uxn implementation
    #[clap(long)]
    jit: bool,

    /// Arguments to pass into the VM
    #[arg(last = true)]
    args: Vec<String>,
}

pub fn run_uxn<U: Uxn + 'static>(mut vm: U, args: &[String]) -> Result<()> {
    let mut dev = Varvara::new();

    let _audio = audio_setup(dev.audio_streams());

    // Run the reset vector
    let start = std::time::Instant::now();
    vm.run(&mut dev, 0x100);
    info!("startup complete in {:?}", start.elapsed());

    dev.output(&vm).check()?;
    dev.send_args(&mut vm, args).check()?;

    let (width, height) = dev.output(&vm).size;
    let options = eframe::NativeOptions {
        window_builder: Some(Box::new(move |v| {
            v.with_inner_size(egui::Vec2::new(width as f32, height as f32))
                .with_resizable(false)
        })),
        ..Default::default()
    };

    let (_tx, rx) = mpsc::channel();
    eframe::run_native(
        "Varvara",
        options,
        Box::new(move |cc| Box::new(Stage::new(vm, dev, rx, &cc.egui_ctx))),
    )
    .map_err(|e| anyhow!("got egui error: {e:?}"))
}

pub fn run() -> Result<()> {
    let env = env_logger::Env::default()
        .filter_or("UXN_LOG", "info")
        .write_style_or("UXN_LOG", "always");
    env_logger::init_from_env(env);

    let args = Args::parse();
    let mut f = std::fs::File::open(&args.rom)
        .with_context(|| format!("failed to open {:?}", args.rom))?;

    let mut rom = vec![];
    f.read_to_end(&mut rom).context("failed to read file")?;

    if args.jit {
        let ram = JitRam::new();
        let vm = UxnJit::new(&rom, ram.leak());
        run_uxn(vm, &args.args)
    } else {
        let ram = VmRam::new();
        let vm = UxnVm::new(&rom, ram.leak());
        run_uxn(vm, &args.args)
    }
}
