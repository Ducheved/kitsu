// Runs before the first paint so a dark window never flashes white.
// The app sets the same attribute (and keeps it in sync) once it starts.
(function () {
  var pref = "system";
  try {
    pref = localStorage.getItem("kitsu.theme") || "system";
  } catch (e) {}
  var dark = pref === "dark" || (pref === "system" && window.matchMedia && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
})();
