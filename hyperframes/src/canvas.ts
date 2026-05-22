// Design-canvas constants.
//
// Every HyperFrames scene authors at this fixed resolution. The wavelet
// runtime mounts the scene inside a 1920×1080 viewport and
// transform:scale-s the viewport to fit the actual window. Authors
// write absolute-pixel CSS at this canvas size and the result fits any
// screen without media queries.

export const CANVAS_WIDTH = 1920;
export const CANVAS_HEIGHT = 1080;
export const CANVAS_ASPECT = `${CANVAS_WIDTH} / ${CANVAS_HEIGHT}`;

/**
 * Project a fraction (0..1) of the canvas to absolute pixels. Useful
 * when an author wants their layout to read in ratios but the runtime
 * to receive concrete numbers.
 *
 *   px(0.5, "x") // → 960
 *   px(0.33, "y") // → 356.4
 */
export function px(fraction: number, axis: "x" | "y"): number {
  return fraction * (axis === "x" ? CANVAS_WIDTH : CANVAS_HEIGHT);
}
