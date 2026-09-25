// "Learn Kitsu": the loop in eight steps, each pointing at the real thing.
// Steps that ask for an action wait for it; Next does it for you where that's
// harmless (opening a page), never where it isn't (accepting a real run).

import { app } from "./app.svelte";
import { t } from "./i18n/index.svelte";
import type { TourStep } from "./tour.svelte";

const kbd = (k: string) => `<kbd>${k}</kbd>`;

/** A run worth reviewing: the open task's, or the first one waiting. */
function reviewTask() {
  const open = app.view.kind === "task" ? app.task(app.view.id) : undefined;
  if (open?.status.kind === "review" && open.status.verdict !== "empty") return open;
  return app.overview?.tasks.find((x) => x.status.kind === "review" && x.status.verdict !== "empty");
}

function showReview() {
  const task = reviewTask();
  if (task && !(app.view.kind === "task" && app.view.id === task.id)) app.openTask(task.id);
}

let acceptedBefore = 0;

export const kitsuTour: TourStep[] = [
  {
    id: "needs",
    target: ["rail-needs", "home-needs", "strip"],
    place: "right",
    fox: "asking",
    title: () => t("tour.needs.title"),
    body: () => t("tour.needs.body"),
    enter: () => {
      if (app.view.kind !== "home") app.go({ kind: "home" });
    },
  },
  {
    id: "open",
    target: ["rail-needs", "home-needs"],
    place: "right",
    title: () => t("tour.open.title"),
    body: () => t("tour.open.body", { j: kbd("j"), k: kbd("k"), enter: kbd("⏎") }),
    done: () => app.view.kind === "task",
    skip: () => {
      const task = reviewTask() ?? app.visibleTasks()[0];
      if (task) app.openTask(task.id);
    },
  },
  {
    id: "review",
    target: ["review"],
    place: "bottom",
    title: () => t("tour.review.title"),
    body: () => t("tour.review.body"),
    when: () => !!reviewTask(),
    enter: showReview,
  },
  {
    id: "accept",
    target: ["review-actions"],
    place: "bottom",
    fox: "asking",
    title: () => t("tour.accept.title"),
    body: () => t(app.preview ? "tour.accept.body" : "tour.accept.bodyReal", { a: kbd("a") }),
    when: () => !!reviewTask(),
    enter: () => {
      showReview();
      acceptedBefore = app.accepted;
    },
    done: () => app.accepted > acceptedBefore,
  },
  {
    id: "rules",
    target: ["rules"],
    place: "bottom",
    title: () => t("tour.rules.title"),
    body: () => t("tour.rules.body"),
    enter: () => app.go({ kind: "rules" }),
  },
  {
    id: "code",
    // Before the switch, the layout toggle; after it, the agents panel it brings.
    target: ["strip", "modes"],
    place: "left",
    title: () => t("tour.code.title"),
    body: () => t("tour.code.body", { code: kbd("⌘2"), work: kbd("⌘1") }),
    done: () => app.mode === "code",
    stay: true,
    skip: () => (app.mode = "code"),
  },
  {
    id: "palette",
    // Floats until the palette is open, then points at it.
    target: ["palette"],
    place: "right",
    title: () => t("tour.palette.title"),
    body: () => t("tour.palette.body", { key: kbd("⌘K") }),
    done: () => app.overlay === "palette",
    stay: true,
  },
  {
    id: "done",
    place: "center",
    dim: true,
    fox: "happy",
    title: () => t("tour.done.title"),
    body: () => t("tour.done.body", { key: kbd("?") }),
  },
];
