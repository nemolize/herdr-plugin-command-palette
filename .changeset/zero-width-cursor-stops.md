---
"herdr-command-palette": patch
---

Show every cursor stop in the input line: a zero-width character in a name is drawn as `◌`, a combining mark separated from its base by the cursor stays visible, and a `\r\n` pair is never clipped in half
