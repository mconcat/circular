import path from 'node:path';
import { appRoot, appPackages, verifyPackages, verifyApp } from './sdk-packages.mjs';
import { verifyFontBundle } from './font-bundle.mjs';

const args = process.argv.slice(2);
const usage = 'Usage: npm run dist -- [--dir] [--out <directory>] | --check [<Circular.app>]';
const option = flag => args.includes(flag) ? args[args.indexOf(flag) + 1] : undefined;
const check = args[0] === '--check';
if (check ? args.length > 2 || (args[1] ?? '').startsWith('--')
  : args.some((arg, i) => !['--dir', '--out'].includes(arg) && args[i - 1] !== '--out') ||
    (args.includes('--out') && !option('--out'))) throw new Error(usage);
console.log(`SDK package copy check: ${await verifyPackages()} files`);
console.log('Local font bundle check:', await verifyFontBundle());
if (check && args[1]) console.log(`Packaged SDK check: ${await verifyApp(path.resolve(args[1]))} files`);
if (!check) {
  const { build, Platform } = await import('electron-builder');
  await build({ projectDir: appRoot, targets: Platform.MAC.createTarget(args.includes('--dir') ? ['dir'] : ['dir', 'dmg']),
    config: {
      ...(option('--out') ? { directories: { output: path.resolve(option('--out')) } } : {}),
      electronDownload: { cache: path.join(appRoot, '.electron-cache') },
      afterPack: async context => {
        const app = path.join(context.appOutDir, 'Circular.app');
        console.log(`Packaged SDK check: ${await verifyApp(app)} files`);
        console.log('Packaged font check:', await verifyFontBundle(path.join(appPackages(app), '../../fonts')));
      },
    },
  });
}
