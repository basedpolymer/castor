//! Keeps MuPDF viewing (thumbnails, PDF Studio pages) and PDF-engine tool jobs apart.
//!
//! engine-pdf's C helpers (image rewriting in `pdf.compress`, OCR…) create MuPDF contexts
//! without locks; when another thread renders with MuPDF at the same time the process can crash
//! (access violation, reproduced by `studio::e2e_tests::thumbnail_during_compress`). Until the
//! engine is fixed (F-studios.md, requests to the coordinator), the app never lets the two
//! groups overlap. Jobs of the same group still run in parallel.

use std::sync::Arc;

use castor_core::runner::{RunHandle, RunRequest, Step};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, Values};
use parking_lot::{Condvar, Mutex};

#[derive(Clone, Copy)]
pub enum Group {
    /// Rendering, text extraction (mupdf-rs).
    View = 0,
    /// Tool jobs that may call engine-pdf's C helpers.
    Job = 1,
}

static COUNTS: Mutex<[usize; 2]> = Mutex::new([0, 0]);
static CHANGED: Condvar = Condvar::new();

pub struct Guard(Group);

/// Wait until the other group is idle, then enter `g`.
pub fn enter(g: Group) -> Guard {
    let (me, other) = (g as usize, 1 - g as usize);
    let mut c = COUNTS.lock();
    while c[other] > 0 {
        CHANGED.wait(&mut c);
    }
    c[me] += 1;
    Guard(g)
}

/// Enter `g` only if the other group is idle (never blocks).
pub fn try_enter(g: Group) -> Option<Guard> {
    let (me, other) = (g as usize, 1 - g as usize);
    let mut c = COUNTS.lock();
    if c[other] > 0 {
        return None;
    }
    c[me] += 1;
    Some(Guard(g))
}

/// Enter `View` without blocking an executor thread: polls every 250 ms.
pub async fn enter_view(ex: &gpui_kit::BackgroundExecutor) -> Guard {
    loop {
        if let Some(g) = try_enter(Group::View) {
            return g;
        }
        ex.timer(std::time::Duration::from_millis(250)).await;
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        COUNTS.lock()[self.0 as usize] -= 1;
        CHANGED.notify_all();
    }
}

/// Does this tool use engine-pdf's C helpers?
pub fn uses_pdf_engine(id: &str) -> bool {
    id.starts_with("pdf.") || matches!(id, "image.to-pdf" | "image.images-to-pdf" | "image.extract-text")
}

/// A tool whose jobs enter the `Job` group.
struct Gated(Arc<dyn Tool>);

impl Tool for Gated {
    fn meta(&self) -> &ToolMeta {
        self.0.meta()
    }
    fn options(&self) -> Vec<castor_core::OptionSpec> {
        self.0.options()
    }
    fn plan(&self, inputs: &[FileInfo], values: &Values) -> castor_core::Result<Vec<Job>> {
        self.0.plan(inputs, values)
    }
    fn run(&self, job: &Job, values: &Values, cx: &RunCx) -> castor_core::Result<Vec<Output>> {
        let _g = enter(Group::Job);
        self.0.run(job, values, cx)
    }
    fn undo(&self, journal: &serde_json::Value) -> castor_core::Result<()> {
        self.0.undo(journal)
    }
    fn preview(&self, input: Option<&FileInfo>, values: &Values, max_side: u32) -> castor_core::Result<Option<Preview>> {
        self.0.preview(input, values, max_side)
    }
    fn downloads(&self, values: &Values) -> Vec<castor_core::download::Asset> {
        self.0.downloads(values)
    }
    fn unavailable(&self) -> Option<String> {
        self.0.unavailable()
    }
}

/// `runner::start` with PDF-engine steps gated.
pub fn start_run(mut req: RunRequest) -> RunHandle {
    req.steps = req
        .steps
        .into_iter()
        .map(|s| if uses_pdf_engine(s.tool.meta().id) { Step { tool: Arc::new(Gated(s.tool)), values: s.values } } else { s })
        .collect();
    castor_core::runner::start(req)
}

#[cfg(test)]
mod tests {
    use super::{Group, enter};

    #[test]
    fn groups_exclude_each_other() {
        let g = enter(Group::View);
        let g2 = enter(Group::View);
        let t = std::thread::spawn(|| {
            let start = std::time::Instant::now();
            let _j = enter(Group::Job);
            start.elapsed()
        });
        std::thread::sleep(std::time::Duration::from_millis(150));
        drop(g);
        drop(g2);
        assert!(t.join().unwrap() >= std::time::Duration::from_millis(100));
    }
}
