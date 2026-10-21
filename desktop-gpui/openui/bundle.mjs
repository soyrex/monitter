// Called by build.rs after NSH's own JSX compiler. Never installs dependencies.
import { build } from 'esbuild';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const here=dirname(fileURLToPath(import.meta.url));
const [entry,outfile]=process.argv.slice(2);
if(!entry||!outfile)throw Error('Usage: node bundle.mjs <NSH-compiled-module.js> <output.js>');
await build({tsconfigRaw:{},entryPoints:[resolve(entry)],outfile:resolve(outfile),bundle:true,format:'esm',platform:'browser',target:'es2022',minify:true,external:['nsh','nsh/*','nsh:rt'],plugins:[{name:'trusted-adapter-imports',setup(build){build.onResolve({filter:/^\.\/(controller\.mjs|calendar\.mjs|charts\.mjs|media\.mjs|vendor\/core\.js)$/},args=>({path:resolve(here,args.path)}));}}]});
