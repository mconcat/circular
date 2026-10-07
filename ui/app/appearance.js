(() => {
  const root = document.documentElement,
    params = new URLSearchParams(location.search);
  root.dataset.theme = params.get("theme") === "light" ? "light" : "dark";
  function paint() {
    const css = getComputedStyle(root),
      get = (n) => css.getPropertyValue("--" + n).trim(),
      rgb = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
    window.StudyPaint = {
      data: get("data"),
      grid: get("chart-grid"),
      history: get("history-data"),
      rest: get("chart-rest"),
      incident: get("red"),
      emissionCore: get("hf-05-core"),
      emissionHalo: get("hf-05-halo"),
      wire: rgb(get("data")),
      wireSelected: rgb(get("blue")),
    };
  }
  const A = (window.Appearance = {
    async apply(theme = root.dataset.theme) {
      root.dataset.theme = theme === "light" ? "light" : "dark";
      paint();
      const url = new URL(location.href);
      url.searchParams.set("theme", root.dataset.theme);
      history.replaceState(null, "", url);
      const sans = "Circular Pretendard",
        mono = "Circular Geist Mono";
      await Promise.all(
        [400, 500, 600].flatMap((w) => [
          document.fonts.load(`${w} 14px "${sans}"`, "Circular 0123456789"),
          document.fonts.load(`${w} 14px "${sans}"`, "한글 입력값"),
          document.fonts.load(`${w} 12px "${mono}"`, "0O 1lI"),
        ]),
      );
      if (window.StudyApp) {
        StudyApp.updateViewers(StudyApp.displayTime(), true);
        StudyApp.timeMachine.draw();
        StudyApp.field.draw(StudyApp.displayTime(), 0);
      }
      document.dispatchEvent(new CustomEvent("appearancechange"));
    },
    open() {
      Product.dialog(
        "Appearance",
        `<p>Theme changes apply immediately to this window.</p><div class="appearance-options"><fieldset><legend>Font stack</legend><p>Pretendard + Geist Mono</p><small>Locally bundled fonts. 400 body · 500 controls · 600 headings.</small></fieldset><fieldset><legend>Theme</legend><label><input type="radio" name="appearance-theme" value="light" ${root.dataset.theme === "light" ? "checked" : ""}> Light · warm mineral</label><label><input type="radio" name="appearance-theme" value="dark" ${root.dataset.theme === "dark" ? "checked" : ""}> Dark · graphite</label></fieldset></div><div class="appearance-proof"><p>Live program · Current observation 0123456789</p><p><span style="font-weight:400">Body Regular</span> · <span style="font-weight:500">Controls Medium</span> · <span style="font-weight:600">Selected actor Semibold</span></p><code>0O · 1lI · event.value * 2 → result_01</code><p>Changing appearance keeps your selection, draft and canvas camera.</p></div><p role="status">Device preference saving is not available yet.</p><div class="dialog-actions"><button class="dark-button" data-product-close>Done</button></div>`,
      );
    },
  });
  document.addEventListener("click", (ev) => {
    if (ev.target.closest("[data-appearance]")) A.open();
  });
  document.addEventListener("change", (ev) => {
    if (ev.target.name === "appearance-theme") A.apply(ev.target.value);
  });
  document.addEventListener("DOMContentLoaded", () => A.apply());
})();
