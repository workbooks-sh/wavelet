// Apply / compose <gm-adjustment> filters on the viewport.
//
// Multiple active adjustments compose by concatenating their filter
// strings (space-separated, the CSS way). backdrop-filter and
// mix-blend-mode are last-write-wins — if two adjustments both set
// `blend=`, the higher-z one wins.

import type { ResolvedAdjustment } from "./types";

export interface AdjustmentApplicator {
  /** Recompute composed filter/backdrop/blend from the active set. */
  apply(active: ResolvedAdjustment[]): void;
  /** Clear all applied styles. */
  reset(): void;
}

export function createAdjustmentApplicator(viewport: HTMLElement): AdjustmentApplicator {
  return {
    apply(active): void {
      if (active.length === 0) {
        viewport.style.filter = "";
        viewport.style.backdropFilter = "";
        viewport.style.mixBlendMode = "";
        return;
      }
      // Compose filters in z order (caller passes them already sorted).
      const filters = active.map((a) => a.filter).filter(Boolean);
      viewport.style.filter = filters.join(" ");
      const backdrop = lastTruthy(active.map((a) => a.backdrop));
      viewport.style.backdropFilter = backdrop ?? "";
      const blend = lastTruthy(active.map((a) => a.blend));
      viewport.style.mixBlendMode = blend ?? "";
    },
    reset(): void {
      viewport.style.filter = "";
      viewport.style.backdropFilter = "";
      viewport.style.mixBlendMode = "";
    },
  };
}

function lastTruthy<T>(values: (T | undefined)[]): T | undefined {
  for (let i = values.length - 1; i >= 0; i--) {
    if (values[i]) return values[i];
  }
  return undefined;
}
