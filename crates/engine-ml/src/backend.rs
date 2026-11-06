//! Inference backends and the shared session cache.
//!
//! - ONNX Runtime (`ort`, statically linked prebuilt) on Windows, Apple Silicon and Linux.
//! - Intel Macs: ONNX Runtime 1.23.2 (the last official x86_64 build) loaded at run time from
//!   the bundle; when it is missing, `tract` (pure Rust, slower on big models).
//! - `tract` is also the fallback anywhere (`CASTOR_ML_BACKEND=tract`, or automatically if
//!   ONNX Runtime fails to initialise).
//!
//! A model is loaded once per process and shared by every job (thread-safe).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, anyhow};
use parking_lot::Mutex;

/// A dense f32 tensor (row-major).
#[derive(Clone, Debug)]
pub struct Tensor {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

impl Tensor {
    pub fn new(shape: Vec<usize>, data: Vec<f32>) -> Tensor {
        debug_assert_eq!(shape.iter().product::<usize>(), data.len());
        Tensor { shape, data }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Ort,
    Tract,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Ort => "onnxruntime",
            Backend::Tract => "tract",
        }
    }
}

/// The backend in use for this process.
pub fn backend() -> Backend {
    static B: OnceLock<Backend> = OnceLock::new();
    *B.get_or_init(|| {
        let forced = std::env::var("CASTOR_ML_BACKEND").unwrap_or_default().to_ascii_lowercase();
        if forced == "tract" || !ort_backend::AVAILABLE {
            return Backend::Tract;
        }
        match ort_backend::init() {
            Ok(()) => Backend::Ort,
            Err(e) => {
                tracing::warn!("ONNX Runtime unavailable ({e:#}); using tract");
                Backend::Tract
            }
        }
    })
}

/// A loaded network: one f32 input, outputs selected at load time.
pub trait Net: Send + Sync {
    /// Run on one input; returns the selected outputs, in the order requested at load time.
    fn run(&self, input: Tensor) -> Result<Vec<Tensor>>;
    /// True if several `run` calls may usefully execute in parallel (tract: each call is
    /// mostly single-threaded; ONNX Runtime already uses every core per call).
    fn parallel_friendly(&self) -> bool;
}

type Key = (PathBuf, Vec<usize>);

fn cache() -> &'static Mutex<HashMap<Key, Arc<dyn Net>>> {
    static C: OnceLock<Mutex<HashMap<Key, Arc<dyn Net>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Load (or fetch from the cache) a network for a fixed input shape, keeping only the outputs
/// with these indices (graph order).
pub fn load(path: &Path, input_shape: &[usize], outputs: &[usize]) -> Result<Arc<dyn Net>> {
    let key = (path.to_path_buf(), input_shape.to_vec());
    let mut c = cache().lock();
    if let Some(n) = c.get(&key) {
        return Ok(n.clone());
    }
    // ONNX Runtime handles any shape with one session: reuse a session of another shape.
    if backend() == Backend::Ort {
        if let Some((_, n)) = c.iter().find(|((p, _), _)| p == path) {
            let n = n.clone();
            c.insert(key, n.clone());
            return Ok(n);
        }
    }
    let t = std::time::Instant::now();
    let net: Arc<dyn Net> = match backend() {
        Backend::Ort => ort_backend::load(path, outputs)?,
        Backend::Tract => Arc::new(TractNet::load(path, input_shape, outputs)?),
    };
    tracing::info!("loaded {} with {} in {:?}", path.display(), backend().name(), t.elapsed());
    c.insert(key, net.clone());
    Ok(net)
}

/// Drop every cached model (frees memory after a batch).
pub fn unload_all() {
    cache().lock().clear();
}

// ---------------------------------------------------------------- tract

type TractPlan = tract_onnx::prelude::TypedSimplePlan;

struct TractNet {
    plan: Arc<TractPlan>,
}

impl TractNet {
    fn load(path: &Path, shape: &[usize], outputs: &[usize]) -> Result<TractNet> {
        use tract_onnx::prelude::*;
        init_tract_threads();
        let mut model = tract_onnx::onnx()
            .model_for_path(path)
            .with_context(|| format!("cannot read model {}", path.display()))?
            .with_input_fact(0, f32::fact(shape).into())?;
        let all = model.output_outlets()?.to_vec();
        let keep: Vec<_> = outputs.iter().filter_map(|i| all.get(*i).copied()).collect();
        model.select_output_outlets(&keep)?;
        let plan = model.into_optimized()?.into_runnable()?;
        Ok(TractNet { plan })
    }
}

fn init_tract_threads() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        tract_linalg::multithread::set_default_executor(tract_linalg::multithread::Executor::multithread(n));
    });
}

impl Net for TractNet {
    fn run(&self, input: Tensor) -> Result<Vec<Tensor>> {
        use tract_onnx::prelude::{IntoTValue, tvec};
        let t = tract_onnx::prelude::Tensor::from_shape(&input.shape, &input.data)?;
        drop(input);
        let out = self.plan.run(tvec!(t.into_tvalue()))?;
        out.iter()
            .map(|v| {
                let view = v.try_as_plain_ram()?;
                Ok(Tensor { shape: v.shape().to_vec(), data: view.as_slice::<f32>()?.to_vec() })
            })
            .collect()
    }
    fn parallel_friendly(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------- ONNX Runtime

mod ort_backend {
    use super::*;

    pub const AVAILABLE: bool = true;

    static ENV: OnceLock<ort::environment::Environment> = OnceLock::new();

    /// Intel Macs: where the ONNX Runtime 1.23 dylib may be.
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    pub fn dylib_path() -> Option<PathBuf> {
        let mut c = Vec::new();
        if let Some(p) = std::env::var_os("CASTOR_ORT_DYLIB") {
            c.push(PathBuf::from(p));
        }
        let res = castor_core::paths::resources_dir();
        c.push(res.join("../Frameworks/libonnxruntime.dylib"));
        c.push(res.join("libonnxruntime.dylib"));
        c.push(res.join("lib/libonnxruntime.dylib"));
        c.push(castor_core::paths::downloads_dir().join("runtime/libonnxruntime.dylib"));
        c.into_iter().find(|p| p.is_file())
    }

    pub fn init() -> Result<()> {
        if ENV.get().is_some() {
            return Ok(());
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        let builder = {
            let p = dylib_path().ok_or_else(|| anyhow!("libonnxruntime.dylib not found"))?;
            ort::init_from(&p).map_err(|e| anyhow!("{e}"))?
        };
        #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
        let builder = ort::init();
        let env = builder.with_name("castor").build().map_err(|e| anyhow!("{e}"))?;
        let _ = ENV.set(env);
        Ok(())
    }

    struct OrtNet {
        session: Mutex<ort::session::Session>,
        outputs: Vec<String>,
    }

    pub fn load(path: &Path, outputs: &[usize]) -> Result<Arc<dyn Net>> {
        use ort::session::builder::GraphOptimizationLevel;
        init()?;
        let env = ENV.get().ok_or_else(|| anyhow!("ONNX Runtime not initialised"))?;
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let session = ort::session::Session::builder(env)
            .map_err(|e| anyhow!("{e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| anyhow!("{e}"))?
            .with_intra_threads(threads)
            .map_err(|e| anyhow!("{e}"))?
            .commit_from_file(path)
            .map_err(|e| anyhow!("cannot load model {}: {e}", path.display()))?;
        let names: Vec<String> = session.outputs().iter().map(|o| o.name().to_string()).collect();
        let outputs = outputs.iter().filter_map(|i| names.get(*i).cloned()).collect();
        Ok(Arc::new(OrtNet { session: Mutex::new(session), outputs }))
    }

    impl Net for OrtNet {
        fn run(&self, input: Tensor) -> Result<Vec<Tensor>> {
            let shape: Vec<i64> = input.shape.iter().map(|d| *d as i64).collect();
            let t = ort::value::Tensor::from_array((shape, input.data)).map_err(|e| anyhow!("{e}"))?;
            let mut s = self.session.lock();
            let outs = s.run(ort::inputs![t]).map_err(|e| anyhow!("{e}"))?;
            let mut v = Vec::with_capacity(self.outputs.len());
            for name in &self.outputs {
                let val = outs.get(name.as_str()).ok_or_else(|| anyhow!("model output {name} missing"))?;
                let (shape, data) = val.try_extract_tensor::<f32>().map_err(|e| anyhow!("{e}"))?;
                v.push(Tensor { shape: shape.iter().map(|d| *d as usize).collect(), data: data.to_vec() });
            }
            Ok(v)
        }
        fn parallel_friendly(&self) -> bool {
            false
        }
    }
}
