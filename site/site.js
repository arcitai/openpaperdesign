const root = document.documentElement;
const toggle = document.querySelector("#theme-toggle");
const system = matchMedia("(prefers-color-scheme: dark)");
let preference;
try {
  preference = localStorage.getItem("openpaperdesign-site-theme");
} catch {
  /* Storage is optional. */
}
if (preference !== "dark" && preference !== "light") preference = null;
function applyTheme(theme) {
  root.dataset.theme = theme;
  toggle.setAttribute(
    "aria-label",
    `Switch to ${theme === "dark" ? "light" : "dark"} theme`,
  );
}
applyTheme(
  preference === "dark" || preference === "light"
    ? preference
    : system.matches
      ? "dark"
      : "light",
);
toggle.addEventListener("click", () => {
  preference = root.dataset.theme === "dark" ? "light" : "dark";
  applyTheme(preference);
  try {
    localStorage.setItem("openpaperdesign-site-theme", preference);
  } catch {
    /* Theme still works. */
  }
});
system.addEventListener("change", () => {
  if (!preference) applyTheme(system.matches ? "dark" : "light");
});
if (["localhost", "127.0.0.1"].includes(location.hostname)) {
  document.querySelectorAll("[data-arcitai]").forEach((link) => {
    link.href = "http://127.0.0.1:4183/";
  });
}
