(() => {
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const spell = (parts, millis, precision) =>
    parts.map((v) => String(v).padStart(2, "0")).join(":") +
    (precision ? "." + String(Math.floor(millis / 10) % 100).padStart(2, "0") : "");
  const history = () => window.STUDY_HISTORY ?? STUDY_HISTORY;
  const placed = (seconds) => {
    const pair = history().wallClock;
    const ms = Math.round(seconds * 1000);
    return pair && ms >= pair.atMs ? new Date(pair.wallMs + (ms - pair.atMs)) : null;
  };
  const format = (t, precision = false, from = t) => {
    const seconds = history().start + Math.max(0, t);
    const wall = placed(history().start + Math.max(0, Math.min(t, from))) ? placed(seconds) : null;
    if (wall)
      return spell([wall.getHours(), wall.getMinutes(), wall.getSeconds()], wall.getMilliseconds(), precision);
    const parts = [
      Math.floor(seconds / 3600) % 24,
      Math.floor(seconds / 60) % 60,
      Math.floor(seconds) % 60,
    ];
    const elapsed = spell(parts, Math.round(seconds * 1000), precision);
    return history().wallClock === undefined ? elapsed : "+" + elapsed;
  };
  const describe = (t) => {
    const seconds = history().start + Math.max(0, t);
    const run = `run clock +${seconds.toFixed(3)} s`;
    const wall = placed(seconds);
    return wall ? `${wall.toLocaleString()} · ${run}` : run;
  };

  class PresentationArchive {
    constructor(scopes) {
      this.entries = [];
      this.indices = new Map();
      this.version = 0;
      this.markers = STUDY_HISTORY.stages.filter((s) => s.label);
      this.frames = STUDY_HISTORY.stages.map((stage) => {
        const frame = structuredClone(scopes);
        for (const g of Object.values(frame)) {
          g.nodes = g.nodes.filter((n) => !stage.omit?.includes(n.id));
          g.edges = g.edges.filter(
            (e) =>
              g.nodes.some((n) => n.id === e.from) &&
              g.nodes.some((n) => n.id === e.to),
          );
          for (const n of g.nodes) {
            const patch = stage.nodes[n.id];
            if (patch)
              Object.assign(n, patch, {
                preview: { ...n.preview, ...patch.preview },
              });
          }
        }
        return { at: stage.at, scopes: frame };
      });
      this.frames.push({
        at: STUDY_HISTORY.duration,
        scopes: structuredClone(scopes),
      });
    }
    frame(t) {
      let lo = 0,
        hi = this.frames.length;
      while (lo + 1 < hi) {
        const mid = (lo + hi) >> 1;
        if (this.frames[mid].at <= t) lo = mid;
        else hi = mid;
      }
      return this.frames[lo];
    }
    append(record) {
      const index = (this.indices.get(record.actor) || 0) + (record.count || 1);
      this.indices.set(record.actor, index);
      const r = { ...record, index, time: format(record.at, true) };
      this.entries.push(r);
      this.version++;
      return r;
    }
    rows(t, actors) {
      return this.entries
        .filter((r) => r.at <= t && (!actors || actors.has(r.actor)))
        .sort((a, b) => b.at - a.at);
    }
    density(start, end, seconds, actors) {
      const first = Math.floor(start / seconds),
        last = Math.floor(end / seconds);
      const values = new Float32Array(last - first + 1);
      for (const r of this.arrivals ?? this.entries) {
        if (r.event !== "actor_arrival" || r.at > end || !actors.has(r.actor))
          continue;
        const bucket = Math.floor((r.at - 0.000001) / seconds);
        if (bucket >= first && bucket <= last)
          values[bucket - first] += (r.count || 1) / seconds;
      }
      for (let i = 0; i < values.length; i++)
        if ((first + i) * seconds < this.observedFrom) values[i] = NaN;
      return { first, seconds, values };
    }
  }

  class TimeMachine {
    constructor(archive, options) {
      this.archive = archive;
      this.options = options;
      this.mode = "live";
      this.at = STUDY_HISTORY.duration;
      this.speed = 1;
      const chosen = +document.getElementById("time-range")?.value;
      this.window = chosen > 0 ? chosen : 120;
      this.range = [0, 120];
      this.lastChart = "";
      this.barHeights = new Map();
      this.markerElements = new Map();
      this.tickElements = new Map();
      this.reduced = matchMedia("(prefers-reduced-motion: reduce)");
      this.animation = 0;
      this.hover = null;
      this.dragging = false;
      this.frame = 0;
      this.panel = document.getElementById("time-machine");
      this.track = document.getElementById("time-track");
      this.canvas = document.getElementById("time-density");
      this.bind();
      document.addEventListener("visibilitychange", () => this.requestTick());
      this.reduced.addEventListener("change", () => this.requestTick());
      this.resize = new ResizeObserver(() => {
        this.measure();
        this.lastChart = "";
        this.draw();
        this.requestTick();
      });
      this.resize.observe(this.track);
      this.refresh();
      this.options.transport.attach?.(this);
    }
    get head() {
      return this.options.head();
    }
    get position() {
      return this.options.transport.position(this);
    }
    seek(t) {
      return this.options.transport.seek(this, t);
    }
    resume() {
      return this.options.transport.resume(this);
    }
    live() {
      return this.options.transport.live(this);
    }
    setSpeed(value) {
      this.at = this.position;
      this.speed = value;
      return this.options.transport.speedChanged(this);
    }
    show({ mode, at = this.at, horizon = this.horizon, ended = false, seek = true }) {
      Object.assign(this, { mode, at, horizon, ended });
      if (mode === "live") this.lastChart = "";
      this.options.onView(this.position, mode, seek);
      this.refresh();
    }
    endDrag() {
      this.dragging = false;
      if (this.frame) cancelAnimationFrame(this.frame);
      this.frame = 0;
    }
    requestTick() {
      const active =
        !document.hidden &&
        this.options.visible() &&
        this.mode !== "history" &&
        !this.reduced.matches &&
        (this.mode === "replay" || this.settling);
      if (!active) {
        if (this.animation) cancelAnimationFrame(this.animation);
        this.animation = 0;
        return;
      }
      if (!this.animation)
        this.animation = requestAnimationFrame(() => {
          this.animation = 0;
          this.tick();
          this.requestTick();
        });
    }
    tick() {
      if (this.options.visible && !this.options.visible()) return;
      if (this.mode === "replay" && this.position >= this.horizon) {
        this.show({ mode: "history", at: this.horizon, ended: true });
        document.getElementById("time-context").textContent = "End of recording";
      }
      if (this.mode === "live" && !this.dragging)
        this.range = [Math.max(0, this.head - this.window), this.head];
      this.moveCursor();
      this.draw();
    }
    pointerTime(e) {
      const box = this.track.getBoundingClientRect();
      return (
        this.range[0] +
        clamp((e.clientX - box.left) / box.width, 0, 1) *
          (this.range[1] - this.range[0])
      );
    }
    queueSeek(t) {
      this.pending = t;
      if (!this.frame)
        this.frame = requestAnimationFrame(() => {
          this.frame = 0;
          this.seek(this.pending);
        });
    }
    bind() {
      this.track.addEventListener("pointerdown", (e) => {
        if (e.button !== 0) return;
        this.track.setPointerCapture(e.pointerId);
        this.track.focus({ preventScroll: true });
        const judged = this.seek(this.pointerTime(e));
        this.dragging = this.mode !== "live" || judged !== undefined;
        e.preventDefault();
      });
      this.track.addEventListener("pointermove", (e) => {
        const t = this.pointerTime(e);
        if (this.dragging) this.queueSeek(t);
        else {
          this.hover = t;
          this.drawHover();
        }
      });
      const release = (e) => {
        if (!this.dragging) return;
        if (this.frame) cancelAnimationFrame(this.frame);
        this.frame = 0;
        if (e.type !== "pointercancel") this.seek(this.pointerTime(e));
        this.dragging = false;
        this.hover = null;
        this.drawHover();
      };
      this.track.addEventListener("pointerup", release);
      this.track.addEventListener("pointercancel", release);
      this.track.addEventListener("lostpointercapture", () => {
        this.dragging = false;
      });
      this.track.addEventListener("pointerleave", () => {
        if (!this.dragging) {
          this.hover = null;
          this.drawHover();
        }
      });
      this.track.addEventListener("keydown", (e) => {
        const step = e.shiftKey ? 10 : 1;
        if (
          ["ArrowLeft", "ArrowRight", "Home", "End", " ", "Escape"].includes(
            e.key,
          )
        ) {
          e.preventDefault();
          e.stopPropagation();
          if (e.key === "Escape") this.live();
          else if (e.key === " ") this.mode !== "live" && this.resume();
          else
            this.seek(
              e.key === "Home"
                ? this.range[0]
                : e.key === "End"
                  ? this.range[1]
                  : (this.options.transport.asked?.(this) ?? this.position) +
                    (e.key === "ArrowLeft" ? -step : step),
            );
        }
      });
      document
        .getElementById("time-resume")
        .addEventListener("click", () => this.resume());
      document
        .getElementById("time-live")
        .addEventListener("click", () => this.live());
      document
        .getElementById("time-speed")
        .addEventListener("change", (e) => this.setSpeed(+e.target.value));
      document.getElementById("time-range").addEventListener("change", (e) => {
        this.window = +e.target.value;
        const center =
          this.mode === "live" ? this.head - this.window / 2 : this.position;
        this.range = [
          Math.max(
            0,
            Math.min(this.head - this.window, center - this.window / 2),
          ),
          0,
        ];
        this.range[1] = Math.min(this.head, this.range[0] + this.window);
        this.lastChart = "";
        this.refresh();
      });
      this.panel
        .querySelector(".time-markers")
        .addEventListener("click", (e) => {
          const marker = e.target.closest("[data-time]");
          if (marker) this.seek(+marker.dataset.time);
        });
    }
    refresh() {
      this.options.bar?.prepare?.(this);
      this.redraw();
      this.options.bar?.refresh?.(this);
    }
    redraw() {
      if (this.options.visible && !this.options.visible()) return;
      if (this.mode === "live" && !this.dragging)
        this.range = [Math.max(0, this.head - this.window), this.head];
      const history = this.mode !== "live";
      this.panel.dataset.mode = this.mode;
      document.getElementById("time-mode").textContent =
        this.mode === "live"
          ? "LIVE"
          : this.mode === "replay"
            ? "REPLAYING"
            : "HISTORY";
      document.getElementById("time-context").textContent = history
        ? this.ended ? "End of recording" : ""
        : this.options.bar?.phrase ?? "Arrivals in this scope";
      if (this.options.observing?.() === false && !history) {
        document.getElementById("time-mode").textContent = "";
        const ended = this.options.ended?.();
        if (ended) document.getElementById("time-context").textContent = ended;
      }
      document.getElementById("time-resume").disabled = !history;
      document.getElementById("time-resume-label").textContent =
        this.mode === "replay" ? "Pause replay" : "Resume";
      document
        .getElementById("time-resume")
        .classList.toggle("replaying", this.mode === "replay");
      document.getElementById("time-live").classList.toggle("active", !history);
      document
        .getElementById("time-live")
        .setAttribute("aria-pressed", String(!history));
      document.getElementById("time-speed").disabled = !history;
      this.track.setAttribute("aria-valuemin", this.range[0].toFixed(2));
      this.track.setAttribute("aria-valuemax", this.range[1].toFixed(2));
      this.tick();
      this.requestTick();
    }
    moveCursor() {
      const t = this.position,
        fraction = (t - this.range[0]) / (this.range[1] - this.range[0]);
      const cursor = document.getElementById("time-cursor");
      cursor.style.left = clamp(fraction * 100, 0, 100) + "%";
      const reading = format(t, true, this.range[0]);
      const current = document.getElementById("time-current");
      current.textContent = reading;
      current.title = describe(t);
      this.options.reading?.(reading);
      document.getElementById("time-offset").textContent =
        this.mode === "live"
          ? "now"
          : "−" + Math.max(0, Math.round(this.head - t)) + "s";
      this.track.setAttribute("aria-valuenow", t.toFixed(2));
      this.track.setAttribute(
        "aria-valuetext",
        `${reading}, ${this.mode}`,
      );
      this.options.bar?.cursor?.(this);
    }
    measure() {
      this.trackWidth = this.track.getBoundingClientRect().width;
      this.trackClientWidth = this.track.clientWidth;
    }
    draw() {
      this.settling = false;
      if (this.trackWidth === undefined) this.measure();
      if (!this.trackWidth) return;
      const w = this.trackWidth,
        h = 61,
        dpr = Math.min(devicePixelRatio || 1, 2);
      if (
        this.canvas.width !== Math.round(w * dpr) ||
        this.canvas.height !== h * dpr
      ) {
        this.canvas.width = Math.round(w * dpr);
        this.canvas.height = h * dpr;
      }
      const now = performance.now(),
        dt = this.drawnAt ? Math.min(0.1, (now - this.drawnAt) / 1000) : 1;
      this.drawnAt = now;
      const actors = this.options.actors(),
        scopeKey = [...actors].join();
      const span = this.range[1] - this.range[0];
      const answered = this.options.bar?.buckets?.(this, w);
      const seconds = answered
        ? answered.seconds
        : 0.25 *
          2 **
            Math.max(
              0,
              Math.ceil(Math.log2(span / Math.max(1, Math.floor(w / 5)) / 0.25)),
            );
      const context = `${scopeKey}:${seconds}:${this.window}`;
      const key = answered
        ? `${context}:${answered.version}`
        : `${context}:${Math.floor(this.range[0] / seconds)}:${Math.floor(this.range[1] / seconds)}:${this.archive.version}`;
      if (key !== this.lastChart) {
        this.lastChart = key;
        this.buckets =
          answered ??
          this.archive.density(this.range[0], this.range[1], seconds, actors);
        const peak = Math.max(5, ...this.buckets.values.filter(Number.isFinite));
        const unit = 10 ** Math.floor(Math.log10(peak));
        const max = Math.ceil(peak / (unit / 2)) * (unit / 2);
        if (context !== this.chartContext) {
          this.chartContext = context;
          this.scale = this.scaleTarget = max;
          this.barHeights.clear();
        } else this.scaleTarget = Math.max(this.scaleTarget, max);
      }
      const animated = this.mode === "live" && !this.reduced.matches;
      const blend = animated ? 1 - Math.exp(-dt * 14) : 1;
      this.scale +=
        (this.scaleTarget - this.scale) *
        (animated ? 1 - Math.exp(-dt * 5) : 1);
      const ctx = this.canvas.getContext("2d");
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      ctx.strokeStyle = window.StudyPaint?.grid;
      ctx.lineWidth = 1;
      for (const y of [20.5, 40.5, 60.5]) {
        ctx.beginPath();
        ctx.moveTo(0, y);
        ctx.lineTo(w, y);
        ctx.stroke();
      }
      const pixels = w / span,
        position = this.position;
      this.buckets.values.forEach((value, i) => {
        if (Number.isNaN(value)) return;
        const bucket = this.buckets.first + i,
          at = bucket * seconds;
        const before =
          this.barHeights.get(bucket) ?? (at + seconds > this.head ? 0 : value);
        const shown = before + (value - before) * blend;
        this.barHeights.set(bucket, shown);
        const height = Math.max(2, (shown / this.scale) * 44),
          x = (at - this.range[0]) * pixels;
        ctx.fillStyle =
          this.mode === "live"
            ? window.StudyPaint?.data
            : at <= position
              ? window.StudyPaint?.history
              : window.StudyPaint?.rest;
        ctx.fillRect(
          x,
          h - height,
          Math.max(1, seconds * pixels - 1.5),
          height,
        );
        if (this.buckets.incidents?.[i] > 0) {
          ctx.fillStyle = window.StudyPaint?.incident;
          ctx.fillRect(x, 0, Math.max(2, seconds * pixels - 1.5), 4);
        }
      });
      for (const bucket of this.barHeights.keys()) {
        if (
          bucket < this.buckets.first ||
          bucket >= this.buckets.first + this.buckets.values.length
        )
          this.barHeights.delete(bucket);
      }
      this.settling =
        animated &&
        this.buckets.values.some(
          (value, i) =>
            Number.isFinite(value) &&
            Math.abs(
              this.barHeights.get(this.buckets.first + i) / this.scale -
                value / this.scaleTarget,
            ) *
              44 *
              dpr >
              window.WireField.still,
        );
      const known = this.buckets.values.filter(Number.isFinite);
      const scaleSlot = document.getElementById("time-scale");
      scaleSlot.textContent = known.length ? "axis " + Math.ceil(this.scale) + " /s" : "";
      scaleSlot.title = known.length
        ? `Chart axis top, not a rate · tallest bar in view ${Math.max(...known).toFixed(1)} events/s`
        : "";
      this.positionAnnotations(actors, pixels, span);
    }
    positionAnnotations(actors, pixels, span) {
      const markerBox = this.panel.querySelector(".time-markers");
      const moments = new Map();
      for (const m of this.archive.markers) {
        if (!moments.has(m.at)) moments.set(m.at, []);
        moments.get(m.at).push(m);
      }
      for (const [at, recorded] of moments) {
        const marks = recorded.filter(m => m.actor === undefined || actors.has(m.actor));
        let button = this.markerElements.get(at);
        if (!button) {
          button = document.createElement("button");
          button.dataset.time = at;
          button.innerHTML = "<i></i><span></span>";
          markerBox.append(button);
          this.markerElements.set(at, button);
        }
        const label = marks.map(m => m.label).join(" · ");
        button.querySelector("span").textContent = label;
        if (marks[0]?.kind) button.dataset.kind = marks[0].kind;
        else delete button.dataset.kind;
        button.dataset.kinds = marks.map(m => m.kind).filter(Boolean).join(" ");
        const when = format(at, false, this.range[0]);
        button.title = [when, ...marks.flatMap(m => [m.label, m.detail])].filter(Boolean).join(" · ");
        button.setAttribute("aria-label", `Jump to ${label}, ${when}`);
        button.hidden = marks.length === 0 || at < this.range[0] || at > this.range[1];
        const x = (at - this.range[0]) * pixels;
        button.style.left = x + "px";
        const right = x > span * pixels / 2;
        button.style.transform = right ? "translateX(calc(-100% + 7px))" : "translateX(-7px)";
        button.style.flexDirection = right ? "row-reverse" : "row";
      }
      for (const [key, button] of this.markerElements)
        if (!moments.has(key)) { button.remove(); this.markerElements.delete(key); }
      const unit =
        span <= 40
          ? 10
          : span <= 150
            ? 30
            : [60, 120, 300, 600, 900, 1800, 3600, 7200, 10800, 21600, 43200].find(
                (step) => step >= span / 6,
              ) ?? 86400 * Math.ceil(span / 6 / 86400);
      const start = Math.ceil(this.range[0] / unit) * unit;
      const tickBox = document.getElementById("time-ticks"),
        active = new Set();
      for (let t = start; t <= this.range[1]; t += unit) {
        active.add(t);
        let tick = this.tickElements.get(t);
        if (!tick) {
          tick = document.createElement("span");
          tickBox.append(tick);
          this.tickElements.set(t, tick);
        }
        const text = format(t, false, this.range[0]);
        if (tick.textContent !== text) tick.textContent = text;
        const x = (t - this.range[0]) * pixels;
        tick.style.left = x + "px";
        tick.style.transform =
          x < 30
            ? "none"
            : x > this.trackClientWidth - 30
              ? "translateX(-100%)"
              : "translateX(-50%)";
      }
      for (const [t, tick] of this.tickElements)
        if (!active.has(t)) {
          tick.remove();
          this.tickElements.delete(t);
        }
    }
    drawHover() {
      const el = document.getElementById("time-hover");
      el.hidden = this.hover === null || this.dragging || !this.buckets;
      if (el.hidden) return;
      const fraction = clamp(
        (this.hover - this.range[0]) / (this.range[1] - this.range[0]),
        0,
        1,
      );
      const i =
        Math.floor(this.hover / this.buckets.seconds) - this.buckets.first;
      el.style.left = fraction * 100 + "%";
      el.querySelector("strong").textContent = format(this.hover, true, this.range[0]);
      el.querySelector("small").textContent =
        Math.round(this.buckets.values[i] || 0) + " events/s · click to seek";
      el.classList.toggle("near-right", fraction > 0.8);
      el.classList.toggle("near-left", fraction < 0.2);
      this.options.bar?.hover?.(this);
    }
  }
  window.PresentationArchive = PresentationArchive;
  window.TimeMachine = TimeMachine;
  window.studyTimeFormat = format;
  window.studyTimeTitle = describe;
})();
