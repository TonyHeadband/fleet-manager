# Bundled fonts

All SIL Open Font License 1.1; each family's licence is the `LICENSE-*.txt` beside it. Every file is a static
instance (no `fvar` table) except the two `*-Variable.ttf` mono files, which only the web UI loads for its mono weight:
egui draws a variable font only at its default instance, so it could not stand in for Medium, and keeps the Regular
files. Settings › General picks the UI font and the mono font separately.

| File | Source | Version |
|---|---|---|
| `IBMPlexSans-Regular.ttf`, `IBMPlexSans-Medium.ttf`, `IBMPlexMono-Regular.ttf` | https://github.com/IBM/plex | bundled before this README; licence `LICENSE-IBMPlex.txt` |
| `SpaceGrotesk-Regular.ttf`, `SpaceGrotesk-Medium.ttf` | https://github.com/floriankarsten/space-grotesk/releases/download/2.0.0/SpaceGrotesk-2.0.0.zip, `ttf/static/` | 2.0.0 |
| `IBMPlexMono-Medium.ttf`, `IBMPlexMono-SemiBold.ttf` | https://github.com/google/fonts, `ofl/ibmplexmono/` (web mono weight; no Bold file: the heaviest step uses SemiBold) | google/fonts main, fetched 2026-11-01; licence `LICENSE-IBMPlex.txt` |
| `JetBrainsMono-Variable.ttf` | https://github.com/google/fonts, `ofl/jetbrainsmono/JetBrainsMono[wght].ttf` (wght 100-800; web mono weight) | google/fonts main, fetched 2026-11-01 |
| `AtkinsonHyperlegibleMono-Variable.ttf` | https://github.com/google/fonts, `ofl/atkinsonhyperlegiblemono/AtkinsonHyperlegibleMono[wght].ttf` (wght 200-800; web mono weight) | google/fonts main, fetched 2026-11-01 |
| `JetBrainsMono-Regular.ttf` | https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip, `fonts/ttf/` | 2.304 |
| `AtkinsonHyperlegibleNext-Regular.ttf`, `AtkinsonHyperlegibleNext-Medium.ttf` | https://github.com/googlefonts/atkinson-hyperlegible-next, `fonts/ttf/` | commit 7925f50f649b3813257faf2f4c0b381011f434f1 |
| `AtkinsonHyperlegibleMono-Regular.ttf` | https://github.com/googlefonts/atkinson-hyperlegible-next-mono, `fonts/ttf/` | commit 154d50362016cc3e873eb21d242cd0772384c8f9 |
