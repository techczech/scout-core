//! The embedder seam: text in, unit vectors out.
//!
//! - [`Embedder`] is the trait the vector store and semantic search use.
//! - [`HashEmbedder`] is a deterministic bag-of-hashed-words fake: no model,
//!   no download; tests use it (and `SCOUT_EMBEDDER=hash` in the CLI).
//! - `OnnxEmbedder` (feature `onnx`) runs a multilingual sentence-embedding
//!   model in-process through fastembed / ONNX Runtime. The model is
//!   downloaded once, on first use, to the platform data dir.

use crate::normalize;
use crate::tokenize::words;
use anyhow::Result;

/// Text → L2-normalised vectors. Passages and queries are embedded
/// separately because some models (E5) prefix them differently.
pub trait Embedder: Send + Sync {
    /// The model and prompt scheme; vectors built under another id are
    /// rebuilt.
    fn model_id(&self) -> String;
    fn embed_passages(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
    /// A line to show before first use when that use downloads the model
    /// (name, size, destination); `None` when the model is ready.
    fn download_note(&self) -> Option<String> {
        None
    }
}

/// Scale `v` to unit length (a zero vector stays zero).
pub fn l2_normalise(v: &mut [f32]) {
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
}

/// The deterministic fake: each word adds ±1 to a hashed bucket.
#[derive(Debug, Clone)]
pub struct HashEmbedder {
    pub dim: usize,
}

impl Default for HashEmbedder {
    fn default() -> Self {
        HashEmbedder { dim: 64 }
    }
}

impl HashEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dim];
        for w in words(&normalize::normalize(text, &[]).text) {
            // FNV-1a: stable across runs and platforms.
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in w.to_lowercase().bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            let sign = if (h >> 63) == 0 { 1.0 } else { -1.0 };
            v[(h % self.dim as u64) as usize] += sign;
        }
        l2_normalise(&mut v);
        v
    }
}

impl Embedder for HashEmbedder {
    fn model_id(&self) -> String {
        format!("hash-{}", self.dim)
    }
    fn embed_passages(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.embed(t)).collect())
    }
    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.embed(text))
    }
}

#[cfg(feature = "onnx")]
pub use onnx::{OnnxEmbedder, OnnxModel, DEFAULT_MODEL};

#[cfg(feature = "onnx")]
mod onnx {
    use super::{l2_normalise, Embedder};
    use anyhow::{anyhow, bail, Result};
    use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// A supported model: its key (`SCOUT_EMBED_MODEL`), fastembed id, Hub
    /// repo, download size and prompt prefixes.
    #[derive(Debug, Clone, Copy)]
    pub struct OnnxModel {
        pub key: &'static str,
        model: fn() -> EmbeddingModel,
        pub repo: &'static str,
        pub download_mb: u32,
        query_prefix: &'static str,
        passage_prefix: &'static str,
    }

    pub const MODELS: &[OnnxModel] = &[
        OnnxModel {
            key: "e5-small",
            model: || EmbeddingModel::MultilingualE5Small,
            repo: "intfloat/multilingual-e5-small",
            download_mb: 490,
            query_prefix: "query: ",
            passage_prefix: "passage: ",
        },
        OnnxModel {
            key: "minilm-l12",
            model: || EmbeddingModel::ParaphraseMLMiniLML12V2,
            repo: "Xenova/paraphrase-multilingual-MiniLM-L12-v2",
            download_mb: 490,
            query_prefix: "",
            passage_prefix: "",
        },
        OnnxModel {
            key: "e5-base",
            model: || EmbeddingModel::MultilingualE5Base,
            repo: "intfloat/multilingual-e5-base",
            download_mb: 1115,
            query_prefix: "query: ",
            passage_prefix: "passage: ",
        },
    ];

    /// The model chosen by the recall test (ticket 07).
    pub const DEFAULT_MODEL: &str = "minilm-l12";

    /// Passages longer than this many tokens are truncated.
    const MAX_TOKENS: usize = 256;
    const BATCH: usize = 64;
    /// Extra attempts at an interrupted model download.
    const DOWNLOAD_TRIES: usize = 5;

    /// fastembed behind a lazy, locked session: nothing loads until the
    /// first embed, so full-text commands never pay for the model.
    pub struct OnnxEmbedder {
        spec: OnnxModel,
        cache_dir: PathBuf,
        inner: Mutex<Option<TextEmbedding>>,
    }

    impl OnnxEmbedder {
        /// `key` names one of [`MODELS`]; the model files live in
        /// `cache_dir`.
        pub fn new(key: &str, cache_dir: PathBuf) -> Result<OnnxEmbedder> {
            let spec = *MODELS.iter().find(|m| m.key == key).ok_or_else(|| {
                let keys: Vec<&str> = MODELS.iter().map(|m| m.key).collect();
                anyhow!(
                    "unknown embedding model {key:?} (known: {})",
                    keys.join(", ")
                )
            })?;
            Ok(OnnxEmbedder {
                spec,
                cache_dir,
                inner: Mutex::new(None),
            })
        }

        /// `SCOUT_EMBED_MODEL` (default [`DEFAULT_MODEL`]), files in
        /// `SCOUT_MODEL_DIR`, else `<data dir>/scout/models`.
        pub fn from_env() -> Result<OnnxEmbedder> {
            let key = std::env::var("SCOUT_EMBED_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
            let dir = std::env::var_os("SCOUT_MODEL_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(crate::index::models_dir);
            OnnxEmbedder::new(&key, dir)
        }

        /// Where the files really go: fastembed honours `HF_HOME` (a shared
        /// Hugging Face cache the user chose) over the directory given.
        pub fn effective_dir(&self) -> PathBuf {
            std::env::var_os("HF_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| self.cache_dir.clone())
        }

        fn downloaded(&self) -> bool {
            let d = self
                .effective_dir()
                .join(format!("models--{}", self.spec.repo.replace('/', "--")));
            d.join("snapshots").is_dir()
        }

        fn run(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| anyhow!("embedder lock poisoned"))?;
            if guard.is_none() {
                std::fs::create_dir_all(&self.cache_dir)?;
                let opts = TextInitOptions::new((self.spec.model)())
                    .with_cache_dir(self.cache_dir.clone())
                    .with_max_length(MAX_TOKENS)
                    .with_show_download_progress(true);
                // A download that breaks off resumes from its partial file,
                // so a slow connection gets a few more tries.
                let mut attempt = 0;
                let model = loop {
                    match TextEmbedding::try_new(opts.clone()) {
                        Ok(m) => break m,
                        Err(e) if attempt < DOWNLOAD_TRIES && !self.downloaded() => {
                            attempt += 1;
                            eprintln!("scout: note: model download interrupted ({e}); resuming");
                        }
                        Err(e) => bail!("load embedding model {}: {e}", self.spec.key),
                    }
                };
                *guard = Some(model);
            }
            let model = guard.as_mut().expect("loaded");
            let mut out = model
                .embed(&texts, Some(BATCH))
                .map_err(|e| anyhow!("embed: {e}"))?;
            if out.len() != texts.len() {
                bail!(
                    "embedder returned {} vectors for {} texts",
                    out.len(),
                    texts.len()
                );
            }
            for v in out.iter_mut() {
                l2_normalise(v);
            }
            Ok(out)
        }
    }

    impl Embedder for OnnxEmbedder {
        fn model_id(&self) -> String {
            format!("{}@{}", self.spec.key, MAX_TOKENS)
        }
        fn embed_passages(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
            let p = self.spec.passage_prefix;
            self.run(texts.iter().map(|t| format!("{p}{t}")).collect())
        }
        fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
            let q = self.spec.query_prefix;
            Ok(self.run(vec![format!("{q}{text}")])?.remove(0))
        }
        fn download_note(&self) -> Option<String> {
            if self.downloaded() {
                return None;
            }
            Some(format!(
                "scout: note: downloading embedding model {} ({}, about {} MB) once to {}",
                self.spec.key,
                self.spec.repo,
                self.spec.download_mb,
                self.effective_dir().display()
            ))
        }
    }
}
