# Meetior

Local-first meeting assistant for Google Meet on macOS. Transcribes meetings
with **Parakeet TDT v3** (ONNX), summarizes and extracts todos with a local
**Ollama** model. No audio or text leaves your machine.

> **Status:** foundation only. Audio capture and ASR are stubbed; storage,
> summarization, and the UI are wired end-to-end. See [`CLAUDE.md`](./CLAUDE.md)
> for architecture and roadmap.

## Quick start (macOS 13+)

```sh
brew install pnpm onnxruntime ollama
ollama pull llama3.1:8b-instruct-q4_K_M
pnpm install

export ORT_DYLIB_PATH=$(brew --prefix onnxruntime)/lib/libonnxruntime.dylib
pnpm tauri dev
```

For the full setup and roadmap see `CLAUDE.md`.
