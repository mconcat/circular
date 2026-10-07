import { runShell } from '../shell.mjs';
import { editFixtureTransport } from './edit-fixture.mjs';

await runShell({ openTransport: editFixtureTransport, initialState: 'SDK fixture' });
