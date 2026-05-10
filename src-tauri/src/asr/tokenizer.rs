//! Tokenizer + streaming detokenizer for the ASR decoder output (M2a step 3).
//!
//! Parakeet emits token IDs; we need text. The model ships with a HuggingFace
//! `tokenizer.json` that uses SentencePiece BPE under the hood — `▁` (U+2581)
//! prefixes word boundaries and gets replaced with a space at decode time.
//!
//! Two pieces:
//!   * [`Tokenizer`] — the small trait the rest of the crate codes against.
//!     [`BpeTokenizer`] is the production impl, wrapping the `tokenizers`
//!     crate. Tests use the in-module `MockTokenizer` to exercise the
//!     streaming logic without a real model file.
//!   * [`StreamingDecoder`] — accepts one (or many) token IDs at a time and
//!     returns the **new text** since the last call. Internally it just
//!     re-decodes the whole token sequence and diffs against the previous
//!     decoded string. O(n²) total across n tokens but that's fine: we run
//!     this per-segment, n is small, and the operation is text-prefix
//!     comparison.
//!
//! ## Why diff-on-decode rather than concatenate-tokens?
//!
//! BPE merges + SentencePiece word-boundary handling mean a single token's
//! contribution to text is context-dependent. Concatenating per-token
//! decodes loses the "is this a word boundary or a continuation" signal.
//! Re-decoding the whole sequence each call is correct by construction.

#![allow(dead_code)] // Wired up in step 3.

use std::path::Path;
use std::sync::Arc;

use crate::{Error, Result};

/// Minimal contract for a token-id → text decoder. Decoupled from any
/// specific library so the streaming logic above can be tested with a
/// trivial mock.
pub trait Tokenizer: Send + Sync {
    fn decode(&self, ids: &[u32]) -> Result<String>;
}

/// Production tokenizer: thin wrapper around HuggingFace `tokenizers` so we
/// can swap models by dropping a different `tokenizer.json` into the model
/// directory.
pub struct BpeTokenizer {
    inner: tokenizers::Tokenizer,
}

impl BpeTokenizer {
    /// Load a tokenizer.json (the format NeMo's export script produces
    /// alongside encoder.onnx + decoder_joint.onnx).
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let inner = tokenizers::Tokenizer::from_file(path.as_ref())
            .map_err(|e| Error::Asr(format!("load tokenizer: {e}")))?;
        Ok(Self { inner })
    }

    pub fn vocab_size(&self) -> usize {
        self.inner.get_vocab_size(true)
    }
}

impl Tokenizer for BpeTokenizer {
    fn decode(&self, ids: &[u32]) -> Result<String> {
        self.inner
            .decode(ids, /* skip_special_tokens */ true)
            .map_err(|e| Error::Asr(format!("decode: {e}")))
    }
}

/// Maintains the running token sequence; emits **only the new text** added
/// since the last call. Lets the caller append to a transcript without
/// re-rendering the whole thing each token.
///
/// `T: Tokenizer` lets tests substitute a mock without touching the
/// `tokenizers` crate. The production code path will use
/// `StreamingDecoder<BpeTokenizer>` (or `StreamingDecoder<Arc<dyn Tokenizer>>`).
pub struct StreamingDecoder<T: Tokenizer> {
    tokenizer: Arc<T>,
    ids: Vec<u32>,
    /// The full decoded text from the previous call; cached so we can diff
    /// against it on the next push.
    decoded: String,
}

impl<T: Tokenizer> StreamingDecoder<T> {
    pub fn new(tokenizer: Arc<T>) -> Self {
        Self {
            tokenizer,
            ids: Vec::new(),
            decoded: String::new(),
        }
    }

    /// Push a single token, get back the text that was newly added (which
    /// may be `""` if the token was a continuation that didn't yet form a
    /// printable run).
    pub fn push(&mut self, id: u32) -> Result<String> {
        self.push_many(&[id])
    }

    /// Push multiple tokens at once. Same diff semantics.
    pub fn push_many(&mut self, ids: &[u32]) -> Result<String> {
        self.ids.extend_from_slice(ids);
        let full = self.tokenizer.decode(&self.ids)?;
        let delta = if full.starts_with(&self.decoded) {
            full[self.decoded.len()..].to_string()
        } else {
            // Tokenizer rewrote earlier text (rare — happens with some BPE
            // boundary cases). Surface the whole thing and let the caller
            // replace; transcripts are append-only at the segment level
            // anyway, so this only matters within a segment.
            tracing::debug!(
                prev = %self.decoded,
                full = %full,
                "tokenizer rewrote prefix during streaming decode",
            );
            full.clone()
        };
        self.decoded = full;
        Ok(delta)
    }

    /// Full text decoded so far this session.
    pub fn text(&self) -> &str {
        &self.decoded
    }

    /// Number of tokens accumulated.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Forget all tokens. Call between transcript segments.
    pub fn reset(&mut self) {
        self.ids.clear();
        self.decoded.clear();
    }
}

// ---------------------------------------------------------------------------
// Tests — exercise the streaming logic against a mock so we don't need a
// real tokenizer.json checked into the repo. The BpeTokenizer wrapper itself
// is trivial; integration with real Parakeet artifacts is M2a step 3.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Mock tokenizer with a controllable vocab. Records calls so tests can
    /// assert decode-call counts (e.g. that streaming actually re-decodes
    /// each push).
    struct MockTokenizer {
        vocab: HashMap<u32, &'static str>,
        calls: Mutex<usize>,
    }

    impl MockTokenizer {
        fn new(entries: &[(u32, &'static str)]) -> Arc<Self> {
            Arc::new(Self {
                vocab: entries.iter().copied().collect(),
                calls: Mutex::new(0),
            })
        }
    }

    impl Tokenizer for MockTokenizer {
        fn decode(&self, ids: &[u32]) -> Result<String> {
            *self.calls.lock().unwrap() += 1;
            let mut out = String::new();
            for id in ids {
                let piece = self
                    .vocab
                    .get(id)
                    .ok_or_else(|| Error::Asr(format!("unknown id {id}")))?;
                // SentencePiece-style: `▁` marks word boundaries → space.
                let rendered = piece.replace('▁', " ");
                out.push_str(&rendered);
            }
            // Strip a single leading space, mirroring what real SentencePiece
            // tokenizers do with `clean_up_tokenization_spaces`.
            Ok(out.strip_prefix(' ').unwrap_or(&out).to_string())
        }
    }

    fn vocab() -> Arc<MockTokenizer> {
        MockTokenizer::new(&[
            (1, "▁hello"),
            (2, "▁world"),
            (3, "!"),
            (4, "▁meeting"),
            (5, "ior"),
        ])
    }

    #[test]
    fn push_returns_only_the_new_text() {
        let mut dec = StreamingDecoder::new(vocab());
        assert_eq!(dec.push(1).unwrap(), "hello");
        assert_eq!(dec.push(2).unwrap(), " world");
        assert_eq!(dec.push(3).unwrap(), "!");
        assert_eq!(dec.text(), "hello world!");
    }

    #[test]
    fn push_many_concatenates_correctly() {
        let mut dec = StreamingDecoder::new(vocab());
        let delta = dec.push_many(&[1, 2, 3]).unwrap();
        assert_eq!(delta, "hello world!");
        assert_eq!(dec.text(), "hello world!");
    }

    #[test]
    fn empty_push_is_empty_delta() {
        let mut dec = StreamingDecoder::new(vocab());
        assert_eq!(dec.push_many(&[]).unwrap(), "");
        assert_eq!(dec.text(), "");
        assert!(dec.is_empty());
    }

    #[test]
    fn bpe_continuation_token_extends_previous_word() {
        // Token 4 = "▁meeting", token 5 = "ior" (a BPE continuation). The
        // diff should produce just "ior" the second time.
        let mut dec = StreamingDecoder::new(vocab());
        assert_eq!(dec.push(4).unwrap(), "meeting");
        assert_eq!(dec.push(5).unwrap(), "ior");
        assert_eq!(dec.text(), "meetingior");
    }

    #[test]
    fn reset_clears_state_for_next_segment() {
        let mut dec = StreamingDecoder::new(vocab());
        dec.push_many(&[1, 2]).unwrap();
        assert_eq!(dec.text(), "hello world");
        assert_eq!(dec.len(), 2);
        dec.reset();
        assert!(dec.is_empty());
        assert_eq!(dec.text(), "");
        assert_eq!(dec.push(1).unwrap(), "hello");
    }

    #[test]
    fn unknown_token_id_propagates_error() {
        let mut dec = StreamingDecoder::new(vocab());
        let err = dec.push(999).unwrap_err();
        assert!(matches!(err, Error::Asr(_)), "got {err:?}");
    }

    #[test]
    fn tokenizer_is_called_once_per_push_call() {
        let tok = vocab();
        let mut dec = StreamingDecoder::new(tok.clone());
        dec.push(1).unwrap();
        dec.push(2).unwrap();
        dec.push(3).unwrap();
        // 3 push calls → 3 decode calls. (No internal caching beyond the
        // text diff; this is intentional simplicity.)
        assert_eq!(*tok.calls.lock().unwrap(), 3);
    }
}
