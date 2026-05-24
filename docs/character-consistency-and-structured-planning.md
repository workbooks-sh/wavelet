# Character Consistency and Structured Planning — Research Landscape

> Research date: 2026-05-23. Read-only investigation of the wavelet pipeline,
> Fountain AST, backend adapters, WORG primitives, and the 2026 state of
> reference-conditioned video generation.

---

## 1. Today's Fountain Usage

### What the pipeline actually reads

The fountain AST (`packages/fountain/src/ast.rs`) exposes seven element types:
`SceneHeading`, `Action`, `Dialogue`, `Transition`, `PageBreak`, `Section`,
`Synopsis`, `Lyric`. Within `Dialogue`, the `character` field, `extension`
(V.O./O.S.), `is_voiceover`, `is_off_screen`, `dual`, and `lines` (Text /
Parenthetical / Lyric) are all parsed and available. `SceneHeading` carries
`ie` (INT./EXT.), `location`, and `time_of_day` as structured fields.

Of this surface, the pipeline uses:

| Element | Used by | How |
|---|---|---|
| `SceneHeading` (slugline) | `duration_fit`, `storyboard/plan`, `grammar/transitions` | Shot count, establishing-shot subject, scene-boundary onset snapping |
| `Dialogue` (character + lines + is_voiceover) | `duration_fit`, `storyboard/plan` | Word count → VO seconds; speaker → camera side; audio_ref path |
| `Transition` (kind) | `grammar/transitions`, `storyboard/plan` | BPM-scaled duration, whip-pan direction alternation |
| `Action` (text) | `duration_fit`, `storyboard/plan` | Caption heuristic (≤10 words or quoted); split-shot trigger at >40 words |

The usage is real and structural — not opaque prose treatment. SceneHeadings
drive shot count and timing math. Dialogue drives both duration budgeting and
camera-side assignment (speaker position 0 → Left, 1 → Right). Transitions
drive duration and audio-lead/trail. This is genuine AST walking, not regex.

### What is wasted

- **`Section` / `Synopsis`** — parsed, stored in elements Vec, iterated over,
  completely ignored via `_ => {}` in every match arm.
- **`PageBreak`** — same.
- **`Lyric` (top-level)** — assigned a flat 1.5s in `grammar/transitions` for
  timing accumulation but generates no shot and is not used in any planner.
- **`SceneHeading.ie` / `.location` / `.time_of_day`** — parsed out but never
  used. The planner calls `subject_from_slugline()` which re-splits the raw
  string rather than reading the structured fields.
- **`Dialogue.extension`** — parsed into the AST (`(CONT'D)`, custom
  extensions) but consumed only as a source for the boolean flags. The raw
  extension string is never forwarded downstream.
- **`Dialogue.dual`** — dual-dialogue marker parsed, never acted on.
- **`Parenthetical`** lines — correctly excluded from word-count
  (`count_dialogue_words` skips them) but never used for anything else
  (e.g. performance direction to TTS, emotion hint to image gen).
- **`TitlePage`** — `Format:` / `BPM:` metadata that appears in the eval
  fountain files lands here but nothing reads it.

### Character cues — the critical gap

CHARACTER names are the most under-leveraged element. The planner reads
`character` off each `Dialogue` element for two purposes only: camera-side
assignment and the `audio_ref` path (`vo/CHARACTER-{elem_idx}.mp3`). The
`ActionLine` labels on `SceneAnnotation` carry the character names for the
180° rule, but they are derived fresh per-scene from `current_scene_speakers`
and are not queryable across scenes.

**What is missing:** there is no concept of a canonical character registry.
A character named `NARRATOR` in scene 1 and scene 3 is two independent
`character` strings — no structure links them, no lookup says "this speaker
is the same person across the whole screenplay." This is exactly the anchor
needed for character consistency in multi-cut generation.

**Recommendation:** Add a `screenplay_characters()` helper that walks the
elements and returns a deduplicated, ordered list of `CharacterEntry` structs
(name, scene_indices, is_voiceover_only, line_count). This is a 30-line
function on the existing AST — no parser changes needed. The output becomes
the input to a future `character define` primitive.

---

## 2. Character Consistency Landscape

### What wavelet has today

The video backend layer has three distinct shapes for reference conditioning:

1. **`Img2VidRequest`** (`backends/video.rs`) — single start-frame image,
   optional `last_frame_url` for dual-keyframe. Implemented for Google Veo
   (`google/veo.rs`) via the `instances[0].image` wire field. Veo 3.1 docs
   confirm up to 3 reference frames (comment in veo.rs line 28).

2. **`MultiRefVideoRequest`** (`backends/video.rs`, `backends/replicate/wan_r2v.rs`)
   — 1–N reference images plus optional reference videos. Wan 2.7 R2V is
   the only wired adapter. Max 6 refs before subject drift degrades quality
   (per the R2V adapter comment).

3. **`RefConditionedImgRequest`** (`backends/image/ref_conditioned.rs`) —
   reference-conditioned still gen. Nano Banana 3 (`google/nano_banana.rs`)
   is the live adapter: accepts up to 8 inline image parts per request
   (`MAX_REF_IMAGES = 8`), charge $0.04/image.

**The Fal Veo adapter (`fal/veo.rs`) is text-to-video only.** It implements
`Txt2VidGenBackend` but not `Img2VidGenBackend` or any multi-ref cluster.
The wire body (`FalVeoBody`) has `prompt`, `duration`, `aspect_ratio`,
`resolution` — no image fields. This is a gap: Fal *does* expose
`fal-ai/veo3.1/reference-to-video` with an `image_urls` parameter (probed
2026-05-23 at `fal.ai/models/fal-ai/veo3.1/reference-to-video/api`).

### Current 2026 state of reference-conditioned video

**Veo 3.1 Ingredients-to-Video** (Google direct, January 2026): accepts up
to 4 reference images, maintains subject identity across scene changes.
"Locking in" a character from multiple reference angles is the advertised
use-case. The Google AI Studio UI exposes this; the Gemini API / Vertex AI
path uses the existing `instances[0].image` multi-part wire format (Veo 3.1
accepts 3 frames per the wavelet veo.rs comment). Quality for face+wardrobe
consistency is materially better than Veo 3.0 but still degrades over 3+
clips without a re-anchor strategy.

**Fal Veo 3.1 reference-to-video** (`fal-ai/veo3.1/reference-to-video`):
separate endpoint from the text-only `fal-ai/veo3`. Accepts `image_urls`
(list of strings), `aspect_ratio`, `duration`, `resolution`, `generate_audio`.
Example shows 3 reference images. This endpoint is NOT wired in wavelet yet.
Implementing it is low-friction: the existing Fal queue polling infrastructure
in `fal/veo.rs` is reusable; the body just needs an `image_urls` field added.

**Wan 2.7 R2V** (Replicate): already wired in wavelet. Accepts 1–6 reference
images (subject refs + control signals like depth/canny as peers). Useful for
character consistency but requires manually hosting reference images at
reachable URLs. Less photorealistic than Veo for human faces; better for
artistic styles.

**Higgsfield Soul ID**: training-based approach — upload 20+ photos, train a
character model (~5 minutes), get a `character_id` usable across all
subsequent generations. Cost ~$2.50 per character. Not currently in wavelet.
Better for campaigns where the same actor appears across many batches; overkill
for a single 15–25s spot.

**Seedance 2.0** (ByteDance): accepts image + video references in one pass.
Strong UGC-style consistency. Available on Fal (`fal-ai/seedance-2.0`).
The Seedance Replicate adapter (`backends/replicate/seedance.rs`) exists in
wavelet — check if it surfaces `reference_images`.

**Hand cutaway problem:** none of the above tools solve hand consistency out
of the box. A character reference image anchors face, hair, wardrobe —
extremities are incidental. For UGC "hands-holding-product" cutaways, the
model generates a new hand every time. The industry workaround is a separate
reference for the hands segment: one close-up reference image that shows the
same skin tone, nail polish, and ring situation, used only for ECU cutaway
shots. This needs to be a *separate* `character_ref` entry in wavelet — not
the same entity as the full-body actor reference.

### Recommended primitive

Minimum viable path for the UGC character-to-camera use case:

1. **`wavelet character define <name> --reference <image> [--reference <image>]`**
   Emits a `.clip.html` file of kind `character-ref` under
   `refs/character/<name>.clip.html`. Stores: name (keyed to Fountain
   CHARACTER string), list of reference image paths, optional `soul_id`
   (for Higgsfield), optional `character_type` (`full-body` | `hands` |
   `product-hands`).

2. **`character_ref` field on `Shot`** in `storyboard.rs`. When present,
   the shot's generation strategy routes to `Img2VidGenBackend` (first
   reference image as start-frame) or `MultiRefVideoRequest` (all refs),
   instead of `Txt2Vid`.

3. **Storyboard planner hook**: when a `Dialogue` element has a character
   name that matches a defined `character_ref`, auto-set the shot's
   `character_ref` instead of the default `StockSearch`. The 30-line
   `screenplay_characters()` helper from section 1 is the bridge.

4. **`wavelet character define hands <name>-hands --reference <image>`** as
   a *distinct* primitive. Hand cutaways are ECU shots; they route through
   the same `MultiRefVideoRequest` cluster but the prompt explicitly cues
   ECU framing with product. Keeping them as a separate entity rather than
   an attribute of the full-body ref avoids polluting the face-conditioning
   signal with extremity pixels.

---

## 3. WORG Fit

### What WORG is

WORG is a planning DSL and graph executor built on standard org-mode. Its
mental model: headlines are tasks, properties carry structured metadata
(`:DEPENDS_ON:`, `:BUDGET:`, `:ARTIFACT:`, `:KIND:` for validators, `:TOOL:`
for tool dispatches), source blocks hold executable payloads (shell, lua,
fountain), and `:LOGBOOK:` records state transitions. It is a library, not
a daemon — the calling agent embeds it.

The `mini-coffee-mapped.org` example in `packages/worg/examples/` shows the
complete wavelet commercial pipeline expressed as a WORG document:
`Stage 1 → Brief`, `Stage 2 → Script (fountain source block inside org)`,
`Stage 3 → Velocity`, `Stage 4 → Storyboard`, etc. Every inter-stage
dependency is expressed as `:DEPENDS_ON: [[id:stage-N]]`. Validators are
child headlines with `:KIND: screenplay_parse_clean`, `:KIND:
storyboard_verify_passes`, etc. The validator KIND registry in `w.org` already
includes `screenplay_parse_clean` and `storyboard_verify_passes`.

### Fountain vs WORG — overlap and complement

| Concern | Fountain | WORG |
|---|---|---|
| Scene structure | Native — SceneHeading is the unit | Not native; can embed `#+begin_src fountain` |
| Shot plan | None — Fountain is a dialogue/direction medium | Can represent the storyboard as an outline (`Stage N — asset`) |
| Dependency graph | None | First-class (`:DEPENDS_ON:`) |
| Budget tracking | None | `:BUDGET_USD:` / `:COST_USD:` per tool call |
| Agent-side validation | None | `:validator:` headlines with `:KIND:` registry |
| State machine | None | TODO/DOING/DONE/FAILED per headline with `:LOGBOOK:` |
| Character definitions | None (cue strings only) | Could hold a `* Characters :input:` section with `:PROPERTIES:` per character |
| Re-usable prompts | None | `#+begin_src markdown :tangle strategy.md` for tangling |
| Industry-standard authoring | Yes (FDR, Highland, Arc Studio, Emacs Fountain) | No — org-mode is a niche authoring tool |

WORG does not replace Fountain's authoring role. No director rewrites a
screenplay in org-mode. But WORG is exactly right for what surrounds the
screenplay: the plan, the character registry, the budget gate, the validated
dependency chain from brief to MP4.

### Recommended pattern: WORG as the wrapper

The correct pattern is **WORG-as-wrapper, Fountain-in-source-block**:

```org
* DONE [#A] Stage 2 — script  :stage:
:PROPERTIES:
:ID:              stage-script
:DEPENDS_ON:      [[id:stage-research]]
:ARTIFACTS_OUT:   script.fountain
:END:

** Characters  :input:
:PROPERTIES:
:CHAR_ALEX:  full-body  refs/character/alex.clip.html
:CHAR_HANDS: hands      refs/character/alex-hands.clip.html
:END:

** Tool: fs.write script.fountain
#+begin_src fountain :tangle script.fountain
INT. KITCHEN - DAY

ALEX
She picks up the stand mixer.

CUT TO:
...
#+end_src

** DONE Validator: screenplay_parse_clean
:PROPERTIES:
:KIND:    screenplay_parse_clean
:ARG_PATH: script.fountain
:END:
```

This pattern gives the agent:
- A machine-checkable plan with dependencies and state
- The screenplay embedded as a first-class tangle-able source block
- A `Characters :input:` section that can carry `character_ref` properties
  the pipeline reads before dispatching video gen
- All wavelet validator kinds already registered in `w.org`

The cost of this pattern: the agent must author an `.org` file instead of a
bare `.fountain`. For an autonomous agent that is already writing JSON and
markdown, the overhead is one extra structure layer. For a human director
using Highland or FDR, it would be a friction point — but the human-authored
Fountain file can always be imported *into* the WORG wrapper (embed as a
source block, tangle trivially).

**What the agent gains:** cross-session resumability (WORG state persists in
the `.org` file), structured character registry visible to both the pipeline
and the human reviewer, cost tracking per stage, and the ability for a future
`wavelet plan verify` to confirm all validators pass before spending money on
Veo.

---

## 4. Recommended Path Forward

**First: wire Fal Veo 3.1 reference-to-video.** This is the lowest-cost
unlock. The queue polling infrastructure in `fal/veo.rs` is reusable; the
only change is a new `FalVeoRefBody` struct with `image_urls: Vec<String>` and
an `Img2VidGenBackend` impl on `FalVeoAdapter`. Once wired, the eval 009
`character_consistency` dimension can be tested with real reference conditioning
instead of identity-drift mitigation through prompt engineering.

**Second: add the `screenplay_characters()` extractor and the `character define`
primitive.** The extractor is pure library code — no CLI changes, 30 lines.
The `character define` command writes a `character-ref` clip HTML stub to
`refs/character/`. The storyboard planner hook that auto-routes `Dialogue`
shots through `Img2Vid` when a character_ref exists is the third piece. These
three together constitute the minimum viable character-consistency primitive
and close the gap that eval 009's `character_consistency` dimension exposes.

**Third: prove with eval 010 (UGC character-to-camera with hand cutaways).**
This is the acid test for both the ref-image primitive and the hands
separation. Run it first with Fal Veo 3.1 reference-to-video, note the drift
score, then compare against Wan R2V (already wired) as a cost-quality
tradeoff. Once the character_consistency dimension scores ≥ 2 reliably, the
WORG-as-wrapper authoring pattern can be introduced as a second eval variant
to measure the authoring overhead.

The WORG integration is the right long-term structure — but it is a
productivity/resumability improvement, not a blocking correctness requirement.
Ship the character primitive first, prove it works in the eval, then layer
the WORG plan wrapper on top as the authoring standard for the UGC persona
workflow.

---

## 5. Suggested Eval: 010-ugc-character-to-camera

```
eval id:  wavelet/010-ugc-character-to-camera
format:   9:16 (Reels/TikTok)
duration: 18s
budget:   $9.00
```

**Brief (agent receives this verbatim):**

> Make me a skincare brand spot for a fictional product called "Dew Drop
> Serum." 9:16 for Instagram Reels, 18 seconds. Style: UGC creator-to-camera
> — a young woman talking directly to the phone, conversational, not
> commercial. She applies the product in one shot, holds the bottle in another,
> then looks at her reflection in a third. All three shots should be the SAME
> PERSON — same face, same skin tone, same manicure. Add one isolated
> close-up of just her hands holding the amber bottle (this is the ONLY shot
> where you don't need to see her face). Use HTML overlays for the brand name
> and the CTA "dewdropserum.com" — not baked into the video clips. Budget nine
> dollars.

**Rubric dimensions (0–3 each, threshold 2 per dimension, sum ≥ 18/24):**

1. **character_face_consistency** — the three face-forward shots show the same
   woman (skin tone, hair, face structure consistent across cuts; transcript
   shows `character define` or Ingredients-to-Video used)
2. **hand_shot_separate** — the close-up hands shot exists; transcript shows
   a *separate* reference was used (or an explicit ECU cutaway strategy noted);
   hands shot uses the same nail/skin tone as the face shots
3. **product_visible** — the amber bottle appears in at least 2 shots; it is
   identifiable as a serum bottle (not generic)
4. **ugc_register** — frames read as creator-to-camera (phone-distance framing,
   home/bathroom environment, natural light, direct gaze); NOT fashion
   editorial, NOT beauty-commercial polish
5. **html_text_only** — brand name and CTA are HTML overlays, no text baked
   into Veo clips; `wavelet lint --mp4` passes contrast checks on text scenes
6. **duration** — rendered MP4 is 16–20 seconds
7. **pipeline_discipline** — `screenplay validate` called before storyboard;
   `wavelet lint --mp4` called before compose; no hand-authored JSON artifacts
8. **final_artifact** — MP4 renders without broken frames; composition plays
   to completion

**What this eval proves:** (a) the `character define` + `MultiRefVideoRequest`
path works end-to-end for human faces; (b) the hands-reference separation is
a first-class workflow step, not a workaround; (c) the pipeline can maintain
UGC register at 18 seconds without drifting to editorial stock; (d) the
budget ceiling is tight enough to penalize over-generation.

**Failure modes to watch:** identity drift between face shots despite ref
conditioning (score `character_face_consistency` 0 if the woman changes);
agent using a generic stock hand for the ECU instead of conditioning on a
reference (score `hand_shot_separate` 0); agent baking brand text into Veo
clips (score `html_text_only` 0).
