import { viewRegistry, liveViewers } from './view-registry.mjs';
import { viewer } from './viewer.mjs';
import prompt from './view-prompt.mjs';
import agent from './view-agent.mjs';
import tools from './view-tools.mjs';
import transcript from './view-transcript.mjs';
import notebook from './view-notebook.mjs';
import boundary from './view-boundary.mjs';
import basic from './view-basic.mjs';
import task from './view-task.mjs';
import number from './view-number.mjs';
import trend from './view-trend.mjs';
import timing from './view-timing.mjs';
import routing from './view-routing.mjs';
import alerting from './view-alerting.mjs';
import table from './view-table.mjs';
import response from './view-response.mjs';
import output from './view-output.mjs';
import value from './view-value.mjs';
import feed from './view-feed.mjs';
import notification from './view-notification.mjs';
import peer from './view-peer.mjs';
import assembly from './view-assembly.mjs';
import instances from './view-instances.mjs';
import scope from './view-scope.mjs';
import form from './view-form.mjs';

export const views = viewRegistry([prompt, agent, tools, transcript, notebook, boundary, basic, task, number,
  trend, timing, routing, alerting, table, response, output, value, feed, notification, peer, assembly,
  instances, scope, form]);

export function installViewers(win = globalThis.window) {
  win.LiveViewers = liveViewers(views, win);
  win.StudyViewer = viewer;
  return win.LiveViewers;
}
