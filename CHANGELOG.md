# Changelog

## 0.7.0

### Minor Changes

- c0bd7bc: Add creating, opening and removing Git worktrees to the command palette, picking from the worktrees of the repository behind the current workspace

### Patch Changes

- 59d151b: Cap the status line at four rows, ending a longer message with `...`, so a long herdr error can no longer push the candidates off the palette
- 48fa1e0: A rejected `icon` override is named by its code points (`U+25A1`) in its `skipped` row instead of being drawn, so the row no longer shows the very glyph it rejects
- 9097d90: The palette no longer draws East Asian Ambiguous characters, which a CJK-locale terminal draws two cells wide: shipped titles end in `...` instead of `…`, separators are `⋅` instead of `·`, the input cursor is `⎸` instead of `▏`, and status notes use `-` instead of `—`
- cc4b46a: The selected row is marked with `▸` instead of `▶`, so a terminal that draws East Asian Ambiguous characters two cells wide no longer shifts that row one cell out of line
- 43ae1e6: On herdr 0.9, pressing the palette key while a popup is already open again says so and points to `Esc`, instead of printing herdr's raw error
- 810fc93: Show what herdr wrote to stderr, or the JSON it returned, when its answer carries neither a result nor an error, instead of only "herdr returned no result"
- 195396c: All three startup notes now fit the popup at once, down to the 51 columns a narrow terminal clamps it to, when the settings error is short, such as a misspelt key: the unusable-settings note names `settings.toml` instead of its full path, and the outdated-herdr note reads `herdr <version> < catalog <version>`. A longer settings error still ends in `...`

## 0.6.0

### Minor Changes

- 0b38d07: Add pane resizing in all four directions to the command palette

### Patch Changes

- fa67835: Skip a catalog `icon` that a CJK-locale terminal draws two cells wide, such as `□`, instead of letting it push its row's title out of line

## 0.5.0

### Minor Changes

- 6c7991e: Lead each palette row with an icon for what it acts on — pane, tab, workspace, another plugin's action — so the list can be scanned by shape; set `icon` on a catalog entry to change one, or `icons = false` in `settings.toml` to turn them off

### Patch Changes

- 0d0ce78: Say so when the popup is shorter than 6 rows instead of drawing a list too short to use; only Esc and Ctrl-C act until it grows back

## [0.4.0](https://github.com/nemolize/herdr-plugin-command-palette/compare/v0.3.1...v0.4.0) (2026-09-28)


### Features

* add vertical pane swaps to the command palette ([335ec60](https://github.com/nemolize/herdr-plugin-command-palette/commit/335ec605267aa4884c7fdd1f91b12fcfe01cdd47))

## [0.3.1](https://github.com/nemolize/herdr-plugin-command-palette/compare/v0.3.0...v0.3.1) (2026-09-23)


### Bug Fixes

* pass --plugin after the action id on plugin action invoke ([bab2c4d](https://github.com/nemolize/herdr-plugin-command-palette/commit/bab2c4d1a9b3411ea2f844c809ff58183e8b3496))

## [0.3.0](https://github.com/nemolize/herdr-plugin-command-palette/compare/v0.2.0...v0.3.0) (2026-09-12)


### Features

* rename workspaces, tabs and panes from the palette ([5226ecf](https://github.com/nemolize/herdr-plugin-command-palette/commit/5226ecf9efe56ec676b796d81079cf294de22290))
* show each command's keybinding beside it in the palette ([9fada86](https://github.com/nemolize/herdr-plugin-command-palette/commit/9fada865138050e89c3c99414957d46c6157950e))


### Bug Fixes

* count the status row by rendering it, and drive the harness on conditions ([9d5e5a6](https://github.com/nemolize/herdr-plugin-command-palette/commit/9d5e5a612384ad68737d12297073b5dae06a73ab))
* judge the palette's exit status and liveness, not just that it ended ([837202c](https://github.com/nemolize/herdr-plugin-command-palette/commit/837202cc0b6aa868fd8d10b309eac0280f434bba))
* read the keys herdr actually binds, not the ones its template lists ([737435f](https://github.com/nemolize/herdr-plugin-command-palette/commit/737435f8113feebe05ca5056fc1af49efe5f011f))
* report a failed dispatch in the palette, and test the path that runs a pick ([a37ad1c](https://github.com/nemolize/herdr-plugin-command-palette/commit/a37ad1c3a8a5166feeb75db4836e4986f2bcc34e))
* sweep the whole process group on close, and trim restating comments ([d192927](https://github.com/nemolize/herdr-plugin-command-palette/commit/d1929271e00a41826d0e3990d8a7fcf9aea363d8))

## [0.2.0](https://github.com/nemolize/herdr-plugin-command-palette/compare/v0.1.0...v0.2.0) (2026-09-05)


### Features

* cut releases by merging a release PR instead of hand-tagging ([df2b9a6](https://github.com/nemolize/herdr-plugin-command-palette/commit/df2b9a669f0f42a9f56a6310ae5ecc75090c21aa))
* show the running version at the footer's right edge ([#37](https://github.com/nemolize/herdr-plugin-command-palette/issues/37)) ([4087c73](https://github.com/nemolize/herdr-plugin-command-palette/commit/4087c73d1464ebbf71f56a327e29cc87e331c613)), closes [#36](https://github.com/nemolize/herdr-plugin-command-palette/issues/36)


### Bug Fixes

* close the E2E harness's silent-pass and leak paths ([edb630a](https://github.com/nemolize/herdr-plugin-command-palette/commit/edb630a50a895823e6830661101919a0790c02ed))
* stop drawing a second frame inside the pane ([39ca567](https://github.com/nemolize/herdr-plugin-command-palette/commit/39ca5679f6d3bec8786c0af39d12b2fee397039d))
