(async () => {
  const scene = new URLSearchParams(location.search).get('scene') ?? 'canvas';
  const a = StudyApp;
  if (scene === 'dense') await Product.scenario('dense');
  if (scene === 'approvals') await Product.scenario('approvals');
  if (scene === 'error') await Product.scenario('idle_failed');
  const n = a.allNode('research');
  a.state.selected = 'research';
  a.state.selectedSet = new Set(['research']);
  n.title = 'Research agent';
  n.preview.task = 'Understanding the next question · Working context';
  n.preview.text = 'Reading local sources and preparing a reply. The program stays editable while actors work independently.';
  StudyViewer.message('prompt', 'Please read the local sources.');
  a.timeMachine.started = -980;
  a.archive.advance(a.timeMachine.head, a.state.paused);
  a.renderGraph();
  if (scene !== 'dense') Object.assign(a.state, {zoom:.85, x:100, y:74});
  a.transformWorld();
  a.timeMachine.refresh();
  if (scene === 'configure') { a.state.tab = 'configure'; a.renderInspector(); }
  if (scene === 'approvals') Product.actions.approvals();
  if (scene === 'error') { a.state.selected = 'tools'; a.renderGraph(); }
  document.querySelector('#toast').classList.remove('visible');
  await Appearance.apply(document.documentElement.dataset.theme);
})();
