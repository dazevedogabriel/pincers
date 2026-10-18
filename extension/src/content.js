// Alt+click on a link leaves that download to the browser, like IDM's
// "hold Alt to prevent downloading with IDM".
document.addEventListener(
  "click",
  (e) => {
    if (e.altKey && e.target instanceof Element && e.target.closest("a[href]")) {
      (globalThis.browser ?? globalThis.chrome).runtime.sendMessage({ type: "bypass" });
    }
  },
  true,
);
