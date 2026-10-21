//! One worker thread owns the model: load on demand, unload when idle.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::rest::RestEmbedder;
use sapphire_bridge_api::{ApiKey, LocalModel, QType, RemoteModel};

use crate::template::TEMPLATE_VERSION;
use crate::{Error, Result};

/// A loaded embedding model or provider. Owned by the worker thread.
pub trait Embed: Send {
    /// One vector per text, in order.
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// One vector per image (encoded bytes: PNG, JPEG, WebP or GIF), in order, in the same
    /// space as the texts'. Only a model whose [`ModelInfo::images`] is set supports it.
    fn embed_images(&mut self, images: &[Vec<u8>]) -> Result<Vec<Vec<f32>>> {
        let _ = images;
        Err(Error::Embed("this model cannot embed images".into()))
    }
}

/// Builds the model. Called on the worker thread, on demand.
pub type Loader = Box<dyn FnMut() -> Result<Box<dyn Embed>> + Send>;

/// What a configured service produces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    pub model: String,
    pub dimension: u32,
    /// The prompt template applied before embedding; `0` when none is (REST).
    pub template_version: u32,
    /// Content tokens kept before embedding (local); `None` when the provider decides (REST).
    pub max_tokens: Option<u32>,
    /// Whether [`EmbedService::embed_images`] works.
    pub images: bool,
}

/// The model unloads after this long without a request.
pub const IDLE_UNLOAD: Duration = Duration::from_secs(10 * 60);
/// After a failed load, requests fail fast for this long before the next attempt.
pub const RETRY_BACKOFF: Duration = Duration::from_secs(60);
/// Texts per call to [`Embed::embed`].
pub const BATCH: usize = 32;
/// Images per call to [`Embed::embed_images`]: each is up to `max_tokens` tokens long.
pub const IMAGE_BATCH: usize = 1;

type Reply = tokio::sync::oneshot::Sender<Result<Vec<Vec<f32>>>>;

/// What a job embeds.
enum Input {
    Texts(Vec<String>),
    Images(Vec<Vec<u8>>),
}

impl Input {
    fn len(&self) -> usize {
        match self {
            Input::Texts(t) => t.len(),
            Input::Images(i) => i.len(),
        }
    }
}

type Job = (Input, Reply);

/// The embedding service: a request channel to one worker thread.
///
/// Requests from any number of callers queue and are served one at a time, in order.
/// [`info`](Self::info) and [`loaded`](Self::loaded) never wait on the worker.
pub struct EmbedService {
    info: ModelInfo,
    loaded: Arc<AtomicBool>,
    tx: mpsc::Sender<Job>,
}

impl EmbedService {
    /// The local model, its weights held as `qtype` and its files cached in `cache_dir`
    /// (hf-hub layout). The model loads on the first request.
    pub fn local(model: &LocalModel, qtype: QType, cache_dir: PathBuf) -> EmbedService {
        let info = ModelInfo {
            model: model.model.clone(),
            dimension: model.dimension,
            template_version: TEMPLATE_VERSION,
            max_tokens: Some(model.max_tokens as u32),
            images: cfg!(feature = "local"),
        };
        let loader = local_loader(model.clone(), qtype, cache_dir);
        Self::with_loader(info, loader, IDLE_UNLOAD, RETRY_BACKOFF)
    }

    /// The remote model, sending `key` when there is one (a local endpoint may need none).
    pub fn remote(model: &RemoteModel, key: Option<ApiKey>) -> EmbedService {
        let info = ModelInfo {
            model: model.model.clone(),
            dimension: model.dimension,
            template_version: 0,
            max_tokens: None,
            images: false,
        };
        let model = model.clone();
        let loader: Loader =
            Box::new(
                move || Ok(Box::new(RestEmbedder::new(&model, key.clone())) as Box<dyn Embed>),
            );
        Self::with_loader(info, loader, IDLE_UNLOAD, RETRY_BACKOFF)
    }

    /// A service over `loader`: what [`local`](Self::local) and [`remote`](Self::remote) build on,
    /// and what tests use.
    pub fn with_loader(
        info: ModelInfo,
        loader: Loader,
        idle_unload: Duration,
        retry_backoff: Duration,
    ) -> EmbedService {
        let (tx, rx) = mpsc::channel();
        let loaded = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            loader,
            model: None,
            last_failure: None,
            retry_backoff,
            loaded: loaded.clone(),
        };
        std::thread::Builder::new()
            .name("sapphire-embed".into())
            .spawn(move || worker.run(rx, idle_unload))
            .expect("spawn the embedding worker thread");
        EmbedService { info, loaded, tx }
    }

    /// The configured model. Cheap; never blocks.
    pub fn info(&self) -> ModelInfo {
        self.info.clone()
    }

    /// Whether the model is in memory now. Cheap; never blocks.
    pub fn loaded(&self) -> bool {
        self.loaded.load(Ordering::Acquire)
    }

    /// Embed `texts`, in batches of [`BATCH`] inside the worker. Loads the model if needed.
    ///
    /// A caller that drops this future does not cancel its job: the worker still runs it.
    pub async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        self.submit(Input::Texts(texts)).await
    }

    /// Embed `images` (encoded bytes), one at a time inside the worker. Loads the model if
    /// needed; fails when the model cannot embed images.
    pub async fn embed_images(&self, images: Vec<Vec<u8>>) -> Result<Vec<Vec<f32>>> {
        if !self.info.images && !images.is_empty() {
            return Err(Error::Embed(format!(
                "{} cannot embed images",
                self.info.model
            )));
        }
        self.submit(Input::Images(images)).await
    }

    async fn submit(&self, input: Input) -> Result<Vec<Vec<f32>>> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.tx.send((input, reply)).map_err(|_| Error::Stopped)?;
        rx.await.map_err(|_| Error::Stopped)?
    }
}

#[cfg(feature = "local")]
fn local_loader(settings: LocalModel, qtype: QType, cache_dir: PathBuf) -> Loader {
    Box::new(move || {
        Ok(
            Box::new(crate::local::LocalQwen::load(&settings, qtype, &cache_dir)?)
                as Box<dyn Embed>,
        )
    })
}

#[cfg(not(feature = "local"))]
fn local_loader(_settings: LocalModel, _qtype: QType, _cache_dir: PathBuf) -> Loader {
    Box::new(|| {
        Err(Error::Load(
            "this bridge was built without the `local` embedding feature".into(),
        ))
    })
}

/// The worker thread's state. Dropping the last [`EmbedService`] ends the thread.
struct Worker {
    loader: Loader,
    model: Option<Box<dyn Embed>>,
    /// When and why the last load failed.
    last_failure: Option<(Instant, String)>,
    retry_backoff: Duration,
    loaded: Arc<AtomicBool>,
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Job>, idle_unload: Duration) {
        loop {
            match rx.recv_timeout(idle_unload) {
                Ok((input, reply)) => {
                    let result = self.serve(&input);
                    let _ = reply.send(result);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self.model.take().is_some() {
                        self.loaded.store(false, Ordering::Release);
                        tracing::info!("embedding model unloaded after {idle_unload:?} idle");
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn serve(&mut self, input: &Input) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(input.len());
        match input {
            Input::Texts(texts) => {
                for chunk in texts.chunks(BATCH) {
                    out.extend(self.call(chunk.len(), |m| m.embed(chunk))?);
                }
            }
            Input::Images(images) => {
                for chunk in images.chunks(IMAGE_BATCH) {
                    out.extend(self.call(chunk.len(), |m| m.embed_images(chunk))?);
                }
            }
        }
        Ok(out)
    }

    /// One call into the model, for `len` inputs, loading it first if needed.
    fn call(
        &mut self,
        len: usize,
        call: impl FnOnce(&mut dyn Embed) -> Result<Vec<Vec<f32>>>,
    ) -> Result<Vec<Vec<f32>>> {
        let model = self.ensure_loaded()?;
        let vectors = match catch_unwind(AssertUnwindSafe(|| call(model.as_mut()))) {
            Ok(r) => r?,
            Err(panic) => {
                // The model's state is unknown after a panic; load it again next time.
                self.model = None;
                self.loaded.store(false, Ordering::Release);
                return Err(Error::Embed(format!(
                    "the model panicked: {}",
                    panic_message(&panic)
                )));
            }
        };
        if vectors.len() != len {
            return Err(Error::Embed(format!(
                "the model returned {} vectors for {len} inputs",
                vectors.len(),
            )));
        }
        Ok(vectors)
    }

    fn ensure_loaded(&mut self) -> Result<&mut Box<dyn Embed>> {
        if self.model.is_none() {
            if let Some((at, message)) = &self.last_failure
                && at.elapsed() < self.retry_backoff
            {
                return Err(Error::Load(message.clone()));
            }
            let started = Instant::now();
            let result = match catch_unwind(AssertUnwindSafe(|| (self.loader)())) {
                Ok(r) => r,
                Err(panic) => Err(Error::Load(format!(
                    "the loader panicked: {}",
                    panic_message(&panic)
                ))),
            };
            match result {
                Ok(model) => {
                    tracing::info!("embedding model loaded in {:?}", started.elapsed());
                    self.last_failure = None;
                    self.model = Some(model);
                    self.loaded.store(true, Ordering::Release);
                }
                Err(e) => {
                    let message = match e {
                        Error::Load(m) => m,
                        other => other.to_string(),
                    };
                    tracing::warn!("embedding model failed to load: {message}");
                    self.last_failure = Some((Instant::now(), message.clone()));
                    return Err(Error::Load(message));
                }
            }
        }
        Ok(self.model.as_mut().expect("loaded above"))
    }
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    /// Returns `[len(text)]` per text and records every call's texts. Panics on `"boom"`.
    /// Sets `dropped` when dropped.
    struct FakeEmbed {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        dropped: Arc<AtomicBool>,
    }

    impl Embed for FakeEmbed {
        fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            assert!(!texts.iter().any(|t| t == "boom"), "boom");
            self.calls.lock().unwrap().push(texts.to_vec());
            Ok(texts.iter().map(|t| vec![t.len() as f32]).collect())
        }

        /// `[-len(bytes)]` per image, recorded as one call of `"<n> images"`.
        fn embed_images(&mut self, images: &[Vec<u8>]) -> Result<Vec<Vec<f32>>> {
            self.calls
                .lock()
                .unwrap()
                .push(vec![format!("{} images", images.len())]);
            Ok(images.iter().map(|i| vec![-(i.len() as f32)]).collect())
        }
    }

    impl Drop for FakeEmbed {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    struct Harness {
        service: EmbedService,
        loads: Arc<AtomicUsize>,
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        dropped: Arc<AtomicBool>,
    }

    fn info() -> ModelInfo {
        ModelInfo {
            model: "fake".into(),
            dimension: 1,
            template_version: 1,
            max_tokens: None,
            images: true,
        }
    }

    fn harness(idle: Duration) -> Harness {
        let loads = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let (l, c, d) = (loads.clone(), calls.clone(), dropped.clone());
        let loader: Loader = Box::new(move || {
            l.fetch_add(1, Ordering::SeqCst);
            d.store(false, Ordering::SeqCst);
            Ok(Box::new(FakeEmbed {
                calls: c.clone(),
                dropped: d.clone(),
            }) as Box<dyn Embed>)
        });
        let service = EmbedService::with_loader(info(), loader, idle, Duration::from_secs(60));
        Harness {
            service,
            loads,
            calls,
            dropped,
        }
    }

    #[tokio::test]
    async fn a_panic_in_embed_fails_the_request_and_reloads_next_time() {
        let h = harness(Duration::from_secs(60));
        h.service.embed(texts(&["a"])).await.unwrap();
        let err = h.service.embed(texts(&["boom"])).await.unwrap_err();
        assert!(matches!(err, Error::Embed(_)), "{err}");
        assert!(err.to_string().contains("panicked"), "{err}");
        assert!(!h.service.loaded());
        assert!(h.dropped.load(Ordering::SeqCst));
        assert_eq!(
            h.service.embed(texts(&["ok"])).await.unwrap(),
            vec![vec![2.0]]
        );
        assert_eq!(h.loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn dropping_the_service_ends_the_worker() {
        let h = harness(Duration::from_secs(60));
        h.service.embed(texts(&["a"])).await.unwrap();
        assert!(!h.dropped.load(Ordering::SeqCst));
        let dropped = h.dropped.clone();
        drop(h.service);
        // The worker exits on disconnect and drops its model with it.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dropped.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "the worker did not exit");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn requests_are_served_in_order() {
        let h = harness(Duration::from_secs(60));
        let (a, b, c) = tokio::join!(
            h.service.embed(texts(&["a"])),
            h.service.embed(texts(&["bb", "bb"])),
            h.service.embed(texts(&["ccc"])),
        );
        assert_eq!(a.unwrap(), vec![vec![1.0]]);
        assert_eq!(b.unwrap(), vec![vec![2.0], vec![2.0]]);
        assert_eq!(c.unwrap(), vec![vec![3.0]]);
        let calls = h.calls.lock().unwrap();
        assert_eq!(
            *calls,
            vec![texts(&["a"]), texts(&["bb", "bb"]), texts(&["ccc"])]
        );
        assert_eq!(h.service.info(), info());
    }

    #[tokio::test]
    async fn the_loader_runs_once_across_requests() {
        let h = harness(Duration::from_secs(60));
        assert!(!h.service.loaded());
        h.service.embed(texts(&["a"])).await.unwrap();
        assert!(h.service.loaded());
        h.service.embed(texts(&["b"])).await.unwrap();
        assert_eq!(h.loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_model_unloads_when_idle_and_reloads_on_demand() {
        let h = harness(Duration::from_millis(50));
        h.service.embed(texts(&["a"])).await.unwrap();
        // The worker is a std thread on a real-time timeout; a busy runner can schedule it
        // late, so wait for the unload rather than for a fixed time.
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.service.loaded() {
            assert!(Instant::now() < deadline, "the model was not unloaded");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        h.service.embed(texts(&["b"])).await.unwrap();
        assert!(h.service.loaded());
        assert_eq!(h.loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failed_load_fails_fast_within_the_backoff() {
        let loads = Arc::new(AtomicUsize::new(0));
        let l = loads.clone();
        let loader: Loader = Box::new(move || {
            l.fetch_add(1, Ordering::SeqCst);
            Err(Error::Load("no network".into()))
        });
        let service = EmbedService::with_loader(
            info(),
            loader,
            Duration::from_secs(60),
            Duration::from_secs(60),
        );
        let first = service.embed(texts(&["a"])).await.unwrap_err();
        assert!(first.to_string().contains("no network"), "{first}");
        let second = service.embed(texts(&["a"])).await.unwrap_err();
        assert!(second.to_string().contains("no network"), "{second}");
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert!(!service.loaded());
    }

    #[tokio::test]
    async fn a_failed_load_is_retried_after_the_backoff() {
        let loads = Arc::new(AtomicUsize::new(0));
        let l = loads.clone();
        let loader: Loader = Box::new(move || {
            l.fetch_add(1, Ordering::SeqCst);
            Err(Error::Load("no network".into()))
        });
        let service = EmbedService::with_loader(
            info(),
            loader,
            Duration::from_secs(60),
            Duration::from_millis(20),
        );
        service.embed(texts(&["a"])).await.unwrap_err();
        tokio::time::sleep(Duration::from_millis(60)).await;
        service.embed(texts(&["a"])).await.unwrap_err();
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn seventy_texts_go_in_batches_of_at_most_32() {
        let h = harness(Duration::from_secs(60));
        let many: Vec<String> = (0..70).map(|i| "x".repeat(i % 5 + 1)).collect();
        let out = h.service.embed(many.clone()).await.unwrap();
        assert_eq!(out.len(), 70);
        for (v, t) in out.iter().zip(&many) {
            assert_eq!(v, &vec![t.len() as f32]);
        }
        let sizes: Vec<usize> = h.calls.lock().unwrap().iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![32, 32, 6]);
    }

    #[tokio::test]
    async fn images_go_one_at_a_time_through_the_same_model() {
        let h = harness(Duration::from_secs(60));
        h.service.embed(texts(&["a"])).await.unwrap();
        let out = h
            .service
            .embed_images(vec![vec![0; 3], vec![0; 5]])
            .await
            .unwrap();
        assert_eq!(out, vec![vec![-3.0], vec![-5.0]]);
        assert_eq!(h.loads.load(Ordering::SeqCst), 1);
        let calls = h.calls.lock().unwrap().clone();
        assert_eq!(calls[1..], [vec!["1 images"], vec!["1 images"]]);
    }

    #[tokio::test]
    async fn a_text_only_model_refuses_images_without_loading() {
        let remote = EmbedService::remote(
            &RemoteModel {
                endpoint: "http://127.0.0.1:9".into(),
                model: "m".into(),
                dimension: 8,
            },
            None,
        );
        let err = remote.embed_images(vec![vec![1]]).await.unwrap_err();
        assert!(err.to_string().contains("cannot embed images"), "{err}");
        assert!(!remote.loaded());
    }

    #[tokio::test]
    async fn an_empty_request_does_not_load() {
        let h = harness(Duration::from_secs(60));
        assert!(h.service.embed(Vec::new()).await.unwrap().is_empty());
        assert_eq!(h.loads.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn each_slot_reports_its_model_without_loading() {
        let local = EmbedService::local(
            &LocalModel::default(),
            QType::default(),
            PathBuf::from("unused"),
        );
        assert_eq!(
            local.info(),
            ModelInfo {
                model: crate::LOCAL_MODEL.into(),
                dimension: 1024,
                template_version: crate::TEMPLATE_VERSION,
                max_tokens: Some(1024),
                images: true,
            }
        );
        assert!(!local.loaded());

        let remote = EmbedService::remote(
            &RemoteModel {
                endpoint: "http://127.0.0.1:9".into(),
                model: "m".into(),
                dimension: 8,
            },
            None,
        );
        assert_eq!(
            remote.info(),
            ModelInfo {
                model: "m".into(),
                dimension: 8,
                template_version: 0,
                max_tokens: None,
                images: false,
            }
        );
    }
}
