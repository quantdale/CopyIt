//! Run reporting: the JSONL event log, per-step visible-text snapshots, and the
//! failure bundle.
//!
//! Everything lands under the gitignored `sim-report/` directory at the repo
//! root — never inside the app's data locations. On success a run writes the
//! event log and per-step snapshots; on failure it additionally writes a
//! bundle: the failure detail, copies of the run's data files, the seed, and
//! an exact reproduction command.
//!
//! The event log is deliberately path-free and seed-deterministic: only the
//! step number, the intent, the resolved target, the input summary, the
//! virtual time, and a state digest — so two runs with the same seed produce
//! byte-identical normalized logs (the determinism test relies on that).

use crate::storage;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(feature = "sim")]
use eframe::egui;

/// One line of the event log.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EventRecord {
    pub step: usize,
    pub intent: String,
    pub target: String,
    pub input: String,
    pub time_ms: u64,
    pub state: String,
}

/// A summary of what one step changed on screen: every visible text of the
/// frame, in render order. Stored per step so failures can ship the last one.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub step: usize,
    pub visible: Vec<String>,
}

impl Snapshot {
    /// Human-readable rendering of the snapshot for per-step files.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "--- snapshot after step {} ---", self.step);
        for line in &self.visible {
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

/// The outcome of a completed (or failed) run, plus its report location.
pub struct Report {
    pub journey: String,
    pub seed: u64,
    /// Directory the run's throwaway store lives in (also used for the
    /// isolation guard).
    pub run_dir: PathBuf,
    /// Where this run's report files are written (`sim-report/<journey>-<seed>`).
    pub report_dir: PathBuf,
    /// Ship a copy of each data file into the bundle on failure.
    pub store: crate::store::Store,
    /// All events recorded so far, in order. Deterministic per (journey, seed).
    pub events: Vec<EventRecord>,
    /// The last snapshot taken; replaced as steps complete.
    pub last_snapshot: Option<Snapshot>,
}

impl Report {
    /// Creates the report, wiping any previous run with the same name+seed so a
    /// rerun never ships stale artifacts.
    pub fn new(journey: &str, seed: u64, run_dir: PathBuf, store: crate::store::Store) -> Self {
        let report_dir = PathBuf::from("sim-report").join(format!("{journey}-{seed}"));
        let _ = fs::remove_dir_all(&report_dir);
        let _ = fs::create_dir_all(report_dir.join("snapshots"));
        Report {
            journey: journey.to_string(),
            seed,
            run_dir,
            report_dir,
            store,
            events: Vec::new(),
            last_snapshot: None,
        }
    }

    /// Computes the state digest of the run's store: snippets count + file sizes.
    /// Deterministic for a given store content.
    pub fn state_digest(&self) -> String {
        let snippets = match storage::load(&self.store.snippets_path) {
            storage::Load::Loaded(s) => s.len(),
            _ => 0,
        };
        let snip_len = fs::metadata(&self.store.snippets_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let cfg_len = fs::metadata(&self.store.config_path)
            .map(|m| m.len())
            .unwrap_or(0);
        format!("{snippets} snippets, snippets.json {snip_len}B, config.json {cfg_len}B")
    }

    /// Appends an event to the log and writes the log line to disk.
    pub fn push_event(&mut self, record: EventRecord) -> io::Result<()> {
        self.events.push(record.clone());
        let mut line = serde_json::to_string(&record).unwrap_or_default();
        line.push('\n');
        append(&self.report_dir.join("event.log"), &line)
    }

    /// Writes a per-step snapshot file and keeps the last one in memory.
    pub fn write_snapshot(&mut self, snapshot: Snapshot) -> io::Result<()> {
        let rendered = snapshot.render();
        fs::write(self.report_dir.join("snapshots").join(format!("step-{:03}.txt", snapshot.step)), rendered)?;
        self.last_snapshot = Some(snapshot);
        Ok(())
    }

    /// Writes the pass summary.
    #[cfg(test)]
    pub fn write_summary(&self) -> io::Result<()> {
        fs::write(
            self.report_dir.join("summary.txt"),
            format!(
                "journey: {}\nseed: {}\nresult: pass\nsteps: {}\nrepro: cargo run --features sim -- --simulate {} --seed {}\n",
                self.journey,
                self.seed,
                self.events.len(),
                self.journey,
                self.seed
            ),
        )
    }

    /// Writes the failure bundle: failure detail, the last snapshot, copies of
    /// the data files (including any `.corrupt` backups), the seed, and the
    /// exact reproduction command.
    pub fn fail(&self, step: usize, message: &str) -> io::Result<()> {
        let dir = &self.report_dir;
        let mut detail = String::new();
        let _ = writeln!(detail, "journey: {}", self.journey);
        let _ = writeln!(detail, "seed: {}", self.seed);
        let _ = writeln!(detail, "failed at step: {}", step);
        let _ = writeln!(detail, "failure: {}", message);
        if let Some(snap) = &self.last_snapshot {
            let _ = write!(detail, "{}", snap.render());
        }
        let _ = writeln!(detail, "repro: cargo run --features sim -- --simulate {} --seed {}", self.journey, self.seed);
        let _ = writeln!(detail, "data dir: {}", self.run_dir.display());
        fs::write(dir.join("failure.log"), detail)?;
        fs::write(dir.join("seed.txt"), self.seed.to_string())?;
        fs::write(dir.join("REPRO.md"), repro_command(&self.journey, self.seed))?;

        // Copy the run's data files so the failure can be inspected (or replayed)
        // without needing the temp store.
        copy_if_exists(&self.store.snippets_path, &dir.join("snippets.json"))?;
        copy_if_exists(&self.store.config_path, &dir.join("config.json"))?;
        // Any `.corrupt` / `.corrupt.1` / `.corrupt.2` backups made by the app
        // also ship, so the failure bundle includes every backup variant.
        if let Some(parent) = self.store.snippets_path.parent() {
            if let Ok(entries) = fs::read_dir(parent) {
                for entry in entries.flatten() {
                    if let Some(fname) = entry.file_name().to_str() {
                        if fname.starts_with("snippets.json.corrupt") {
                            copy_if_exists(
                                &entry.path(),
                                &dir.join(fname),
                            )?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn append(path: &Path, contents: &str) -> io::Result<()> {
    use std::io::Write;
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(contents.as_bytes())
}

fn copy_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    if from.exists() {
        fs::copy(from, to)?;
    }
    Ok(())
}

fn repro_command(journey: &str, seed: u64) -> String {
    format!(
        "# Reproduce this failure\n\n\
         cargo run --features sim -- --simulate {journey} --seed {seed}\n\n\
         Or headlessly for the same event stream:\n\
         cargo test --bin copyit -- --nocapture\n"
    )
}

/// Writes a `ColorImage` to disk as a PNG using stored (uncompressed) deflate
/// blocks. Only used by headed mode (`feature = "sim"`); no external image crate
/// is needed, so `Cargo.toml` stays untouched.
#[cfg(feature = "sim")]
pub fn write_png(path: &Path, image: &egui::ColorImage) -> io::Result<()> {
    let [w, h] = image.size;
    let mut raw = Vec::with_capacity((w * 4 + 1) * h);
    for y in 0..h {
        raw.push(0); // filter: none
        for x in 0..w {
            let c = image.pixels[y * w + x];
            raw.extend_from_slice(&[c.r(), c.g(), c.b(), c.a()]);
        }
    }

    // zlib stream: 2-byte header, stored deflate blocks, 4-byte adler32.
    let mut deflate = Vec::new();
    deflate.extend_from_slice(&[0x78, 0x01]);
    let mut i = 0;
    while i < raw.len() {
        let chunk = (raw.len() - i).min(65535);
        let final_bit = if i + chunk >= raw.len() { 1 } else { 0 };
        deflate.push(final_bit);
        deflate.extend_from_slice(&(chunk as u16).to_le_bytes());
        deflate.extend_from_slice(&(!(chunk as u16)).to_le_bytes());
        deflate.extend_from_slice(&raw[i..i + chunk]);
        i += chunk;
    }
    deflate.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    png_chunk(&mut out, b"IHDR", &ihdr);
    png_chunk(&mut out, b"IDAT", &deflate);
    png_chunk(&mut out, b"IEND", &[]);
    fs::write(path, out)
}

#[cfg(feature = "sim")]
fn png_chunk(out: &mut Vec<u8>, tag: &[u8], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc = !0u32;
    for &b in tag.iter().chain(data.iter()) {
        crc = (crc >> 8) ^ CRC_TABLE[((crc ^ b as u32) & 0xff) as usize];
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

#[cfg(feature = "sim")]
fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

#[cfg(feature = "sim")]
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};
