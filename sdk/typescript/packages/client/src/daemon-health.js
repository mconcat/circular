import { LifecycleWord } from '@circular/protocol/tables';
import { daemonHealthPageFromValue as recordedPage } from '../../protocol/src/internal/observation-values.js';

const NOT_STANDING = 'not_standing';
const pipelineStands = word => word !== null && LifecycleWord.find(entry => entry.as_str === word).pipeline_stands;

export function daemonHealthPageFromValue(value) {
  const page = recordedPage(value);
  if (pipelineStands(page.anchor.lifecycle)) return page;
  return { ...page, items: page.items.map(row => ({ ...row, state: NOT_STANDING, recorded: row.state })) };
}
