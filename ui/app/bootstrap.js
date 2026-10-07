window.circularReady = (async () => {
  const fixture = new URLSearchParams(location.search).get('fixture') === '1';
  const canvasScripts = ['icons.js', 'product-catalog.js', 'wire-field.js', 'time-machine.js', 'app.js', 'product.js'];
  const studyScripts = ['fixture.js', 'catalog-data.js', 'time-fixture.js'];
  const load = async (list) => {
    for (const src of list) await new Promise((resolve, reject) => {
      const script = document.createElement('script');
      script.src = src; script.onload = resolve; script.onerror = () => reject(new Error(src));
      document.head.append(script);
    });
  };
  const adapter = fixture ? null : await import('./renderer/adapter.mjs');
  const viewKinds = await import('./renderer/views.mjs');
  viewKinds.installViewers(window);
  window.CanvasLayout = await import('./renderer/layout.mjs');
  window.CanvasTier = await import('./renderer/tier.mjs');
  window.CardSize = await import('./renderer/card-size.mjs');
  const frames = window.circularFrames;
  let metrics;
  if (adapter) {
    const { openSession } = await import('./renderer/session.mjs');
    await adapter.initialize({ connect: frames && (attachment => openSession(frames, attachment)) });
  } else {
    await load(studyScripts);
    window.StudySource = (await import('./fixture-source.mjs')).fixtureSource();
  }
  if (!window.StudySource.noProject) {
    for (const template of document.querySelectorAll('template[data-project-shell]'))
      template.replaceWith(template.content);
    if (adapter) for (const element of document.querySelectorAll('[data-fixture-only]')) element.remove();
    await load(canvasScripts);
    metrics = await window.CardSize.documentMetrics(document, viewKinds.views);
  } else {
    await load(['icons.js', 'product-catalog.js', 'product.js']);
  }
  const ready = await (adapter ?? window.StudySource).mount({ metrics });
  await document.fonts.ready;
  return ready ?? true;
})();
