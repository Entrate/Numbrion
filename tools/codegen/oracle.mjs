// Shared read-only pin check. No child processes and no checkout writes.
import fs from 'node:fs';
import path from 'node:path';
export const COMMIT = '7332b60e22b9e8194bb53549549eba241d73cc9a';
export function assertOraclePin(checkout) {
 let gitDir = path.join(checkout, '.git');
 if (!fs.statSync(gitDir).isDirectory()) gitDir = path.resolve(checkout, fs.readFileSync(gitDir,'utf8').trim().slice(8));
 let head = fs.readFileSync(path.join(gitDir,'HEAD'),'utf8').trim();
 const commonFile=path.join(gitDir,'commondir');
 const common=fs.existsSync(commonFile)?path.resolve(gitDir,fs.readFileSync(commonFile,'utf8').trim()):gitDir;
 if (head.startsWith('ref: ')) {
  const name=head.slice(5), loose=path.join(common,name);
  head=fs.existsSync(loose)?fs.readFileSync(loose,'utf8').trim():fs.readFileSync(path.join(common,'packed-refs'),'utf8').split('\n').find(l=>l.endsWith(' '+name))?.split(' ')[0];
 }
 if(head!==COMMIT)throw Error(`Wrong oracle commit: ${head}; expected ${COMMIT}`);
}
