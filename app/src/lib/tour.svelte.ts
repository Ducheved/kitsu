// A guided tour: coach marks over real elements, found by their
// data-tour="…" attribute. The component (Tour.svelte) knows nothing about
// Kitsu; the steps (tour-steps.ts) do.

import type { FoxState } from "../components/Fox.svelte";

export interface TourStep {
  id: string;
  /** data-tour names to point at; the first one on screen wins. None: the popover floats. */
  target?: string[];
  /** Where the popover goes: beside the target, or, without one, centered / near the bottom. */
  place?: "right" | "left" | "top" | "bottom" | "center";
  title: () => string;
  /** HTML. Only our own strings go here, with keys wrapped in <kbd>. */
  body: () => string;
  fox?: FoxState;
  /** Dim the page even without a target. */
  dim?: boolean;
  /** Left out when this says no (e.g. nothing to review). */
  when?: () => boolean;
  /** Get the screen ready: open the page the step talks about. */
  enter?: () => void;
  /** The user did what the step asks. The tour moves on by itself unless `stay`. */
  done?: () => boolean;
  stay?: boolean;
  /** What Next does for someone who didn't do it themselves. */
  skip?: () => void;
}

const SEEN = "kitsu.tour";

class Tour {
  open = $state(false);

  start() {
    this.open = true;
    try {
      localStorage.setItem(SEEN, "seen");
    } catch {
      // Storage off: the tour works, it just can't remember it ran.
    }
  }

  stop() {
    this.open = false;
  }

  /** Nobody has seen the tour in this browser yet. Storage that throws counts as a first visit. */
  unseen(): boolean {
    try {
      return localStorage.getItem(SEEN) === null;
    } catch {
      return true;
    }
  }
}

export const tour = new Tour();
