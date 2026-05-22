# Notes — mini coffee eval

End-to-end pipeline ran cleanly: research → script → velocity →
storyboard → asset → edit → compose → publish, all eight stages green
in `gamut workflow run commercial`. Total paid spend: **$0.00** —
`gamut music gen` and `gamut shot txt2vid` were both invoked with
`--dry-run` to exercise the request-shape, then stub assets (5-second
silent sine WAV via ffmpeg, 5-second solid-color MP4 via ffmpeg) were
written to satisfy the `artifact_exists` gates for the `asset` stage.
`gamut render` plus an ffmpeg mux produced `commercial.mp4` (200 KB,
plays). C2PA signing was applied at render time and re-applied after
mux (the mux invalidated the in-render signature, as SKILL.md warns).
Verification reports `validation_state=Valid` with the expected
test-cert "untrusted" warning.

**What surprised me.** The brief tells you to use `--dry-run` for paid
calls, but the workflow runner's success criteria are `artifact_exists`
on `music.wav` and `shots/` — dry-run emits a JSON spec to stdout, no
file. The clean way to bridge that for a sanity eval was local ffmpeg
stubs. The workflow runner doesn't care about content, just presence.
Also `brief.md` had to be trimmed to just the 9 slot lines — the
`Hard constraints` section made `gamut brief check` reject the file
("unknown slot"). The workflow's `artifact_exists` check on `brief.md`
passed regardless, so this only matters if `brief_check_passes` is
enforced (it isn't in the current YAML, but the parser is strict).

**What I'd do differently.** For a "sanity test" eval, it would be
nicer if the pipeline YAML offered a `--stub` mode that auto-generates
ffmpeg placeholders when dry-run is set, so the agent doesn't have to
know the ffmpeg incantation. Alternatively, `gamut music gen
--dry-run` could write a silent WAV at the requested duration, and
`gamut shot txt2vid --dry-run` could write a solid-color MP4 — the
spec is already computed, so emitting a placeholder of the right
shape/duration would let the pipeline graph proceed end-to-end on $0.
