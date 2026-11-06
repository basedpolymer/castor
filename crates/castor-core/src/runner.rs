//! Runs a tool (or a chain of tools) on a batch of files, on worker threads.
//!
//! The UI starts a run with [`start`] and drains [`RunHandle::events`] on its own executor;
//! the CLI uses [`run_blocking`]. Every finished run is appended to the history.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::files::FileInfo;
use crate::history::{self, RunRecord, RunStatus};
use crate::job::{Cancelled, CancelToken, Job, Output, RunCx};
use crate::options::Values;
use crate::tool::Tool;

/// Where results are written.
#[derive(Clone, Debug)]
pub enum OutputPolicy {
    /// Next to each input file (default). Jobs without inputs go to `fallback`.
    NextToInput { fallback: PathBuf },
    /// Everything in this folder (created if needed).
    Folder(PathBuf),
}

impl OutputPolicy {
    fn dir_for(&self, job: &Job) -> PathBuf {
        match self {
            OutputPolicy::Folder(p) => p.clone(),
            OutputPolicy::NextToInput { fallback } => job
                .inputs
                .first()
                .and_then(|f| if f.is_dir { Some(f.path.clone()) } else { f.path.parent().map(|p| p.to_path_buf()) })
                .unwrap_or_else(|| fallback.clone()),
        }
    }
}

/// One step of a run: a tool and its option values.
#[derive(Clone)]
pub struct Step {
    pub tool: Arc<dyn Tool>,
    pub values: Values,
}

pub struct RunRequest {
    /// One step = a plain tool run; several = a chain (outputs of step k feed step k+1).
    pub steps: Vec<Step>,
    pub inputs: Vec<FileInfo>,
    pub output: OutputPolicy,
    /// Name shown in history for chains (tool runs use the tool id).
    pub chain_name: Option<String>,
    /// Max parallel jobs (0 = automatic).
    pub max_parallel: usize,
    /// Append to the runs history.
    pub record: bool,
}

#[derive(Clone, Debug)]
pub enum RunEvent {
    /// A step is starting. `jobs` = labels of its jobs, in order.
    StepStarted { step: usize, steps: usize, tool_id: String, jobs: Vec<String> },
    JobStarted { step: usize, job: usize },
    JobProgress { step: usize, job: usize, frac: f32 },
    JobMessage { step: usize, job: usize, msg: String },
    JobDone { step: usize, job: usize, outputs: Vec<Output>, bytes_in: u64, bytes_out: u64, secs: f32 },
    JobFailed { step: usize, job: usize, error: String },
    JobCancelled { step: usize, job: usize },
    Finished(Box<RunRecord>),
}

pub struct RunHandle {
    pub id: String,
    pub events: Receiver<RunEvent>,
    cancel: CancelToken,
}

impl RunHandle {
    pub fn cancel(&self) {
        self.cancel.cancel()
    }
}

/// Start a run on a background thread.
pub fn start(req: RunRequest) -> RunHandle {
    let (tx, rx) = unbounded();
    let cancel = CancelToken::new();
    let id = history::new_run_id();
    let (id2, cancel2) = (id.clone(), cancel.clone());
    std::thread::Builder::new()
        .name(format!("castor-run-{id}"))
        .spawn(move || {
            execute(&id2, req, &cancel2, &tx);
        })
        .expect("spawn run thread");
    RunHandle { id, events: rx, cancel }
}

/// Run synchronously, calling `on_event` for every event. Returns the final record.
pub fn run_blocking(req: RunRequest, cancel: CancelToken, mut on_event: impl FnMut(&RunEvent)) -> RunRecord {
    let (tx, rx) = unbounded();
    let id = history::new_run_id();
    std::thread::scope(|s| {
        // `tx` moves into the worker: the channel closes when the run is over.
        let t = s.spawn(move || execute(&id, req, &cancel, &tx));
        for ev in rx.iter() {
            on_event(&ev);
        }
        t.join().expect("run thread panicked")
    })
}

fn execute(id: &str, req: RunRequest, cancel: &CancelToken, tx: &Sender<RunEvent>) -> RunRecord {
    let started = Instant::now();
    let mut record = RunRecord::new(id, &req);
    let steps = req.steps.len();
    let mut inputs = req.inputs.clone();
    let tmp_root = crate::paths::temp_dir().join(id);
    let mut all_failed = false;

    for (si, step) in req.steps.iter().enumerate() {
        let last = si + 1 == steps;
        let meta = step.tool.meta();
        let policy = if last {
            req.output.clone()
        } else {
            OutputPolicy::Folder(tmp_root.join(format!("step-{si}")))
        };
        let usable: Vec<FileInfo> = inputs.iter().filter(|f| step.tool.accepts(f)).cloned().collect();
        let jobs = match step.tool.plan(&usable, &step.values) {
            Ok(j) => j,
            Err(e) => {
                record.errors.push(format!("{}: {e:#}", meta.id));
                all_failed = true;
                break;
            }
        };
        let _ = tx.send(RunEvent::StepStarted {
            step: si,
            steps,
            tool_id: meta.id.to_string(),
            jobs: jobs.iter().map(Job::label).collect(),
        });

        let parallel = if meta.serial {
            1
        } else if req.max_parallel > 0 {
            req.max_parallel
        } else {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8)
        }
        .min(jobs.len().max(1));

        let next = AtomicUsize::new(0);
        let results: parking_lot::Mutex<Vec<(usize, Result<Vec<Output>, String>)>> = Default::default();
        std::thread::scope(|s| {
            for _ in 0..parallel {
                s.spawn(|| {
                    loop {
                        let ji = next.fetch_add(1, Ordering::SeqCst);
                        let Some(job) = jobs.get(ji) else { break };
                        if cancel.is_cancelled() {
                            let _ = tx.send(RunEvent::JobCancelled { step: si, job: ji });
                            results.lock().push((ji, Err("cancelled".into())));
                            continue;
                        }
                        let _ = tx.send(RunEvent::JobStarted { step: si, job: ji });
                        let t0 = Instant::now();
                        let out_dir = policy.dir_for(job);
                        let tmp = tmp_root.join(format!("s{si}-j{ji}"));
                        let res = std::fs::create_dir_all(&out_dir)
                            .and_then(|_| std::fs::create_dir_all(&tmp))
                            .map_err(anyhow::Error::from)
                            .and_then(|_| {
                                let (txp, txm) = (tx.clone(), tx.clone());
                                let cx = RunCx::new(out_dir.clone(), tmp.clone(), meta.suffix, cancel.clone())
                                    .on_progress(move |f| {
                                        let _ = txp.send(RunEvent::JobProgress { step: si, job: ji, frac: f });
                                    })
                                    .on_message(move |m| {
                                        let _ = txm.send(RunEvent::JobMessage { step: si, job: ji, msg: m });
                                    });
                                // A panicking tool must not take the app down.
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    step.tool.run(job, &step.values, &cx)
                                }))
                                .unwrap_or_else(|p| {
                                    let msg = p
                                        .downcast_ref::<String>()
                                        .cloned()
                                        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                                        .unwrap_or_else(|| "internal error".into());
                                    Err(anyhow::anyhow!("internal error: {msg}"))
                                })
                            });
                        let _ = std::fs::remove_dir_all(&tmp);
                        let secs = t0.elapsed().as_secs_f32();
                        match res {
                            Ok(outputs) => {
                                let bytes_in: u64 = job.inputs.iter().map(|f| f.size).sum();
                                let bytes_out: u64 = outputs
                                    .iter()
                                    .map(|o| std::fs::metadata(&o.path).map(|m| m.len()).unwrap_or(0))
                                    .sum();
                                let _ = tx.send(RunEvent::JobDone {
                                    step: si,
                                    job: ji,
                                    outputs: outputs.clone(),
                                    bytes_in,
                                    bytes_out,
                                    secs,
                                });
                                results.lock().push((ji, Ok(outputs)));
                            }
                            Err(e) if e.is::<Cancelled>() => {
                                let _ = tx.send(RunEvent::JobCancelled { step: si, job: ji });
                                results.lock().push((ji, Err("cancelled".into())));
                            }
                            Err(e) => {
                                let msg = format!("{e:#}");
                                tracing::warn!(tool = meta.id, job = ji, "job failed: {msg}");
                                let _ = tx.send(RunEvent::JobFailed { step: si, job: ji, error: msg.clone() });
                                results.lock().push((ji, Err(msg)));
                            }
                        }
                    }
                });
            }
        });

        let mut results = results.into_inner();
        results.sort_by_key(|(i, _)| *i);
        let mut next_inputs = Vec::new();
        for (ji, r) in results {
            match r {
                Ok(outs) => {
                    if si == 0 {
                        record.bytes_in += jobs[ji].inputs.iter().map(|f| f.size).sum::<u64>();
                        record.files_in += jobs[ji].inputs.len();
                    }
                    for o in outs {
                        if last {
                            record.bytes_out += std::fs::metadata(&o.path).map(|m| m.len()).unwrap_or(0);
                            if let Some(u) = &o.undo {
                                record.undo.push((meta.id.to_string(), u.clone()));
                            }
                            record.outputs.push(o.path.clone());
                        }
                        next_inputs.push(FileInfo::from_path(&o.path));
                    }
                }
                Err(e) if e == "cancelled" => record.cancelled += 1,
                Err(e) => {
                    let name = jobs[ji].label();
                    record.errors.push(if name.is_empty() { e } else { format!("{name}: {e}") });
                }
            }
        }
        if next_inputs.is_empty() {
            all_failed = true;
            break;
        }
        inputs = next_inputs;
        if cancel.is_cancelled() {
            break;
        }
    }

    let _ = std::fs::remove_dir_all(&tmp_root);
    record.duration_secs = started.elapsed().as_secs_f32();
    record.files_out = record.outputs.len();
    record.status = if cancel.is_cancelled() {
        RunStatus::Cancelled
    } else if all_failed || (record.outputs.is_empty() && !record.errors.is_empty()) {
        RunStatus::Failed
    } else if !record.errors.is_empty() {
        RunStatus::Partial
    } else {
        RunStatus::Complete
    };
    if req.record {
        if let Err(e) = history::append(&record) {
            tracing::warn!("could not write history: {e:#}");
        }
    }
    let _ = tx.send(RunEvent::Finished(Box::new(record.clone())));
    record
}
