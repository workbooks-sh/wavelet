# clip-ref-arizona

Same composition as `examples/arizona/`, restructured around clip-refs (wb-n33n.10).

Differences from `arizona/`:

- No `comp.json` — the manifest is `index.html` (parsed by `wavelet::compose::load_index_html`).
- No `video_bg` paths on scenes — each scene HTML references its background clip via `<wavelet-clip src="../refs/shot/<…>.clip.html">`.
- Music is also a clip-ref. The `scenes/road.html` carries a `<wavelet-clip src="../refs/music/<…>.clip.html">`; the compose pre-pass hoists it into the composition's `audio_cues` automatically.

The asset binaries (`assets/saguaro.mp4`, `assets/canyon.mp4`, …, `music.mp3`) are expected to be the same files as in `examples/arizona/`. Symlink or copy them in:

```
cd packages/wavelet/examples/clip-ref-arizona
ln -s ../arizona/assets assets
ln -s ../arizona/music.mp3 music.mp3
```

Render:

```
wavelet render examples/clip-ref-arizona/index.html out.mp4
```

The result is frame-identical to `examples/arizona/`.
