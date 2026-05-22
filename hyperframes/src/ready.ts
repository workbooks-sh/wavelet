// Scene-side helpers for hooking into the wavelet runtime's playhead.
//
// Re-exports the runtime's onReady / onTick under the wavelet-
// hyperframes name so scene <script> blocks can import from the
// authoring SDK rather than the runtime registry.
//
//   <gm-scene id="hero" start="0s" duration="3s">
//     <h1 class="title">Hello</h1>
//     <script type="module">
//       import { onReady } from "@work.books/wavelet-hyperframes/ready";
//       onReady("hero", () => {
//         gsap.from(".title", { y: 60, opacity: 0, duration: 0.6 });
//       });
//     </script>
//   </gm-scene>

export {
  onReady,
  onTick,
  type ReadyDetail,
  type TickDetail,
} from "@work.books/wavelet-runtime/events";
