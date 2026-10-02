---
"herdr-command-palette": patch
---

The palette no longer draws East Asian Ambiguous characters, which a CJK-locale terminal draws two cells wide: shipped titles end in `...` instead of `…`, separators are `⋅` instead of `·`, the input cursor is `⎸` instead of `▏`, and status notes use `-` instead of `—`
