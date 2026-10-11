---
"herdr-command-palette": patch
---

Make `qwen3.5:4b` the default model for `✨ Auto generate`, about twice as fast as `qwen3.5:9b`; pull it with `ollama pull qwen3.5:4b`, or keep the old model by setting `model = "qwen3.5:9b"` under `[auto_name]` in `settings.toml`
