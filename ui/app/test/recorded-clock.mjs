import fs from 'node:fs';
import vm from 'node:vm';

const window = {STUDY_HISTORY:{start:0, duration:0, stages:[]}};
vm.runInContext(fs.readFileSync(new URL('../time-machine.js', import.meta.url), 'utf8'),
  vm.createContext({window, STUDY_HISTORY:window.STUDY_HISTORY}));
export const clock = window.studyTimeFormat;
export const clockTitle = window.studyTimeTitle;
