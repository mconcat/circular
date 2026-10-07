export const recordedStudyEntry = '/recorded-study/index.html';

export function recordedStudyFiles(data) {
  if (!data?.study) throw new Error('Static study input is {study, catalog, history}');
  const scripts = new Map([
    ['fixture.js', `window.STUDY = ${JSON.stringify(data.study)};`],
    ['catalog-data.js', `window.PUBLISHED_ACTORS = ${JSON.stringify(data.catalog)};`],
    ['time-fixture.js', `window.STUDY_HISTORY = ${JSON.stringify(data.history)};`],
  ]);
  return pathname => {
    const prefix = '/recorded-study/';
    if (!pathname.startsWith(prefix)) return undefined;
    const name = pathname.slice(prefix.length);
    return { pathname: '/' + name, source: scripts.get(name) };
  };
}
